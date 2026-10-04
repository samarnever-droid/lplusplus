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

use tabled::builder::Builder;

/// The `lpp` compiler binary (from `KEEL_LPP`, else `lpp` on PATH).
pub fn lpp_bin() -> String {
    std::env::var("KEEL_LPP")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "lpp".to_string())
}

fn manifest_in(dir: &Path) -> Result<lpp_pm::manifest::Manifest, String> {
    let p = dir.join("Keel.toml");
    if !p.exists() {
        return Err("no Keel.toml in the project directory".to_string());
    }
    lpp_pm::manifest::Manifest::parse(&std::fs::read_to_string(&p).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())
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
            return Err(format!(
                "refusing to replace real directory {} (remove it first)",
                link.display()
            ));
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
    copy_tree(target, link).map_err(|e| format!("copy: {e}"))
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
        std::fs::create_dir_all(&pdir).map_err(|e| e.to_string())?;

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
            let managed = std::fs::read_to_string(e.path().join("lpp.toml"))
                .map(|s| s.contains("managed = \"keel\""))
                .unwrap_or(false);
            if managed {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
    Ok(staged)
}

// Registry-dep staging (.lpp_packages from the git registry) —
// docs/rewrite/DEP_LINKING.md (slice 11).
// ---------------------------------------------------------------------------

/// The registry deps every member needs, as `name → (locked version, locked
/// checksum)`. `Ok(None)` = no member has a registry dep (no lock needed).
fn registry_deps_needed(
    ws: &lpp_pm::Workspace,
) -> Result<Option<BTreeMap<String, (String, String)>>, String> {
    let index = ws.index();
    let mut names: Vec<String> = Vec::new();
    for m in &ws.members {
        for (dep_name, dep) in &m.manifest.dependencies {
            if dep.path().is_none()
                && !index.contains_key(dep_name.as_str())
                && !names.iter().any(|n| n == dep_name)
            {
                names.push(dep_name.clone());
            }
        }
    }
    if names.is_empty() {
        return Ok(None);
    }
    let lock = lpp_pm::Lock::parse(&std::fs::read_to_string(ws.root.join("Keel.lock")).map_err(
        |_| {
            format!(
                "registry dependency '{}' needs a Keel.lock — run `keel fetch` first",
                names[0]
            )
        },
    )?)
    .map_err(|e| e.to_string())?;
    let mut needed = BTreeMap::new();
    for name in &names {
        let p = lock
            .packages
            .iter()
            .find(|p| p.name == *name)
            .ok_or_else(|| {
                format!("registry dependency '{name}' is not in Keel.lock — run `keel fetch` first")
            })?;
        let checksum = p.checksum.clone().ok_or_else(|| {
            format!("Keel.lock has no checksum for '{name}' — re-run `keel fetch`")
        })?;
        needed.insert(name.clone(), (p.version.clone(), checksum));
    }
    Ok(Some(needed))
}

/// Stage every registry dep of every member into `<root>/.lpp_packages/<dep>/`:
/// fetch the locked artifact (verified against the index), **re-hash it
/// against the lockfile's checksum** (the same supply-chain guarantee as
/// `keel verify` — a tampered registry is refused at build time), extract
/// the tar.gz, and write the managed staging manifest. Idempotent: an entry
/// whose staged `version`+`checksum` already match is not touched.
/// `reg` is needed only when a member has a registry dep.
pub fn stage_registry_deps(
    ws: &lpp_pm::Workspace,
    reg: Option<&lpp_pm::Registry>,
) -> Result<Vec<String>, String> {
    let needed = match registry_deps_needed(ws)? {
        Some(n) => n,
        None => return Ok(Vec::new()),
    };
    let reg = reg.ok_or_else(|| {
        format!(
            "no registry configured (needed for {} registry dep(s)): pass --registry <git-url> or set KEEL_REGISTRY",
            needed.len()
        )
    })?;
    reg.sync().map_err(|e| e.to_string())?;

    let pkgs_dir = ws.root.join(".lpp_packages");
    std::fs::create_dir_all(&pkgs_dir).map_err(|e| e.to_string())?;

    let mut staged = Vec::new();
    for (name, (version, checksum)) in &needed {
        let pdir = pkgs_dir.join(name);

        // Idempotency: staged manifest with the same version + checksum.
        if let Ok(existing) = std::fs::read_to_string(pdir.join("lpp.toml")) {
            if existing.contains(&format!("version = \"{version}\""))
                && existing.contains(&format!("checksum = \"{checksum}\""))
                && existing.contains("managed = \"keel\"")
            {
                staged.push(name.clone());
                continue;
            }
        }

        // Fetch (artifact == index checksum) …
        let (bytes, _entry) = reg
            .fetch(name, version)
            .map_err(|e| format!("registry dep '{name}': {e}"))?;
        // … and re-verify against the LOCK (the source of truth).
        let actual = lpp_pm::ContentAddress::of_bytes(&bytes).to_string();
        if actual != *checksum {
            return Err(lpp_pm::PmError::ChecksumMismatch {
                expected: checksum.clone(),
                actual,
            }
            .to_string());
        }

        // Extract into a temp dir, then replace the entry atomically.
        let tmp = pkgs_dir.join(format!(".tmp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let tar_path = pkgs_dir.join(format!(".artifact-{name}-{}.tar.gz", std::process::id()));
        std::fs::write(&tar_path, &bytes).map_err(|e| e.to_string())?;
        let out = std::process::Command::new("tar")
            .args([
                "-xzf",
                tar_path.to_str().unwrap(),
                "-C",
                tmp.to_str().unwrap(),
            ])
            .output()
            .map_err(|e| format!("failed to run `tar`: {e}"))?;
        let _ = std::fs::remove_file(&tar_path);
        if !out.status.success() {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(format!(
                "failed to extract registry package '{name}': {}",
                String::from_utf8_lossy(&out.stderr)
            ));
        }

        let entry = if tmp.join("src/lib.lpp").is_file() {
            "src/lib.lpp"
        } else if tmp.join("src/main.lpp").is_file() {
            "src/main.lpp"
        } else {
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(format!(
                "registry package '{name}' has no src/lib.lpp or src/main.lpp entry"
            ));
        };

        if pdir.exists() {
            let managed = std::fs::read_to_string(pdir.join("lpp.toml"))
                .map(|s| s.contains("managed = \"keel\""))
                .unwrap_or(false);
            if !managed {
                let _ = std::fs::remove_dir_all(&tmp);
                return Err(format!(
                    "refusing to overwrite non-Keel .lpp_packages/{name} (remove it first)"
                ));
            }
            std::fs::remove_dir_all(&pdir).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&tmp, &pdir).map_err(|e| e.to_string())?;

        let doc = format!(
            "[package]\nname = \"{name}\"\nversion = \"{version}\"\nentry = \"{entry}\"\nchecksum = \"{checksum}\"\nsource = \"registry\"\nmanaged = \"keel\"\n"
        );
        std::fs::write(pdir.join("lpp.toml"), &doc).map_err(|e| e.to_string())?;
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
            let doc = std::fs::read_to_string(e.path().join("lpp.toml")).unwrap_or_default();
            if doc.contains("managed = \"keel\"") && doc.contains("source = \"registry\"") {
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
    let reg_names: BTreeSet<String> = registry_deps_needed(ws)?
        .map(|m| m.keys().cloned().collect())
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
                out_dir.join(manifest.name())
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

/// Build the workspace discovered from `dir`, layer by layer over the
/// path-dep DAG; members within a layer build concurrently. All deps (path
/// + registry) are staged first so `import <dep>` resolves.
pub fn build(
    dir: &Path,
    lpp_bin: &str,
    run_job: &JobRunner,
    reg: Option<&lpp_pm::Registry>,
) -> Result<(), String> {
    let ws = lpp_pm::Workspace::discover(dir).map_err(|e| e.to_string())?;
    let plan = ws.build_plan().map_err(|e| e.to_string())?;
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
        let mut idx: Vec<usize> = layer.clone();
        idx.sort_by(|&a, &b2| ws.members[a].name().cmp(ws.members[b2].name()));

        let (tx, rx) = mpsc::channel();
        let cwd = ws.root.clone();
        std::thread::scope(|s| {
            for &mi in &idx {
                let tx = tx.clone();
                let cwd = cwd.clone();
                let member_name = ws.members[mi].name().to_string();
                let rows = jobs[mi].clone();
                let lpp = lpp_bin.to_string();
                s.spawn(move || {
                    let (statuses, err) = run_job(&member_name, &cwd, &lpp, &rows);
                    let _ = tx.send((member_name, statuses, err));
                });
            }
            drop(tx);
        });
        let mut layer_err: Option<String> = None;
        for (name, statuses, err) in rx {
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
    println!("build OK ({} package(s))", ws.members.len());
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
    let ws = lpp_pm::Workspace::discover(dir).map_err(|e| e.to_string())?;
    let store_path = ws.out_dir().join(".keel").join("fingerprints.toml");
    let mut store = lpp_pm::fingerprint::FingerprintStore::load(&store_path);
    let current =
        lpp_pm::fingerprint::compute_member_fps(&ws, lpp_bin).map_err(|e| e.to_string())?;

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
    let res = build(
        dir,
        lpp_bin,
        &move |_member, cwd, lpp, rows| {
            incremental_job(_member, cwd, lpp, rows, &current2, &store2)
        },
        reg,
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
            let s = store.lock().expect("store lock");
            s.get(&key).map(|e| e.fingerprint == *fp).unwrap_or(false)
                && out_path.map(|p| Path::new(p).exists()).unwrap_or(false)
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
                let mut st = store.lock().expect("store lock");
                st.upsert(&key, fp);
                let _ = st.save();
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

/// `keel check` — type-check only, no codegen: `lpp <entry> --check`.
pub fn check(dir: &Path, lpp_bin: &str, reg: Option<&lpp_pm::Registry>) -> Result<(), String> {
    let ws = lpp_pm::Workspace::discover(dir).map_err(|e| e.to_string())?;
    manifest_in(dir)?;
    stage_all_deps(&ws, reg)?;
    let entry = dir.join(entry_point(dir)?);
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
    manifest_in(dir)?;
    stage_all_deps(&ws, reg)?;
    let entry = dir.join(entry_point(dir)?);
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
