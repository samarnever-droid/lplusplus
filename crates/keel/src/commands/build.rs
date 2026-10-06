//! `keel build` / `keel check` / `keel run` — drive the `lpp` compiler.
//!
//! `keel build` is workspace-aware: it discovers the workspace containing
//! the current directory, topologically orders its members over the
//! path-dep DAG, and builds each layer's members CONCURRENTLY (std threads,
//! zero new deps). A single package is a 1-member workspace — same code
//! path (docs/rewrite/WORKSPACE.md).
//!
//! Path deps are made importable by staging them into
//! `<root>/.lpp_packages/` (docs/rewrite/DEP_LINKING.md); every `lpp`
//! invocation runs with cwd = the workspace root.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::mpsc;

use fs2::FileExt;
use tabled::builder::Builder;

struct WorkspaceGuard(std::fs::File);

impl Drop for WorkspaceGuard {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

fn lock_workspace(root: &Path) -> Result<WorkspaceGuard, String> {
    let state = root.join(".keel");
    std::fs::create_dir_all(&state).map_err(|error| error.to_string())?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(state.join("workspace.lock"))
        .map_err(|error| error.to_string())?;
    file.lock_exclusive().map_err(|error| error.to_string())?;
    Ok(WorkspaceGuard(file))
}

/// The `lpp` compiler binary (from `KEEL_LPP`, else `lpp` on PATH).
pub fn lpp_bin() -> String {
    if let Some(configured) = std::env::var("KEEL_LPP")
        .ok()
        .filter(|value| !value.trim().is_empty())
    {
        return configured;
    }
    if let Ok(current) = std::env::current_exe()
        && let Some(directory) = current.parent()
    {
        if current
            .file_stem()
            .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("lpp"))
        {
            return current.display().to_string();
        }
        let sibling = directory.join(format!("lpp{}", std::env::consts::EXE_SUFFIX));
        if sibling.is_file() {
            return sibling.display().to_string();
        }
        if directory.file_name().is_some_and(|name| name == "deps")
            && let Some(profile) = directory.parent()
        {
            let sibling = profile.join(format!("lpp{}", std::env::consts::EXE_SUFFIX));
            if sibling.is_file() {
                return sibling.display().to_string();
            }
        }
    }
    "lpp".to_string()
}

/// The project's entry point: `src/main.lpp` (binary) or `src/lib.lpp` (lib).
fn entry_point(dir: &Path) -> Result<String, String> {
    for name in ["src/main.lpp", "src/lib.lpp"] {
        if dir.join(name).exists() {
            return Ok(name.to_string());
        }
    }
    Err("no entry point found (expected src/main.lpp or src/lib.lpp)".to_string())
}

// ---------------------------------------------------------------------------
// Path-dep staging (.lpp_packages) — docs/rewrite/DEP_LINKING.md
// ---------------------------------------------------------------------------

/// Replace (or create) the symlink `link` → `target`. Refuses to clobber a
/// real directory (user data); a stale symlink/file is replaced.
fn set_symlink(link: &Path, target: &Path) -> Result<(), String> {
    if let Some(md) = std::fs::symlink_metadata(link).ok() {
        if md.file_type().is_symlink() || md.is_file() {
            std::fs::remove_file(link).map_err(|e| e.to_string())?;
        } else {
            #[cfg(unix)]
            return Err(format!(
                "refusing to replace real directory {} (remove it first)",
                link.display()
            ));
            #[cfg(not(unix))]
            std::fs::remove_dir_all(link).map_err(|e| e.to_string())?;
        }
    }
    if let Some(parent) = link.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(target, link).map_err(|e| format!("symlink: {e}"))
    }
    #[cfg(not(unix))]
    {
        if link.is_dir() {
            std::fs::remove_dir_all(link).map_err(|e| format!("remove old copy: {e}"))?;
        }
        copy_tree(target, link).map_err(|e| format!("copy: {e}"))
    }
}

#[cfg(not(unix))]
fn copy_tree(src: &Path, dst: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dst)?;
    for e in std::fs::read_dir(src)? {
        let e = e?;
        let t = dst.join(e.file_name());
        if e.path().is_dir() {
            copy_tree(&e.path(), &t)?;
        } else {
            std::fs::copy(e.path(), &t)?;
        }
    }
    Ok(())
}

/// Stage every path dep of every member into `<root>/.lpp_packages/<dep>/`
/// (`lpp.toml` with the member's entry + a `src` symlink), so lpp's module
/// resolver can find `import <dep>`. Idempotent; prunes only entries Keel
/// itself managed (`managed = "keel"`) that are neither path deps nor in
/// `extra_needed` (the registry-staged names). Returns the staged dep names.
pub fn stage_path_deps(
    ws: &lpp_pm::Workspace,
    extra_needed: &BTreeSet<String>,
) -> Result<Vec<String>, String> {
    let pkgs_dir = ws.root.join(".lpp_packages");
    let index = ws.index();

    // needed: dep name → (member dir, entry, version)
    let mut needed: BTreeMap<String, (PathBuf, String, String)> = BTreeMap::new();
    for m in &ws.members {
        for (dep_name, dep) in &m.manifest.dependencies {
            if dep.path().is_none() {
                continue;
            }
            match index.get(dep_name.as_str()) {
                Some(&mi) => {
                    let dm = &ws.members[mi];
                    let entry = entry_point(&dm.dir)?;
                    needed.insert(
                        dep_name.clone(),
                        (dm.dir.clone(), entry, dm.manifest.version().to_string()),
                    );
                }
                // Not a member: `build_plan` rejects it (E6019) before jobs run.
                None => continue,
            }
        }
    }

    std::fs::create_dir_all(&pkgs_dir).map_err(|e| e.to_string())?;

    let mut staged = Vec::new();
    for (name, (dep_dir, entry, version)) in &needed {
        let pdir = pkgs_dir.join(name);
        if pdir.exists() && !pdir.join(".keel-path-stage").is_file() {
            return Err(format!(
                "refusing to overwrite non-Keel .lpp_packages/{name}"
            ));
        }
        std::fs::create_dir_all(&pdir).map_err(|e| e.to_string())?;
        std::fs::write(pdir.join(".keel-path-stage"), "managed by keel\n")
            .map_err(|e| e.to_string())?;

        let doc = format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\nentry = \"{entry}\"\nmanaged = \"keel\"\n"
        );
        let manifest_path = pdir.join("lpp.toml");
        if std::fs::read_to_string(&manifest_path).ok().as_deref() != Some(doc.as_str()) {
            std::fs::write(&manifest_path, &doc).map_err(|e| e.to_string())?;
        }

        // Absolute target: symlink resolution is relative to the LINK's
        // directory, not to the workspace root.
        let dep_dir = dep_dir
            .canonicalize()
            .map_err(|e| format!("path dep '{name}': {e}"))?;
        let dep_src = dep_dir.join("src");
        if !dep_src.is_dir() {
            return Err(format!("path dep '{name}' has no src/ directory"));
        }
        let link = pdir.join("src");
        let current_target = std::fs::read_link(&link).ok();
        if current_target.as_deref() != Some(dep_src.as_path()) {
            set_symlink(&link, &dep_src)?;
        }
        std::fs::write(
            pdir.join(".keel-path-stage"),
            format!("{}\n", dep_dir.display()),
        )
        .map_err(|e| e.to_string())?;
        staged.push(name.clone());
    }

    // Prune stale MANAGED entries (never touch foreign .lpp_packages dirs,
    // and never touch registry-staged entries — `stage_all_deps` prunes
    // those separately).
    if let Ok(rd) = std::fs::read_dir(&pkgs_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if needed.contains_key(&name) || extra_needed.contains(&name) {
                continue;
            }
            if e.path().join(".keel-path-stage").is_file() {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    Ok(staged)
}

// Registry-dep staging (.lpp_packages from the git registry) —
// docs/rewrite/DEP_LINKING.md (slice 11).
// ---------------------------------------------------------------------------

/// The complete reachable registry dependency closure, sourced from the
/// validated lockfile.
fn registry_deps_needed(
    ws: &lpp_pm::Workspace,
) -> Result<Option<(lpp_pm::Lock, BTreeMap<String, (String, String)>)>, String> {
    let member_names: BTreeSet<String> = ws
        .members
        .iter()
        .map(|member| member.name().to_string())
        .collect();
    let mut direct = Vec::new();
    for member in &ws.members {
        let default_features = member
            .manifest
            .features
            .get("default")
            .cloned()
            .unwrap_or_default();
        for (name, dependency) in &member.manifest.dependencies {
            let active =
                !dependency.optional() || default_features.iter().any(|feature| feature == name);
            if dependency.path().is_none() && active && !member_names.contains(name) {
                direct.push((name.clone(), dependency.version().to_string()));
            }
        }
    }
    if direct.is_empty() {
        return Ok(None);
    }
    let lock_path = ws.root.join("Keel.lock");
    let document = std::fs::read_to_string(&lock_path).map_err(|_| {
        format!(
            "registry dependency '{}' needs a Keel.lock — run `keel fetch` first",
            direct[0].0
        )
    })?;
    let lock = lpp_pm::Lock::parse(&document).map_err(|error| error.to_string())?;

    let mut queue = Vec::new();
    for (name, requirement) in direct {
        let package = lock.package(&name).ok_or_else(|| {
            format!("registry dependency '{name}' is not in Keel.lock — run `keel fetch`")
        })?;
        let selected =
            lpp_pm::validation::version(&package.version).map_err(|error| error.to_string())?;
        let requirement =
            lpp_pm::validation::requirement(&requirement).map_err(|error| error.to_string())?;
        if !requirement.matches(&selected) {
            return Err(format!(
                "Keel.lock selects {name} {}, which does not satisfy {requirement}; run `keel fetch` or `keel update`",
                package.version
            ));
        }
        queue.push(name);
    }

    let mut needed = BTreeMap::new();
    let mut seen = BTreeSet::new();
    while let Some(name) = queue.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        let package = lock
            .package(&name)
            .ok_or_else(|| format!("locked dependency '{name}' is missing from Keel.lock"))?;
        if package.source != "registry" {
            continue;
        }
        let checksum = package.checksum.clone().ok_or_else(|| {
            format!("Keel.lock has no checksum for '{name}' — re-run `keel fetch`")
        })?;
        needed.insert(name, (package.version.clone(), checksum));
        queue.extend(package.deps.iter().cloned());
    }
    Ok(Some((lock, needed)))
}

/// Stage the full locked registry closure. A reusable stage is verified by a
/// deterministic tree hash, so modified extracted source is never compiled.
pub fn stage_registry_deps(
    ws: &lpp_pm::Workspace,
    reg: Option<&lpp_pm::Registry>,
) -> Result<Vec<String>, String> {
    let Some((lock, needed)) = registry_deps_needed(ws)? else {
        return Ok(Vec::new());
    };
    if let (Some(identity), Some(registry)) = (&lock.registry, reg)
        && identity != registry.remote()
    {
        return Err(format!(
            "Keel.lock belongs to registry '{identity}', but '{}' was configured",
            registry.remote()
        ));
    }

    let pkgs_dir = ws.root.join(".lpp_packages");
    std::fs::create_dir_all(&pkgs_dir).map_err(|error| error.to_string())?;
    let mut staged = Vec::new();

    for (name, (version, checksum)) in &needed {
        lpp_pm::validation::package_name(name).map_err(|error| error.to_string())?;
        lpp_pm::validation::version(version).map_err(|error| error.to_string())?;
        lpp_pm::validation::checksum(checksum).map_err(|error| error.to_string())?;
        let package_dir = pkgs_dir.join(name);
        let marker_path = package_dir.join(".keel-stage");
        if let Ok(marker) = std::fs::read_to_string(&marker_path) {
            let fields: Vec<&str> = marker.lines().collect();
            if fields.len() == 3 && fields[0] == version && fields[1] == checksum {
                let actual_tree = super::archive::tree_hash(&package_dir)?;
                if actual_tree != fields[2] {
                    return Err(format!(
                        "staged package '{name}' failed integrity verification; remove {} and rebuild",
                        package_dir.display()
                    ));
                }
                staged.push(name.clone());
                continue;
            }
        }

        // Build is deliberately offline-first: prefer the durable content
        // store by locked checksum. Fall back to the configured local registry
        // clone without performing a network sync.
        let address =
            lpp_pm::ContentAddress::try_new(checksum).map_err(|error| error.to_string())?;
        let store = lpp_pm::DiskBlobStore::open(crate::cache_dir().join("content"))
            .map_err(|error| error.to_string())?;
        let bytes = match lpp_pm::BlobStore::fetch(&store, &address) {
            Ok(bytes) => bytes,
            Err(lpp_pm::PmError::BlobNotFound(_)) => {
                let registry = reg.ok_or_else(|| {
                    format!(
                        "locked package '{name}' is not in the content cache; run `keel fetch` with its registry configured"
                    )
                })?;
                registry
                    .fetch_locked(name, version)
                    .map(|(bytes, _)| bytes)
                    .map_err(|error| format!("registry dependency '{name}': {error}"))?
            }
            Err(error) => return Err(error.to_string()),
        };
        let actual = lpp_pm::ContentAddress::of_bytes(&bytes).to_string();
        if actual != *checksum {
            return Err(lpp_pm::PmError::ChecksumMismatch {
                expected: checksum.clone(),
                actual,
            }
            .to_string());
        }

        let temporary = pkgs_dir.join(format!(
            ".tmp-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&temporary);
        std::fs::create_dir_all(&temporary).map_err(|error| error.to_string())?;
        if let Err(error) = super::archive::unpack(&bytes, &temporary) {
            let _ = std::fs::remove_dir_all(&temporary);
            return Err(format!(
                "failed to extract registry package '{name}': {error}"
            ));
        }
        let entry = if temporary.join("src/lib.lpp").is_file() {
            "src/lib.lpp"
        } else if temporary.join("src/main.lpp").is_file() {
            "src/main.lpp"
        } else {
            let _ = std::fs::remove_dir_all(&temporary);
            return Err(format!(
                "registry package '{name}' has no src/lib.lpp or src/main.lpp entry"
            ));
        };
        let published = lpp_pm::manifest::Manifest::parse(
            &std::fs::read_to_string(temporary.join("Keel.toml"))
                .map_err(|_| format!("registry package '{name}' has no valid Keel.toml"))?,
        )
        .map_err(|error| error.to_string())?;
        if published.name() != name || published.version() != version {
            let _ = std::fs::remove_dir_all(&temporary);
            return Err(format!(
                "registry artifact identity mismatch: expected {name} {version}, contains {} {}",
                published.name(),
                published.version()
            ));
        }

        if package_dir.exists() {
            let managed = package_dir.join(".keel-stage").is_file();
            if !managed {
                let _ = std::fs::remove_dir_all(&temporary);
                return Err(format!(
                    "refusing to overwrite non-Keel .lpp_packages/{name}"
                ));
            }
            std::fs::remove_dir_all(&package_dir).map_err(|error| error.to_string())?;
        }
        std::fs::rename(&temporary, &package_dir).map_err(|error| error.to_string())?;
        let document = format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\nentry = \"{entry}\"\nchecksum = \"{checksum}\"\nsource = \"registry\"\nmanaged = \"keel\"\n"
        );
        std::fs::write(package_dir.join("lpp.toml"), document)
            .map_err(|error| error.to_string())?;
        let tree_hash = super::archive::tree_hash(&package_dir)?;
        std::fs::write(
            package_dir.join(".keel-stage"),
            format!("{version}\n{checksum}\n{tree_hash}\n"),
        )
        .map_err(|error| error.to_string())?;
        staged.push(name.clone());
    }
    Ok(staged)
}

/// Prune stale registry-managed entries (not in `keep`).
fn prune_registry_managed(ws: &lpp_pm::Workspace, keep: &BTreeSet<String>) -> Result<(), String> {
    let pkgs_dir = ws.root.join(".lpp_packages");
    if let Ok(rd) = std::fs::read_dir(&pkgs_dir) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if keep.contains(&name) || name.starts_with('.') {
                continue;
            }
            if e.path().join(".keel-stage").is_file() {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    Ok(())
}

/// Stage ALL deps (path + registry) for a compile command, then prune stale
/// managed entries. `reg` is used only when a member has a registry dep.
pub fn stage_all_deps(
    ws: &lpp_pm::Workspace,
    reg: Option<&lpp_pm::Registry>,
) -> Result<Vec<String>, String> {
    ws.build_plan().map_err(|error| error.to_string())?;
    let reg_names: BTreeSet<String> = registry_deps_needed(ws)?
        .map(|(_, packages)| packages.keys().cloned().collect())
        .unwrap_or_default();
    let path_staged = stage_path_deps(ws, &reg_names)?;
    let reg_staged = stage_registry_deps(ws, reg)?;
    prune_registry_managed(ws, &reg_names)?;
    let mut all = path_staged;
    all.extend(reg_staged);
    all.sort();
    all.dedup();
    Ok(all)
}

// ---------------------------------------------------------------------------
// Jobs
// ---------------------------------------------------------------------------

/// The lpp invocations a member needs: one (target, cmd) row per target.
///
/// A **library** member (`src/lib.lpp`) emits an object file
/// (`--emit-object`, no link, no `main` required); a **binary** member
/// (`src/main.lpp`) links an executable.
pub fn member_jobs(
    member: &lpp_pm::Member,
    out_dir: &Path,
) -> Result<Vec<(String, Vec<String>)>, String> {
    let manifest = &member.manifest;
    let entry = entry_point(&member.dir)?;
    let is_lib = entry == "src/lib.lpp";
    let entry_path = member.dir.join(&entry).display().to_string();
    let targets: Vec<String> = match &manifest.targets {
        Some(t) if !t.supported.is_empty() => t.supported.clone(),
        _ => vec!["host".to_string()],
    };
    let mut rows = Vec::new();
    for target in targets {
        let out_path = if target == "host" {
            if is_lib {
                out_dir.join(format!("{}.o", manifest.name()))
            } else {
                out_dir.join(format!(
                    "{}{}",
                    manifest.name(),
                    std::env::consts::EXE_SUFFIX
                ))
            }
        } else if target.starts_with("wasm") {
            out_dir.join(format!("{}.wasm", manifest.name()))
        } else if is_lib {
            out_dir.join(format!("{}.{}.o", manifest.name(), target))
        } else {
            out_dir.join(format!("{}-{}", manifest.name(), target))
        };
        let mut cmd: Vec<String> = vec![entry_path.clone()];
        if is_lib {
            cmd.push("--emit-object".to_string());
        }
        if target != "host" {
            cmd.push("--target".to_string());
            cmd.push(target.clone());
        }
        cmd.push("-o".to_string());
        cmd.push(out_path.display().to_string());
        rows.push((target, cmd));
    }
    Ok(rows)
}

/// The job-runner unit: given a member name, the cwd to run in, the lpp
/// binary, and its (target, cmd) rows, do the work and return one status
/// label per row (e.g. BUILD/CACHED/FAIL) plus an error if any row failed.
/// The incremental layer (docs/rewrite/DELTA.md) plugs its fingerprint skip
/// in here.
pub type JobRunner =
    dyn Sync + Fn(&str, &Path, &str, &[(String, Vec<String>)]) -> (Vec<String>, Result<(), String>);

fn selected_build_members(
    workspace: &lpp_pm::Workspace,
    only: Option<&str>,
) -> Result<std::collections::BTreeSet<usize>, String> {
    let Some(package) = only else {
        return Ok((0..workspace.members.len()).collect());
    };
    let index = workspace.index();
    let start = index
        .get(package)
        .copied()
        .ok_or_else(|| format!("workspace member not found: {package}"))?;
    let mut selected = std::collections::BTreeSet::new();
    let mut pending = vec![start];
    while let Some(member_index) = pending.pop() {
        if !selected.insert(member_index) {
            continue;
        }
        for dependency in &workspace.members[member_index].path_deps {
            let dependency_index = index.get(dependency.as_str()).copied().ok_or_else(|| {
                format!(
                    "workspace member '{}' depends on missing member '{dependency}'",
                    workspace.members[member_index].name()
                )
            })?;
            pending.push(dependency_index);
        }
    }
    Ok(selected)
}

/// Build the workspace discovered from `dir`, layer by layer over the
/// path-dep DAG; members within a layer build concurrently. All deps (path
/// + registry) are staged first so `import <dep>` resolves.
pub fn build(
    dir: &Path,
    lpp_bin: &str,
    run_job: &JobRunner,
    reg: Option<&lpp_pm::Registry>,
) -> Result<(), String> {
    build_selected(dir, lpp_bin, run_job, reg, None)
}

/// Build either the complete workspace or one named member plus all of its
/// workspace path dependencies. Registry dependencies are staged as usual.
pub fn build_selected(
    dir: &Path,
    lpp_bin: &str,
    run_job: &JobRunner,
    reg: Option<&lpp_pm::Registry>,
    only: Option<&str>,
) -> Result<(), String> {
    let ws = lpp_pm::Workspace::discover(dir).map_err(|e| e.to_string())?;
    let plan = ws.build_plan().map_err(|e| e.to_string())?;
    let selected = selected_build_members(&ws, only)?;
    let out_dir = ws.out_dir();
    std::fs::create_dir_all(&out_dir).map_err(|e| e.to_string())?;
    stage_all_deps(&ws, reg)?;

    // Precompute each member's (target, cmd) rows.
    let mut jobs: Vec<Vec<(String, Vec<String>)>> = Vec::with_capacity(ws.members.len());
    for m in &ws.members {
        jobs.push(member_jobs(m, &out_dir)?);
    }

    let mut b = Builder::default();
    b.push_record([
        "package".to_string(),
        "target".to_string(),
        "status".to_string(),
        "lpp command".to_string(),
    ]);
    for layer in &plan {
        // Deterministic output order within a layer: by member name.
        let mut idx: Vec<usize> = layer
            .iter()
            .copied()
            .filter(|member| selected.contains(member))
            .collect();
        idx.sort_by(|&a, &b2| ws.members[a].name().cmp(ws.members[b2].name()));

        let (tx, rx) = mpsc::channel();
        let cwd = ws.root.clone();
        let parallelism = std::env::var("KEEL_JOBS")
            .ok()
            .and_then(|value| value.parse::<usize>().ok())
            .filter(|value| *value > 0)
            .or_else(|| std::thread::available_parallelism().ok().map(usize::from))
            .unwrap_or(1);
        for chunk in idx.chunks(parallelism) {
            std::thread::scope(|scope| {
                for &member_index in chunk {
                    let tx = tx.clone();
                    let cwd = cwd.clone();
                    let member_name = ws.members[member_index].name().to_string();
                    let rows = jobs[member_index].clone();
                    let lpp = lpp_bin.to_string();
                    scope.spawn(move || {
                        let (statuses, error) = run_job(&member_name, &cwd, &lpp, &rows);
                        let _ = tx.send((member_name, statuses, error));
                    });
                }
            });
        }
        drop(tx);
        let mut layer_err: Option<String> = None;
        let mut results: Vec<_> = rx.into_iter().collect();
        results.sort_by(|left, right| left.0.cmp(&right.0));
        for (name, statuses, err) in results {
            let i = ws
                .index()
                .get(name.as_str())
                .copied()
                .expect("member name from run_job");
            let rows = &jobs[i];
            for (i2, st) in statuses.iter().enumerate() {
                b.push_record([
                    name.clone(),
                    rows[i2].0.clone(),
                    st.clone(),
                    format!("lpp {}", rows[i2].1.join(" ")),
                ]);
            }
            if let Err(e) = err {
                if layer_err.is_none() {
                    layer_err = Some(e);
                }
            }
        }
        if let Some(e) = layer_err {
            return Err(e);
        }
    }
    println!("{}", b.build());
    println!("build OK ({} package(s))", selected.len());
    Ok(())
}

/// The default job runner: invoke `lpp` for every target row (cwd = the
/// workspace root, so `.lpp_packages` module resolution works).
pub fn run_lpp_jobs(
    _member: &str,
    cwd: &Path,
    lpp_bin: &str,
    rows: &[(String, Vec<String>)],
) -> (Vec<String>, Result<(), String>) {
    let mut statuses = Vec::new();
    for (target, cmd) in rows {
        let full: Vec<String> = std::iter::once(lpp_bin.to_string())
            .chain(cmd.iter().cloned())
            .collect();
        match std::process::Command::new(&full[0])
            .args(&full[1..])
            .current_dir(cwd)
            .status()
        {
            Ok(s) if s.success() => statuses.push("BUILD".to_string()),
            Ok(s) => {
                statuses.push("FAIL".to_string());
                return (
                    statuses,
                    Err(format!(
                        "lpp exited with {} for target {target}",
                        s.code().unwrap_or(-1)
                    )),
                );
            }
            Err(e) => {
                statuses.push("FAIL".to_string());
                return (statuses, Err(format!("failed to run lpp ({lpp_bin}): {e}")));
            }
        }
    }
    (statuses, Ok(()))
}

// ---------------------------------------------------------------------------
// Incremental builds — docs/rewrite/DELTA.md
// ---------------------------------------------------------------------------

/// `keel build` with **fingerprint incremental builds**: a (member, target)
/// whose fingerprint matches the durable store — and whose artifact still
/// exists — is CACHED (lpp is not invoked). The store lives at
/// `<out>/target/.keel/fingerprints.toml`.
///
/// `kv` is the opt-in hot layer: when provided, the fingerprint
/// map is mirrored into it after the build.
pub fn build_incremental(
    dir: &Path,
    lpp_bin: &str,
    kv: Option<std::sync::Arc<std::sync::Mutex<Box<dyn lpp_pm::KvCache>>>>,
    reg: Option<&lpp_pm::Registry>,
) -> Result<(), String> {
    build_incremental_selected(dir, lpp_bin, kv, reg, None)
}

/// Incremental form of [`build_selected`].
pub fn build_incremental_selected(
    dir: &Path,
    lpp_bin: &str,
    kv: Option<std::sync::Arc<std::sync::Mutex<Box<dyn lpp_pm::KvCache>>>>,
    reg: Option<&lpp_pm::Registry>,
    only: Option<&str>,
) -> Result<(), String> {
    let ws = lpp_pm::Workspace::discover(dir).map_err(|e| e.to_string())?;
    // Validate selection before taking the workspace lock or touching caches.
    selected_build_members(&ws, only)?;
    let _guard = lock_workspace(&ws.root)?;
    // Stage and verify the exact locked dependency graph before computing any
    // fingerprints. Registry checksums from the lock are folded in below.
    stage_all_deps(&ws, reg)?;
    let lock = std::fs::read_to_string(ws.root.join("Keel.lock"))
        .ok()
        .map(|document| lpp_pm::Lock::parse(&document))
        .transpose()
        .map_err(|error| error.to_string())?;
    let store_path = ws.out_dir().join(".keel").join("fingerprints.toml");
    let mut store = lpp_pm::fingerprint::FingerprintStore::load(&store_path);
    let current = lpp_pm::fingerprint::compute_member_fps_with_lock(&ws, lpp_bin, lock.as_ref())
        .map_err(|e| e.to_string())?;

    // Delta report: what changed since the last build, and the rebuild set.
    let delta = lpp_pm::delta::diff(&store.fps(), &current);
    if !delta.is_empty() {
        let rebuild = lpp_pm::delta::invalidate(&delta, &lpp_pm::fingerprint::dep_key_graph(&ws));
        println!(
            "delta: {} new/changed, {} removed → rebuilding {} of {} job(s)",
            delta.added.len() + delta.changed.len(),
            delta.removed.len(),
            rebuild.len(),
            current.len()
        );
    } else if !current.is_empty() {
        println!("delta: no changes → {} job(s) up to date", current.len());
    }

    store.prune(&current);
    let store = std::sync::Arc::new(std::sync::Mutex::new(store));
    let current = std::sync::Arc::new(current);
    let current2 = current.clone();
    let store2 = store.clone();
    let res = build_selected(
        dir,
        lpp_bin,
        &move |_member, cwd, lpp, rows| {
            incremental_job(_member, cwd, lpp, rows, &current2, &store2)
        },
        reg,
        only,
    );

    if res.is_ok() {
        if let Some(k) = &kv {
            let mut kv = k.lock().expect("kv lock");
            lpp_pm::delta::mirror_to_kv(kv.as_mut(), &current);
        }
        let s = store.lock().expect("store lock");
        s.save().map_err(|e| e.to_string())?;
    }
    res
}

/// The incremental job runner: CACHED when fingerprint + artifact match,
/// else BUILD via `lpp`, recording the new fingerprint on success.
fn incremental_job(
    member: &str,
    cwd: &Path,
    lpp_bin: &str,
    rows: &[(String, Vec<String>)],
    current: &std::sync::Arc<std::collections::BTreeMap<String, String>>,
    store: &std::sync::Arc<std::sync::Mutex<lpp_pm::fingerprint::FingerprintStore>>,
) -> (Vec<String>, Result<(), String>) {
    let mut statuses = Vec::new();
    for (target, cmd) in rows {
        let key = format!("{member}|{target}");
        let Some(fp) = current.get(&key) else {
            statuses.push("FAIL".to_string());
            return (
                statuses,
                Err(format!("no fingerprint for {key} — internal error")),
            );
        };
        // CACHED = fingerprint matches AND the artifact still exists.
        let out_path = cmd.last().map(String::as_str);
        let cached = {
            let store = store.lock().expect("store lock");
            store.get(&key).is_some_and(|entry| {
                entry.fingerprint == *fp
                    && out_path.is_some_and(|path| {
                        entry.artifact_hash.as_ref()
                            == lpp_pm::fingerprint::hash_file(Path::new(path)).as_ref()
                    })
            })
        };
        if cached {
            statuses.push("CACHED".to_string());
            continue;
        }
        let full: Vec<String> = std::iter::once(lpp_bin.to_string())
            .chain(cmd.iter().cloned())
            .collect();
        match std::process::Command::new(&full[0])
            .args(&full[1..])
            .current_dir(cwd)
            .status()
        {
            Ok(s) if s.success() => {
                let artifact_hash =
                    out_path.and_then(|path| lpp_pm::fingerprint::hash_file(Path::new(path)));
                if artifact_hash.is_none() {
                    statuses.push("FAIL".to_string());
                    return (
                        statuses,
                        Err(format!("lpp succeeded but produced no artifact for {key}")),
                    );
                }
                let mut store = store.lock().expect("store lock");
                store.upsert_artifact(&key, fp, artifact_hash);
                if let Err(error) = store.save() {
                    statuses.push("FAIL".to_string());
                    return (statuses, Err(error.to_string()));
                }
                statuses.push("BUILD".to_string());
            }
            Ok(s) => {
                statuses.push("FAIL".to_string());
                return (
                    statuses,
                    Err(format!(
                        "lpp exited with {} for target {target}",
                        s.code().unwrap_or(-1)
                    )),
                );
            }
            Err(e) => {
                statuses.push("FAIL".to_string());
                return (statuses, Err(format!("failed to run lpp ({lpp_bin}): {e}")));
            }
        }
    }
    (statuses, Ok(()))
}

// ---------------------------------------------------------------------------
// check / run
// ---------------------------------------------------------------------------

fn selected_member<'a>(
    workspace: &'a lpp_pm::Workspace,
    invoked_from: &Path,
) -> Result<&'a lpp_pm::Member, String> {
    let absolute = if invoked_from.is_absolute() {
        invoked_from.to_path_buf()
    } else {
        std::env::current_dir()
            .map_err(|error| error.to_string())?
            .join(invoked_from)
    };
    let invoked_from = absolute.canonicalize().map_err(|error| error.to_string())?;
    let mut candidates: Vec<&lpp_pm::Member> = workspace
        .members
        .iter()
        .filter(|member| invoked_from == member.dir || invoked_from.starts_with(&member.dir))
        .collect();
    candidates.sort_by_key(|member| std::cmp::Reverse(member.dir.components().count()));
    if let Some(member) = candidates.first() {
        return Ok(*member);
    }
    if workspace.members.len() == 1 && !workspace.virtual_root {
        return Ok(&workspace.members[0]);
    }
    Err(
        "run/check from a virtual workspace root is ambiguous; invoke Keel inside a package member"
            .to_string(),
    )
}

/// `keel check` — type-check only, no codegen: `lpp <entry> --check`.
pub fn check(dir: &Path, lpp_bin: &str, reg: Option<&lpp_pm::Registry>) -> Result<(), String> {
    let ws = lpp_pm::Workspace::discover(dir).map_err(|e| e.to_string())?;
    let _guard = lock_workspace(&ws.root)?;
    let member = selected_member(&ws, dir)?;
    stage_all_deps(&ws, reg)?;
    let entry = member.dir.join(entry_point(&member.dir)?);
    let status = std::process::Command::new(lpp_bin)
        .arg(&entry)
        .arg("--check")
        .current_dir(&ws.root)
        .status()
        .map_err(|e| format!("failed to run lpp ({lpp_bin}): {e}"))?;
    if !status.success() {
        return Err(format!(
            "lpp --check exited with {}",
            status.code().unwrap_or(-1)
        ));
    }
    println!("check OK");
    Ok(())
}

/// `keel run` — build + run the project: `lpp <entry> --run`.
pub fn run(dir: &Path, lpp_bin: &str, reg: Option<&lpp_pm::Registry>) -> Result<(), String> {
    let ws = lpp_pm::Workspace::discover(dir).map_err(|e| e.to_string())?;
    let _guard = lock_workspace(&ws.root)?;
    let member = selected_member(&ws, dir)?;
    stage_all_deps(&ws, reg)?;
    let entry = member.dir.join(entry_point(&member.dir)?);
    let status = std::process::Command::new(lpp_bin)
        .arg(&entry)
        .arg("--run")
        .current_dir(&ws.root)
        .status()
        .map_err(|e| format!("failed to run lpp ({lpp_bin}): {e}"))?;
    if !status.success() {
        return Err(format!("lpp exited with {}", status.code().unwrap_or(-1)));
    }
    Ok(())
}
