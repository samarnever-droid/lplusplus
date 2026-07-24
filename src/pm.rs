use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

#[derive(Debug)]
#[allow(dead_code)]
pub struct Dependency {
    pub name: String,
    pub version: Option<String>,
    pub git: Option<String>,
    pub tag: Option<String>,
    pub branch: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    pub entry: Option<String>,
    pub dependencies: Vec<Dependency>,
}

pub struct RegistryEntry {
    pub git: String,
    pub branch: Option<String>,
    pub tag: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedPackage {
    pub name: String,
    pub version: Option<String>,
    pub source: String,
    pub resolved: Option<String>,
}

pub fn parse_toml(content: &str) -> Result<Package, String> {
    let mut name = String::new();
    let mut version = String::new();
    let mut author = None;
    let mut entry = None;
    let mut dependencies = Vec::new();

    let mut current_section = "";

    for (line_idx, raw_line) in content.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            current_section = &line[1..line.len() - 1];
            continue;
        }

        if let Some(eq_idx) = line.find('=') {
            let key = line[..eq_idx].trim();
            let val_str = line[eq_idx + 1..].trim();

            match current_section {
                "package" => {
                    let cleaned_val = val_str.trim_matches('"').trim_matches('\'').to_string();
                    if key == "name" {
                        name = cleaned_val;
                    } else if key == "version" {
                        version = cleaned_val;
                    } else if key == "author" {
                        author = Some(cleaned_val);
                    } else if key == "entry" {
                        entry = Some(cleaned_val);
                    }
                }
                "dependencies" => {
                    if val_str.starts_with('{') && val_str.ends_with('}') {
                        let inline = &val_str[1..val_str.len() - 1];
                        let mut git = None;
                        let mut version = None;
                        let mut tag = None;
                        let mut branch = None;
                        let mut path = None;

                        for part in inline.split(',') {
                            if let Some(p_eq) = part.find('=') {
                                let pk = part[..p_eq].trim();
                                let pv = part[p_eq + 1..]
                                    .trim()
                                    .trim_matches('"')
                                    .trim_matches('\'')
                                    .trim()
                                    .to_string();
                                if pk == "git" {
                                    git = Some(pv);
                                } else if pk == "version" {
                                    version = Some(pv);
                                } else if pk == "tag" {
                                    tag = Some(pv);
                                } else if pk == "branch" {
                                    branch = Some(pv);
                                } else if pk == "path" {
                                    path = Some(pv);
                                }
                            }
                        }
                        dependencies.push(Dependency {
                            name: key.to_string(),
                            version,
                            git,
                            tag,
                            branch,
                            path,
                        });
                    } else {
                        return Err(format!(
                            "Line {}: invalid dependency value '{}'. Must be an inline table {{ ... }}",
                            line_idx + 1,
                            val_str
                        ));
                    }
                }
                _ => {}
            }
        } else {
            return Err(format!(
                "Line {}: invalid TOML syntax '{}'",
                line_idx + 1,
                line
            ));
        }
    }

    if name.is_empty() {
        return Err("Missing package name in [package] section".to_string());
    }
    if version.is_empty() {
        return Err("Missing package version in [package] section".to_string());
    }

    Ok(Package {
        name,
        version,
        author,
        entry,
        dependencies,
    })
}

pub fn resolve_entry_point() -> String {
    if std::path::Path::new("lpp.toml").exists() {
        if let Ok(content) = fs::read_to_string("lpp.toml") {
            if let Ok(pkg) = parse_toml(&content) {
                if let Some(entry) = pkg.entry {
                    return entry;
                }
            }
        }
    }
    if std::path::Path::new("src/main.lpp").exists() {
        "src/main.lpp".to_string()
    } else if std::path::Path::new("main.lpp").exists() {
        "main.lpp".to_string()
    } else {
        "src/main.lpp".to_string()
    }
}

fn normalize_package_name(name: &str) -> String {
    name.chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn scaffold_toml(package_name: &str) -> String {
    format!(
        "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nauthor = \"Khati\"\nentry = \"src/main.lpp\"\n\n[dependencies]\n",
        package_name
    )
}

fn write_project_scaffold(base_dir: &Path, package_name: &str) -> Result<(), String> {
    fs::create_dir_all(base_dir.join("src"))
        .map_err(|e| format!("Failed to create src/ directory: {}", e))?;
    fs::write(base_dir.join("lpp.toml"), scaffold_toml(package_name))
        .map_err(|e| format!("Failed to write lpp.toml: {}", e))?;
    fs::write(
        base_dir.join("src").join("main.lpp"),
        "def main():\n    print_str(\"Hello from L++ project!\")\n",
    )
    .map_err(|e| format!("Failed to write src/main.lpp: {}", e))?;
    fs::write(
        base_dir.join(".gitignore"),
        ".lpp_packages/\ntarget/\noutput.c\noutput.obj\n*.obj\n*.exe\n*.o\n",
    )
    .map_err(|e| format!("Failed to write .gitignore: {}", e))?;
    Ok(())
}

fn read_manifest() -> Result<Package, String> {
    let content =
        fs::read_to_string("lpp.toml").map_err(|e| format!("Failed to read lpp.toml: {}", e))?;
    parse_toml(&content)
}

fn parse_lockfile(content: &str) -> Vec<LockedPackage> {
    let mut packages = Vec::new();
    let mut current: Option<LockedPackage> = None;
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line == "[[package]]" {
            if let Some(pkg) = current.take() {
                packages.push(pkg);
            }
            current = Some(LockedPackage {
                name: String::new(),
                version: None,
                source: String::new(),
                resolved: None,
            });
            continue;
        }
        if let Some(eq_idx) = line.find('=') {
            let key = line[..eq_idx].trim();
            let value = line[eq_idx + 1..].trim().trim_matches('"').to_string();
            if let Some(pkg) = current.as_mut() {
                match key {
                    "name" => pkg.name = value,
                    "version" => pkg.version = Some(value),
                    "source" => pkg.source = value,
                    "resolved" => pkg.resolved = Some(value),
                    _ => {}
                }
            }
        }
    }
    if let Some(pkg) = current {
        packages.push(pkg);
    }
    packages
}

fn read_lockfile() -> Vec<LockedPackage> {
    fs::read_to_string("lpp.lock")
        .map(|content| parse_lockfile(&content))
        .unwrap_or_default()
}

fn registry_package_names() -> Vec<String> {
    let json =
        fs::read_to_string(Path::new("githubpage").join("registry.json")).unwrap_or_default();
    let mut names = Vec::new();
    let mut in_packages = false;
    for raw_line in json.lines() {
        let line = raw_line.trim();
        if line.starts_with("\"packages\"") {
            in_packages = true;
            continue;
        }
        if in_packages && line.starts_with('}') {
            break;
        }
        if in_packages && line.starts_with('"') {
            if let Some(end_quote) = line[1..].find('"') {
                names.push(line[1..1 + end_quote].to_string());
            }
        }
    }
    names
}

fn command_available(program: &str, probe_args: &[&str]) -> bool {
    std::process::Command::new(program)
        .args(probe_args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
}

fn current_compiler_path() -> Result<PathBuf, String> {
    std::env::current_exe().map_err(|e| format!("Failed to locate current lpp binary: {}", e))
}

fn current_binary_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
}

fn installed_root_dir() -> Option<PathBuf> {
    let exe_dir = current_binary_dir()?;
    if exe_dir.file_name().and_then(|s| s.to_str()) == Some("bin") {
        exe_dir.parent().map(Path::to_path_buf)
    } else {
        None
    }
}

#[allow(dead_code)]
fn resolve_runtime_source() -> Option<PathBuf> {
    for var in &["LPP_HOME", "LPP_DIR"] {
        if let Ok(val) = std::env::var(var) {
            let rt = PathBuf::from(&val).join("lpp_runtime.c");
            if rt.exists() {
                return Some(rt);
            }
            let lib_rt = PathBuf::from(&val).join("lib").join("lpp_runtime.c");
            if lib_rt.exists() {
                return Some(lib_rt);
            }
        }
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let candidates = [
                exe_dir.join("lpp_runtime.c"),
                exe_dir.join("lib/lpp_runtime.c"),
                exe_dir.join("../lpp_runtime.c"),
                exe_dir.join("../lib/lpp_runtime.c"),
                exe_dir.join("../../lpp_runtime.c"),
                exe_dir.join("../../lib/lpp_runtime.c"),
                exe_dir.join("../../../lpp_runtime.c"),
                exe_dir.join("../../../lib/lpp_runtime.c"),
            ];
            for c in &candidates {
                if c.exists() {
                    return Some(c.clone());
                }
            }
        }
    }

    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        let home_rt = PathBuf::from(&home).join(".lpp/lib/lpp_runtime.c");
        if home_rt.exists() {
            return Some(home_rt);
        }
        let home_rt_root = PathBuf::from(&home).join(".lpp/lpp_runtime.c");
        if home_rt_root.exists() {
            return Some(home_rt_root);
        }
    }

    let workspace_runtime = Path::new("lpp_runtime.c");
    if workspace_runtime.exists() {
        return Some(workspace_runtime.to_path_buf());
    }

    installed_root_dir()
        .map(|root| root.join("lib").join("lpp_runtime.c"))
        .filter(|path| path.exists())
}

#[allow(dead_code)]
fn resolve_runtime_object() -> Option<PathBuf> {
    let extension = if cfg!(windows) { "obj" } else { "o" };
    let filename = format!("lpp_runtime.{}", extension);

    for var in &["LPP_HOME", "LPP_DIR"] {
        if let Ok(val) = std::env::var(var) {
            let lib_obj = PathBuf::from(val).join("lib").join(&filename);
            if lib_obj.exists() {
                return Some(lib_obj);
            }
        }
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let candidates = [
                exe_dir.join(&filename),
                exe_dir.join(format!("lib/{}", filename)),
                exe_dir.join(format!("../lib/{}", filename)),
                exe_dir.join(format!("../../lib/{}", filename)),
                exe_dir.join(format!("../../../lib/{}", filename)),
            ];
            for c in &candidates {
                if c.exists() {
                    return Some(c.clone());
                }
            }
        }
    }

    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        let home_obj = PathBuf::from(&home).join(".lpp/lib").join(&filename);
        if home_obj.exists() {
            return Some(home_obj);
        }
    }

    installed_root_dir()
        .map(|root| root.join("lib").join(&filename))
        .filter(|path| path.exists())
}

fn native_binary_suffix() -> &'static str {
    std::env::consts::EXE_SUFFIX
}

fn binary_file_name(name: &str) -> String {
    format!("{}{}", name, native_binary_suffix())
}

fn output_path_for_name(dir: &Path, name: &str) -> PathBuf {
    dir.join(binary_file_name(name))
}

#[allow(dead_code)]
enum LinkStrategy {
    #[cfg_attr(not(windows), allow(dead_code))]
    MsvcLink { runtime_obj: PathBuf },
    /// Host linker/compiler invocation with a prebuilt L++ runtime object.
    /// This is Phase 1 of the native-linker roadmap: user builds no longer
    /// compile lpp_runtime.c on every project build.
    CCompilerObject {
        compiler: String,
        runtime_obj: PathBuf,
    },
    CCompiler {
        compiler: String,
        runtime_src: PathBuf,
    },
}

#[allow(dead_code)]
fn detect_link_strategy() -> Result<LinkStrategy, String> {
    #[cfg(windows)]
    {
        load_msvc_env();
        if command_available("link.exe", &["/?"]) {
            if let Some(runtime_obj) = resolve_runtime_object() {
                return Ok(LinkStrategy::MsvcLink { runtime_obj });
            }
        }
        if command_available("cl.exe", &["/?"]) {
            let runtime_src = resolve_runtime_source()
                .ok_or_else(|| "Failed to locate lpp_runtime.c for native linking.".to_string())?;
            return Ok(LinkStrategy::CCompiler {
                compiler: "cl.exe".to_string(),
                runtime_src,
            });
        }
    }

    for compiler in ["cc", "gcc", "clang"] {
        if command_available(compiler, &["--version"]) {
            if let Some(runtime_obj) = resolve_runtime_object() {
                return Ok(LinkStrategy::CCompilerObject {
                    compiler: compiler.to_string(),
                    runtime_obj,
                });
            }
            let runtime_src = resolve_runtime_source()
                .ok_or_else(|| "Failed to locate lpp_runtime.c for native linking.".to_string())?;
            return Ok(LinkStrategy::CCompiler {
                compiler: compiler.to_string(),
                runtime_src,
            });
        }
    }

    Err(
        "No supported native linker/compiler found. Install MSVC build tools, cc, gcc, or clang."
            .to_string(),
    )
}

#[allow(dead_code)]
fn should_use_mold(compiler: &str) -> Result<bool, String> {
    if compiler.eq_ignore_ascii_case("cl.exe") {
        return Ok(false);
    }
    let requested_mold = std::env::var("LPP_LINKER").ok().as_deref() == Some("mold");
    let has_mold = command_available("mold", &["--version"]);
    if requested_mold && !has_mold {
        return Err(
            "LPP_LINKER=mold was requested, but 'mold' binary was not found in PATH.".to_string(),
        );
    }
    Ok(requested_mold || has_mold)
}

fn package_cache_key(source_path: &Path) -> Result<String, String> {
    // Cache correctness is more important than cache hit rate: hash every L++
    // source in src/, the manifest, compiler version, target, and AOT profile.
    let mut files = Vec::new();
    if Path::new("src").is_dir() {
        for entry in fs::read_dir("src").map_err(|e| format!("read src for cache: {}", e))? {
            let path = entry
                .map_err(|e| format!("read cache entry: {}", e))?
                .path();
            if path.extension().is_some_and(|ext| ext == "lpp") {
                files.push(path);
            }
        }
    }
    if !files.iter().any(|path| path == source_path) {
        files.push(source_path.to_path_buf());
    }
    files.sort();
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    env!("CARGO_PKG_VERSION").hash(&mut hasher);
    std::env::consts::OS.hash(&mut hasher);
    std::env::consts::ARCH.hash(&mut hasher);
    std::env::var("LPP_AOT_OPT")
        .unwrap_or_else(|_| "none".to_string())
        .hash(&mut hasher);
    if let Ok(linker_var) = std::env::var("LPP_LINKER") {
        linker_var.hash(&mut hasher);
    }
    command_available("mold", &["--version"]).hash(&mut hasher);
    for path in files {
        path.to_string_lossy().hash(&mut hasher);
        fs::read(&path)
            .map_err(|e| format!("read '{}' for cache: {}", path.display(), e))?
            .hash(&mut hasher);
    }
    if let Ok(manifest) = fs::read("lpp.toml") {
        manifest.hash(&mut hasher);
    }
    Ok(format!("{:016x}", hasher.finish()))
}

fn compile_source_to_object(source_path: &Path) -> Result<PathBuf, String> {
    let compiler_path = current_compiler_path()?;
    let obj_file = source_path.with_extension("o");
    let cache_dir = Path::new("LppData").join("cache");
    let cache_key = package_cache_key(source_path)?;
    let cache_object = cache_dir.join(format!("{}.o", cache_key));
    if cache_object.exists() {
        fs::copy(&cache_object, &obj_file).map_err(|e| format!("restore cached object: {}", e))?;
        println!("  Cache hit: {}", cache_key);
        return Ok(obj_file);
    }
    let status = std::process::Command::new(&compiler_path)
        .env("LPP_AOT", "1")
        // Package builds consume the object file directly. Skipping the
        // compatibility C artifact avoids a second full backend pass without
        // changing AOT semantics or explicit `lpp emit --aot` behavior.
        .env("LPP_AOT_ONLY", "1")
        .env("BENCHMARK", "1")
        .arg(source_path)
        .stdin(std::process::Stdio::null())
        .status()
        .map_err(|e| {
            format!(
                "Failed to start compiler '{}': {}",
                compiler_path.display(),
                e
            )
        })?;

    if !status.success() {
        return Err(format!(
            "Compilation failed for '{}'.",
            source_path.display()
        ));
    }
    if !obj_file.exists() {
        return Err(format!(
            "Compiled object file '{}' was not generated.",
            obj_file.display()
        ));
    }
    fs::create_dir_all(&cache_dir).map_err(|e| format!("create LppData cache: {}", e))?;
    fs::copy(&obj_file, &cache_object).map_err(|e| format!("write cached object: {}", e))?;
    println!("  Cache miss: stored {}", cache_key);
    Ok(obj_file)
}

#[cfg(windows)]
fn find_vcvars64() -> Option<PathBuf> {
    let fallbacks = [
        "C:\\Program Files\\Microsoft Visual Studio\\2022\\Community\\VC\\Auxiliary\\Build\\vcvars64.bat",
        "C:\\Program Files\\Microsoft Visual Studio\\2022\\Professional\\VC\\Auxiliary\\Build\\vcvars64.bat",
        "C:\\Program Files\\Microsoft Visual Studio\\2022\\Enterprise\\VC\\Auxiliary\\Build\\vcvars64.bat",
        "C:\\Program Files\\Microsoft Visual Studio\\2019\\Community\\VC\\Auxiliary\\Build\\vcvars64.bat",
        "C:\\Program Files\\Microsoft Visual Studio\\2019\\Professional\\VC\\Auxiliary\\Build\\vcvars64.bat",
        "C:\\Program Files\\Microsoft Visual Studio\\2019\\Enterprise\\VC\\Auxiliary\\Build\\vcvars64.bat",
    ];
    for fallback in &fallbacks {
        let p = Path::new(fallback);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }
    None
}

#[allow(dead_code)]
/// Compute a simple hash of a file's contents for cache invalidation.
/// Uses Rust's built-in DefaultHasher (SipHash) — not cryptographic, but
/// fast and sufficient for detecting source changes.
fn file_content_hash(path: &Path) -> Option<u64> {
    let data = fs::read(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    Some(hasher.finish())
}

/// Target triple string for multi-arch cache layout.
fn runtime_cache_target() -> &'static str {
    if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        "linux-x86_64"
    } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
        "linux-aarch64"
    } else if cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        "windows-x86_64"
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        "macos-x86_64"
    } else if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        "macos-arm64"
    } else {
        "unknown"
    }
}

fn resolve_min_runtime_object() -> Option<PathBuf> {
    let ext = if cfg!(target_os = "windows") { "obj" } else { "o" };
    let filename = format!("lpp_runtime_min.{}", ext);

    let src_name = if cfg!(target_os = "windows") {
        "runtime/windows_x86_64_min.c"
    } else {
        "runtime/linux_x86_64_min.c"
    };

    let local_src = Path::new(src_name);
    // Multi-arch cache: LppData/cache/<target>/lpp_runtime_min.o
    let cache_dir = Path::new("LppData").join("cache").join(runtime_cache_target());
    let cache_obj = cache_dir.join(&filename);
    let cache_hash = cache_dir.join("runtime.hash");

    if local_src.exists() {
        // Hash-based invalidation: compare source hash with stored hash
        let current_hash = file_content_hash(local_src);
        let stored_hash = fs::read_to_string(&cache_hash)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok());

        let needs_rebuild = match (current_hash, stored_hash) {
            (Some(cur), Some(stored)) => cur != stored || !cache_obj.exists(),
            _ => true, // no hash or can't read → rebuild
        };

        if needs_rebuild {
            let _ = fs::create_dir_all(&cache_dir);
            let cc = if cfg!(windows) { "cl.exe" } else { "gcc" };
            let mut cmd = std::process::Command::new(cc);
            if cfg!(windows) {
                cmd.arg("/nologo")
                    .arg("/O2")
                    .arg("/GS-")
                    .arg("/Gs1000000")
                    .arg("/DLPP_FREESTANDING")
                    .arg("/c")
                    .arg(local_src)
                    .arg(format!("/Fo:{}", cache_obj.display()));
            } else {
                cmd.arg("-Os")
                    .arg("-fno-stack-protector")
                    .arg("-ffreestanding")
                    .arg("-fno-pic")
                    .arg("-mno-red-zone")
                    .arg("-fno-reorder-blocks-and-partition")
                    .arg("-DLPP_FREESTANDING")
                    .arg("-c")
                    .arg(local_src)
                    .arg("-o")
                    .arg(&cache_obj);
            }
            if cmd.status().map_or(false, |s| s.success()) && cache_obj.exists() {
                // Store the hash for next time
                if let Some(h) = current_hash {
                    let _ = fs::write(&cache_hash, format!("{}\n", h));
                }
                return Some(cache_obj);
            }
        } else {
            return Some(cache_obj);
        }
    }

    if cache_obj.exists() {
        return Some(cache_obj);
    }

    // Fallback: search installed locations (flat filename for backwards compat)
    for var in &["LPP_HOME", "LPP_DIR"] {
        if let Ok(val) = std::env::var(var) {
            let lib_obj = PathBuf::from(val).join("lib").join(&filename);
            if lib_obj.exists() {
                return Some(lib_obj);
            }
        }
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            let candidates = [
                exe_dir.join(&filename),
                exe_dir.join(format!("lib/{}", filename)),
                exe_dir.join(format!("../lib/{}", filename)),
                exe_dir.join(format!("../../lib/{}", filename)),
            ];
            for c in &candidates {
                if c.exists() {
                    return Some(c.clone());
                }
            }
        }
    }

    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        let home_obj = PathBuf::from(&home).join(".lpp/lib").join(&filename);
        if home_obj.exists() {
            return Some(home_obj);
        }
    }

    None
}

/// Link using the host C compiler (cc / cl.exe) with optional -l flags for FFI
pub fn host_link_binary(obj_file: &Path, output_path: &Path, link_libs: &[String]) -> Result<(), String> {
    let cc = if cfg!(windows) { "cl.exe" } else { "cc" };
    let mut cmd = std::process::Command::new(cc);
    if cfg!(windows) {
        cmd.arg("/nologo")
            .arg(obj_file)
            .arg(format!("/Fe:{}", output_path.display()));
        for lib in link_libs {
            cmd.arg(format!("{}.lib", lib));
        }
    } else {
        cmd.arg(obj_file)
            .arg("-o")
            .arg(output_path)
            .arg("-lm"); // always link math
        for lib in link_libs {
            cmd.arg(format!("-l{}", lib));
        }
    }
    // Also link the host runtime
    let runtime_src_path = Path::new("lpp_runtime.c");
    if runtime_src_path.exists() {
        cmd.arg(runtime_src_path);
    }
    let status = cmd
        .stdin(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("Failed to execute host linker '{}': {}", cc, e))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("Host linker '{}' failed", cc))
    }
}

pub fn direct_link_binary(obj_file: &Path, output_path: &Path) -> Result<(), String> {
    let linker = current_binary_dir()
        .map(|dir| dir.join(format!("lpp-link{}", std::env::consts::EXE_SUFFIX)))
        .filter(|path| path.exists())
        .ok_or_else(|| {
            "Direct linker requested but lpp-link is not installed beside lpp.".to_string()
        })?;

    let runtime = resolve_min_runtime_object()
        .ok_or_else(|| {
            let ext = if cfg!(target_os = "windows") { "obj" } else { "o" };
            format!("Direct linker requested but lpp_runtime_min.{} is unavailable. Reinstall L++ or compile runtime source.", ext)
        })?;

    let mut cmd = std::process::Command::new(&linker);
    if cfg!(target_os = "windows") {
        cmd.arg("pe");
    } else if cfg!(target_os = "macos") {
        cmd.arg("macho");
    }
    cmd.arg(obj_file)
        .arg(&runtime)
        .arg("-o")
        .arg(output_path);

    let status = cmd
        .stdin(std::process::Stdio::null())
        .status()
        .map_err(|e| format!("Failed to execute lpp-link: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err("lpp-link failed while creating native executable.".to_string())
    }
}

fn link_native_binary(obj_file: &Path, output_path: &Path) -> Result<(), String> {
    direct_link_binary(obj_file, output_path)
}

pub fn run_command(args: &[String]) {
    if args.is_empty() {
        print_help();
        return;
    }

    match args[0].as_str() {
        "new" => cmd_new(&args[1..]),
        "init" => cmd_init(&args[1..]),
        "install" => cmd_install(false),
        "add" => cmd_add(&args[1..]),
        "remove" => cmd_remove(&args[1..]),
        "update" => cmd_update(),
        "search" => cmd_search(&args[1..]),
        "list" => cmd_list(),
        "tree" => cmd_tree(),
        "metadata" => cmd_metadata(),
        "outdated" => cmd_outdated(),
        "clean" => cmd_clean(),
        "check" => cmd_check(),
        "build" => {
            let _ = cmd_build();
        }
        "run" => cmd_run(),
        "test" => cmd_test(),
        "bench" => cmd_bench(),
        "help" => print_help(),
        cmd => {
            eprintln!("[L++] Unknown package manager command: '{}'", cmd);
            print_help();
        }
    }
}

fn print_help() {
    println!("L++ Package Manager (lpp-pm)");
    println!("Usage: lpp [command] [options]");
    println!("\nCommands:");
    println!("  new <project_name>    Create a new package directory with scaffold");
    println!("  init <project_name>   Initialize the current directory as a package");
    println!("  install               Resolve, download and install all dependencies");
    println!("  add <name>            Add dependency from online registry");
    println!("  add @owner/repo       Add dependency directly from GitHub repository");
    println!("  add <name> --git <U>  Add dependency via explicit git URL");
    println!("  add <name> --path <P> Add dependency via local folder path");
    println!("  add <name> --version <V> Record an expected dependency version");
    println!("  remove <name>         Remove a dependency from lpp.toml");
    println!("  update                Update all resolved dependencies");
    println!("  search <query>        Search packages from the local registry cache");
    println!("  list                  List direct dependencies from lpp.toml");
    println!("  tree                  Print lockfile dependency tree");
    println!("  metadata              Print package metadata");
    println!("  outdated              Show dependencies without pinned versions");
    println!("  clean                 Remove target/ and generated build artifacts");
    println!("  check                 Validate grammar, scope and types in project");
    println!("  build                 Build project into native target executable");
    println!("  run                   Compile and run the project native target");
    println!("  test                  Compile and execute all tests in tests/ folder");
    println!("  bench                 Run King20 benchmarks across all 3 linkers");
    println!("  help                  Show this help menu");
    println!("\nSource-file commands (outside package mode):");
    println!("  lpp check <file.lpp>          Check one source file without artifacts");
    println!("  lpp emit <file.lpp>           Emit Cranelift native object file (.o / .obj)");
    println!("  lpp emit <file.lpp> --aot     Emit Cranelift native object file (.o / .obj)");
    println!("\nBenchmark commands:");
    println!("  lpp bench --self-test         Run 15 integration tests");
    println!("  lpp bench --disk --mem --json King20 across all linkers with stats");
    println!("\nRule: `lpp build` builds an lpp.toml package; `lpp emit` handles one file.");
}

fn cmd_new(args: &[String]) {
    let raw_name = args.get(0).map(|s| s.as_str()).unwrap_or("my_project");
    let package_name = normalize_package_name(raw_name);
    let project_dir = PathBuf::from(raw_name);
    if project_dir.exists() {
        eprintln!(
            "[L++] Error: directory '{}' already exists.",
            project_dir.display()
        );
        return;
    }
    println!("[L++] Creating new project '{}'...", raw_name);
    if let Err(e) = fs::create_dir_all(&project_dir) {
        eprintln!("Failed to create project directory: {}", e);
        return;
    }
    match write_project_scaffold(&project_dir, &package_name) {
        Ok(()) => println!(
            "[L++] Project '{}' created at {}.",
            package_name,
            project_dir.display()
        ),
        Err(e) => eprintln!("{}", e),
    }
}

fn cmd_init(args: &[String]) {
    let project_name =
        normalize_package_name(args.get(0).map(|s| s.as_str()).unwrap_or("my_project"));
    println!("[L++] Initializing new project '{}'...", project_name);
    match write_project_scaffold(Path::new("."), &project_name) {
        Ok(()) => println!("[L++] Project '{}' initialized successfully!", project_name),
        Err(e) => eprintln!("{}", e),
    }
}

pub fn resolve_from_json(json_str: &str, target_name: &str) -> Option<RegistryEntry> {
    let quoted_target = format!("\"{}\"", target_name);
    if let Some(target_idx) = json_str.find(&quoted_target) {
        let rest = &json_str[target_idx + quoted_target.len()..];
        if let Some(colon_idx) = rest.find(':') {
            let block_rest = &rest[colon_idx + 1..];
            if let Some(open_brace) = block_rest.find('{') {
                let entry_content = &block_rest[open_brace + 1..];
                if let Some(close_brace) = entry_content.find('}') {
                    let entry_str = &entry_content[..close_brace];

                    let mut git = String::new();
                    let mut branch = None;
                    let mut tag = None;

                    for field_part in entry_str.split(',') {
                        if let Some(eq_idx) = field_part.find(':') {
                            let key = field_part[..eq_idx]
                                .trim()
                                .trim_matches('"')
                                .trim_matches('\'')
                                .trim();
                            let val = field_part[eq_idx + 1..]
                                .trim()
                                .trim_matches('"')
                                .trim_matches('\'')
                                .trim()
                                .to_string();
                            if key == "git" {
                                git = val;
                            } else if key == "branch" {
                                branch = Some(val);
                            } else if key == "tag" {
                                tag = Some(val);
                            }
                        }
                    }

                    if !git.is_empty() {
                        return Some(RegistryEntry { git, branch, tag });
                    }
                }
            }
        }
    }
    None
}

fn fetch_registry_json() -> Option<String> {
    let local_registry = Path::new("githubpage").join("registry.json");
    if local_registry.exists() {
        return fs::read_to_string(local_registry).ok();
    }

    let url = "https://raw.githubusercontent.com/samarnever-droid/Lpp-a-programing-langauge-/master/githubpage/registry.json";

    if command_available("curl", &["--version"]) {
        let output = std::process::Command::new("curl")
            .args(["-fsSL", url])
            .output()
            .ok()?;
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).into_owned());
        }
    }

    #[cfg(windows)]
    {
        let cmd_arg = format!("Invoke-RestMethod -Uri '{}' | ConvertTo-Json -Depth 5", url);
        let output = std::process::Command::new("powershell")
            .args(["-Command", &cmd_arg])
            .output()
            .ok()?;
        if output.status.success() {
            return Some(String::from_utf8_lossy(&output.stdout).into_owned());
        }
    }

    None
}

pub fn resolve_registry_package(name: &str) -> Option<RegistryEntry> {
    println!("[L++] Querying package registry for '{}'...", name);
    if let Some(json_str) = fetch_registry_json() {
        if let Some(entry) = resolve_from_json(&json_str, name) {
            return Some(entry);
        }
    }
    None
}

fn cmd_install(force_update: bool) {
    println!("[L++] Resolving dependencies...");
    if !std::path::Path::new("lpp.toml").exists() {
        eprintln!("[L++] Error: lpp.toml not found in the current directory.");
        return;
    }

    let content = match fs::read_to_string("lpp.toml") {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read lpp.toml: {}", e);
            return;
        }
    };

    let package = match parse_toml(&content) {
        Ok(pkg) => pkg,
        Err(e) => {
            eprintln!("[L++] TOML Parse error: {}", e);
            return;
        }
    };

    let pkg_dir = std::path::Path::new(".lpp_packages");
    if !pkg_dir.exists() {
        if let Err(e) = fs::create_dir_all(pkg_dir) {
            eprintln!("Failed to create .lpp_packages directory: {}", e);
            return;
        }
    }

    let mut lock_content = String::from("# Generated by L++ Package Manager. Do not edit.\n\n");

    for dep in &package.dependencies {
        println!("[L++] Installing '{}'...", dep.name);
        let dest_path = pkg_dir.join(&dep.name);

        let mut dep_git = dep.git.clone();
        let mut dep_branch = dep.branch.clone();
        let mut dep_tag = dep.tag.clone();

        if dep_git.is_none() && dep.path.is_none() {
            if let Some(entry) = resolve_registry_package(&dep.name) {
                println!(
                    "[L++] Resolved '{}' from registry -> {}",
                    dep.name, entry.git
                );
                dep_git = Some(entry.git);
                dep_branch = entry.branch;
                dep_tag = entry.tag;
            } else {
                eprintln!(
                    "[L++] Error: dependency '{}' has no source (git/path) and is not in the registry.",
                    dep.name
                );
                continue;
            }
        }

        if let Some(ref git_url) = dep_git {
            let mut git_checkout_needed = false;
            if dest_path.exists() {
                if force_update {
                    println!("  Updating '{}' from {}...", dep.name, git_url);
                    let status = std::process::Command::new("git")
                        .env("GIT_TERMINAL_PROMPT", "0")
                        .args(&[
                            "-c",
                            "credential.helper=",
                            "-C",
                            dest_path.to_str().unwrap(),
                            "pull",
                        ])
                        .status();
                    match status {
                        Ok(s) if s.success() => {
                            git_checkout_needed = true;
                        }
                        _ => {
                            eprintln!("  Failed to pull updates for '{}'. skipping.", dep.name);
                            continue;
                        }
                    }
                } else {
                    println!("  Dependency '{}' already installed.", dep.name);
                }
            } else {
                println!("  Cloning '{}' from {}...", dep.name, git_url);
                let status = std::process::Command::new("git")
                    .env("GIT_TERMINAL_PROMPT", "0")
                    .args(&[
                        "-c",
                        "credential.helper=",
                        "clone",
                        git_url,
                        dest_path.to_str().unwrap(),
                    ])
                    .status();
                match status {
                    Ok(s) if s.success() => {
                        git_checkout_needed = true;
                    }
                    _ => {
                        eprintln!("  Failed to clone '{}'. skipping.", dep.name);
                        continue;
                    }
                }
            }

            if git_checkout_needed {
                if let Some(ref tag) = dep_tag {
                    println!("  Checking out tag '{}'...", tag);
                    let _ = std::process::Command::new("git")
                        .env("GIT_TERMINAL_PROMPT", "0")
                        .args(&[
                            "-c",
                            "credential.helper=",
                            "-C",
                            dest_path.to_str().unwrap(),
                            "checkout",
                            tag,
                        ])
                        .status();
                } else if let Some(ref branch) = dep_branch {
                    println!("  Checking out branch '{}'...", branch);
                    let _ = std::process::Command::new("git")
                        .env("GIT_TERMINAL_PROMPT", "0")
                        .args(&[
                            "-c",
                            "credential.helper=",
                            "-C",
                            dest_path.to_str().unwrap(),
                            "checkout",
                            branch,
                        ])
                        .status();
                }
            }

            let commit_output = std::process::Command::new("git")
                .env("GIT_TERMINAL_PROMPT", "0")
                .args(&[
                    "-c",
                    "credential.helper=",
                    "-C",
                    dest_path.to_str().unwrap(),
                    "rev-parse",
                    "HEAD",
                ])
                .output();
            let commit_hash = if let Ok(out) = commit_output {
                if out.status.success() {
                    String::from_utf8_lossy(&out.stdout).trim().to_string()
                } else {
                    "unknown".to_string()
                }
            } else {
                "unknown".to_string()
            };

            lock_content.push_str(&format!(
                "[[package]]\nname = \"{}\"\nversion = \"{}\"\nsource = \"git+{}#{}\"\nresolved = \"{}\"\n\n",
                dep.name,
                dep.version.clone().unwrap_or_else(|| "unbounded".to_string()),
                git_url,
                commit_hash,
                dest_path.display()
            ));
        } else if let Some(ref path) = dep.path {
            println!("  Linked path: {}", path);
            let path_ref = std::path::Path::new(path);
            if !path_ref.exists() {
                eprintln!(
                    "  [L++] Error: path '{}' for dependency '{}' does not exist.",
                    path, dep.name
                );
                continue;
            }

            lock_content.push_str(&format!(
                "[[package]]\nname = \"{}\"\nversion = \"{}\"\nsource = \"path+{}\"\nresolved = \"{}\"\n\n",
                dep.name,
                dep.version.clone().unwrap_or_else(|| "workspace".to_string()),
                path,
                path_ref.display()
            ));
        }
    }

    if let Err(e) = fs::write("lpp.lock", lock_content) {
        eprintln!("Failed to write lpp.lock: {}", e);
    } else {
        println!("[L++] lpp.lock file generated.");
    }

    println!("[L++] Dependencies resolved successfully.");
}

fn cmd_add(args: &[String]) {
    if args.is_empty() {
        eprintln!(
            "Usage: lpp add <package_name> [--git <url> [--tag <tag>] [--branch <branch>]] [--path <local_path>] [--version <semver>]"
        );
        return;
    }

    let mut package_name = args[0].clone();
    let mut git_url = None;
    let mut tag = None;
    let mut branch = None;
    let mut path = None;
    let mut version = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--git" => {
                if i + 1 < args.len() {
                    git_url = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --git expects a URL argument");
                    return;
                }
            }
            "--tag" => {
                if i + 1 < args.len() {
                    tag = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --tag expects a tag name argument");
                    return;
                }
            }
            "--branch" => {
                if i + 1 < args.len() {
                    branch = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --branch expects a branch name argument");
                    return;
                }
            }
            "--version" => {
                if i + 1 < args.len() {
                    version = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --version expects a version string argument");
                    return;
                }
            }
            "--path" => {
                if i + 1 < args.len() {
                    path = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --path expects a directory path argument");
                    return;
                }
            }
            _ => {
                eprintln!("Unknown argument: {}", args[i]);
                return;
            }
        }
    }

    if git_url.is_none() && path.is_none() {
        // Auto-resolve @owner/repo → https://github.com/owner/repo.git
        if package_name.starts_with('@') {
            if let Some(slash_idx) = package_name.find('/') {
                let owner = &package_name[1..slash_idx];
                let repo = &package_name[slash_idx + 1..];
                let url = format!("https://github.com/{}/{}.git", owner, repo);
                println!("[L++] Auto-resolved @{}/{} → {}", owner, repo, url);
                git_url = Some(url);
                branch = Some("master".to_string());
                package_name = repo.to_string();
            }
        }

        if git_url.is_none() {
            if let Some(entry) = resolve_registry_package(&package_name) {
                println!("[L++] Resolved '{}' from registry:", package_name);
                println!("  Git: {}", entry.git);
                if let Some(ref b) = entry.branch {
                    println!("  Branch: {}", b);
                }
                if let Some(ref t) = entry.tag {
                    println!("  Tag: {}", t);
                }
                git_url = Some(entry.git);
                branch = entry.branch;
                tag = entry.tag;

                if package_name.starts_with('@') {
                    if let Some(slash_idx) = package_name.find('/') {
                        package_name = package_name[slash_idx + 1..].to_string();
                    }
                }
            } else {
                eprintln!(
                    "Error: Package '{}' not found in registry. Use --git <url> or @owner/repo format.",
                    package_name
                );
                return;
            }
        }
    }

    if !std::path::Path::new("lpp.toml").exists() {
        eprintln!("Error: lpp.toml not found. Run 'lpp init' first.");
        return;
    }

    let mut content = match fs::read_to_string("lpp.toml") {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read lpp.toml: {}", e);
            return;
        }
    };

    let mut dep_line = format!("\n{} = {{ ", package_name);
    if let Some(ref url) = git_url {
        dep_line.push_str(&format!("git = \"{}\"", url));
        if let Some(ref v) = version {
            dep_line.push_str(&format!(", version = \"{}\"", v));
        }
        if let Some(ref t) = tag {
            dep_line.push_str(&format!(", tag = \"{}\"", t));
        } else if let Some(ref b) = branch {
            dep_line.push_str(&format!(", branch = \"{}\"", b));
        }
    } else if let Some(ref p) = path {
        dep_line.push_str(&format!("path = \"{}\"", p));
        if let Some(ref v) = version {
            dep_line.push_str(&format!(", version = \"{}\"", v));
        }
    }
    dep_line.push_str(" }\n");

    content.push_str(&dep_line);

    if let Err(e) = fs::write("lpp.toml", content) {
        eprintln!("Failed to update lpp.toml: {}", e);
        return;
    }

    println!("[L++] Added dependency '{}' to lpp.toml.", package_name);
    cmd_install(false);
}

#[cfg(test)]
mod tests {
    use super::parse_toml;

    #[test]
    fn parse_toml_requires_package_version() {
        let manifest = "[package]\nname = \"demo\"\n\n[dependencies]\n";
        let err = parse_toml(manifest).expect_err("manifest without version should fail");
        assert!(err.contains("version"));
    }

    #[test]
    fn parse_toml_reads_dependency_version() {
        let manifest = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n\n[dependencies]\nfoo = { git = \"https://example.com/foo.git\", version = \"1.2.3\" }\n";
        let pkg = parse_toml(manifest).expect("manifest should parse");
        assert_eq!(pkg.dependencies.len(), 1);
        assert_eq!(pkg.dependencies[0].version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn parse_lockfile_reads_version_and_source() {
        let lock = "[[package]]\nname = \"foo\"\nversion = \"1.2.3\"\nsource = \"git+https://example.com/foo.git#abc\"\nresolved = \"C:/tmp/foo\"\n";
        let pkgs = super::parse_lockfile(lock);
        assert_eq!(pkgs.len(), 1);
        assert_eq!(pkgs[0].name, "foo");
        assert_eq!(pkgs[0].version.as_deref(), Some("1.2.3"));
        assert!(pkgs[0].source.contains("git+https://example.com/foo.git"));
        assert_eq!(pkgs[0].resolved.as_deref(), Some("C:/tmp/foo"));
    }

    #[test]
    fn should_use_mold_returns_false_for_msvc() {
        assert_eq!(super::should_use_mold("cl.exe").unwrap(), false);
    }

    #[test]
    fn should_use_mold_checks_availability() {
        let result = super::should_use_mold("gcc");
        assert!(result.is_ok());
    }
}

fn cmd_remove(args: &[String]) {
    if args.is_empty() {
        eprintln!("Usage: lpp remove <package_name>");
        return;
    }
    let package_name = &args[0];
    if !std::path::Path::new("lpp.toml").exists() {
        eprintln!("Error: lpp.toml not found.");
        return;
    }
    let content = match fs::read_to_string("lpp.toml") {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read lpp.toml: {}", e);
            return;
        }
    };

    let mut new_lines = Vec::new();
    let mut found = false;
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with(&format!("{} =", package_name))
            || trimmed.starts_with(&format!("{}=", package_name))
        {
            found = true;
            continue;
        }
        new_lines.push(line);
    }

    if !found {
        println!("[L++] Dependency '{}' not found in lpp.toml.", package_name);
        return;
    }

    if let Err(e) = fs::write("lpp.toml", new_lines.join("\n")) {
        eprintln!("Failed to update lpp.toml: {}", e);
        return;
    }
    println!("[L++] Removed dependency '{}' from lpp.toml.", package_name);

    let dest_path = std::path::Path::new(".lpp_packages").join(package_name);
    if dest_path.exists() {
        let _ = fs::remove_dir_all(dest_path);
        println!("[L++] Cleaned up package directory for '{}'.", package_name);
    }

    cmd_install(false);
}

fn cmd_update() {
    println!("[L++] Updating lockfile and pulling latest dependency updates...");
    cmd_install(true);
}

fn cmd_search(args: &[String]) {
    let query = args.get(0).map(|s| s.to_lowercase()).unwrap_or_default();
    let mut results = registry_package_names();
    results.sort();
    if !query.is_empty() {
        results.retain(|name| name.to_lowercase().contains(&query));
    }
    if results.is_empty() {
        println!("[L++] No registry packages matched '{}'.", query);
        return;
    }
    println!("[L++] Registry matches:");
    for name in results {
        println!("  {}", name);
    }
}

fn cmd_list() {
    match read_manifest() {
        Ok(pkg) => {
            println!("[L++] Package: {} {}", pkg.name, pkg.version);
            if pkg.dependencies.is_empty() {
                println!("  (no dependencies)");
                return;
            }
            for dep in pkg.dependencies {
                let source = dep
                    .path
                    .or(dep.git)
                    .unwrap_or_else(|| "registry".to_string());
                let version = dep.version.unwrap_or_else(|| "unbounded".to_string());
                println!("  {} {} [{}]", dep.name, version, source);
            }
        }
        Err(e) => eprintln!("[L++] {}", e),
    }
}

fn cmd_tree() {
    let packages = read_lockfile();
    if packages.is_empty() {
        println!("[L++] No lockfile packages found. Run `lpp install` first.");
        return;
    }
    println!("[L++] Dependency tree:");
    for pkg in packages {
        let version = pkg.version.unwrap_or_else(|| "unknown".to_string());
        println!("  {} {}", pkg.name, version);
        println!("    source: {}", pkg.source);
        if let Some(resolved) = pkg.resolved {
            println!("    resolved: {}", resolved);
        }
    }
}

fn cmd_metadata() {
    match read_manifest() {
        Ok(pkg) => {
            println!("name = {}", pkg.name);
            println!("version = {}", pkg.version);
            if let Some(author) = pkg.author {
                println!("author = {}", author);
            }
            println!("entry = {}", pkg.entry.unwrap_or_else(resolve_entry_point));
            println!("dependencies = {}", pkg.dependencies.len());
            println!("locked_packages = {}", read_lockfile().len());
        }
        Err(e) => eprintln!("[L++] {}", e),
    }
}

fn cmd_outdated() {
    match read_manifest() {
        Ok(pkg) => {
            let mut found = false;
            for dep in pkg.dependencies {
                if dep.version.is_none() {
                    found = true;
                    println!("{} is not version-pinned", dep.name);
                }
            }
            if !found {
                println!("[L++] All direct dependencies are version-pinned.");
            }
        }
        Err(e) => eprintln!("[L++] {}", e),
    }
}

fn cmd_clean() {
    let mut removed = 0;
    for target in ["target", "output.c", "output.obj", "output.o"] {
        let path = Path::new(target);
        if path.is_dir() {
            if fs::remove_dir_all(path).is_ok() {
                removed += 1;
            }
        } else if path.is_file() && fs::remove_file(path).is_ok() {
            removed += 1;
        }
    }
    if let Ok(entries) = fs::read_dir(".") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path
                .extension()
                .map(|ext| ext == "exe" || ext == "o" || ext == "obj")
                .unwrap_or(false)
            {
                if fs::remove_file(&path).is_ok() {
                    removed += 1;
                }
            }
        }
    }
    println!("[L++] Cleaned {} generated artifact(s).", removed);
}

fn cmd_check() {
    println!("[L++] Checking project...");
    let entry_point_str = resolve_entry_point();
    let entry_point = Path::new(&entry_point_str);
    if !entry_point.exists() {
        eprintln!(
            "[L++] Error: entry point '{}' not found.",
            entry_point.display()
        );
        return;
    }

    let compiler_path = match current_compiler_path() {
        Ok(path) => path,
        Err(e) => {
            eprintln!("[L++] {}", e);
            return;
        }
    };

    match std::process::Command::new(&compiler_path)
        .arg(entry_point)
        .arg("--check")
        .status()
    {
        Ok(s) if s.success() => {
            println!("[L++] Project is semantically valid.");
        }
        Ok(_) => {
            eprintln!("[L++] Error: Project check failed.");
        }
        Err(e) => {
            eprintln!(
                "[L++] Error: failed to execute compiler '{}': {}",
                compiler_path.display(),
                e
            );
        }
    }
}

#[cfg(windows)]
pub fn load_msvc_env() {
    if std::process::Command::new("cl.exe")
        .arg("/?")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok()
    {
        return;
    }

    let mut vcvars = std::path::PathBuf::new();
    let fallbacks = [
        "C:\\Program Files\\Microsoft Visual Studio\\2022\\Community\\VC\\Auxiliary\\Build\\vcvars64.bat",
        "C:\\Program Files\\Microsoft Visual Studio\\2022\\Professional\\VC\\Auxiliary\\Build\\vcvars64.bat",
        "C:\\Program Files\\Microsoft Visual Studio\\2022\\Enterprise\\VC\\Auxiliary\\Build\\vcvars64.bat",
        "C:\\Program Files\\Microsoft Visual Studio\\2019\\Community\\VC\\Auxiliary\\Build\\vcvars64.bat",
        "C:\\Program Files\\Microsoft Visual Studio\\2019\\Professional\\VC\\Auxiliary\\Build\\vcvars64.bat",
        "C:\\Program Files\\Microsoft Visual Studio\\2019\\Enterprise\\VC\\Auxiliary\\Build\\vcvars64.bat",
    ];

    for fallback in &fallbacks {
        let p = std::path::Path::new(fallback);
        if p.exists() {
            vcvars = p.to_path_buf();
            break;
        }
    }

    if vcvars.exists() {
        println!("  Loading MSVC environment via: {}", vcvars.display());
        let temp_dir = std::env::temp_dir();
        let bat_path = temp_dir.join("lpp_vcvars.bat");
        let bat_content = format!(
            "@echo off\ncall \"{}\" > nul\nset\n",
            vcvars.to_str().unwrap()
        );
        let output = if fs::write(&bat_path, bat_content).is_ok() {
            let res = std::process::Command::new("cmd.exe")
                .args(&["/c", bat_path.to_str().unwrap()])
                .output();
            let _ = fs::remove_file(&bat_path);
            res
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to write temp batch file",
            ))
        };

        match output {
            Ok(out) if out.status.success() => {
                let env_dump = String::from_utf8_lossy(&out.stdout);
                let mut loaded_count = 0;
                for line in env_dump.lines() {
                    if let Some(eq_idx) = line.find('=') {
                        let name = &line[..eq_idx];
                        let val = &line[eq_idx + 1..];
                        unsafe {
                            std::env::set_var(name, val);
                        }
                        loaded_count += 1;
                    }
                }
                println!("  Loaded {} environment variables from MSVC.", loaded_count);
            }
            Ok(out) => {
                eprintln!("  vcvars64.bat exited with error status: {:?}", out.status);
                eprintln!("  Stderr: {}", String::from_utf8_lossy(&out.stderr));
            }
            Err(e) => {
                eprintln!("  Failed to run cmd.exe for vcvars64.bat: {}", e);
            }
        }
    } else {
        println!("  Could not find vcvars64.bat at standard locations.");
    }
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn load_msvc_env() {}

fn cmd_build() -> Option<String> {
    println!("[L++] Building project...");
    let entry_point_str = resolve_entry_point();
    let entry_point = Path::new(&entry_point_str);
    if !entry_point.exists() {
        eprintln!(
            "[L++] Error: entry point '{}' not found.",
            entry_point.display()
        );
        return None;
    }

    cmd_install(false);

    let target_dir = Path::new("LppData").join("build").join("release");
    let _ = fs::create_dir_all(&target_dir);

    println!("  Compiling {}...", entry_point.display());
    let obj_file = match compile_source_to_object(entry_point) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("[L++] Error: {}", e);
            return None;
        }
    };

    let mut bin_name = "output".to_string();
    if Path::new("lpp.toml").exists() {
        if let Ok(content) = fs::read_to_string("lpp.toml") {
            if let Ok(pkg) = parse_toml(&content) {
                bin_name = pkg.name;
            }
        }
    }

    let exe_path = output_path_for_name(&target_dir, &bin_name);

    println!("  Linking {}...", exe_path.display());
    let link_result = link_native_binary(&obj_file, &exe_path);
    let _ = fs::remove_file(&obj_file);

    if let Err(e) = link_result {
        eprintln!("[L++] Error: {}", e);
        None
    } else {
        println!("[L++] Build successful: {}", exe_path.display());
        Some(exe_path.to_string_lossy().into_owned())
    }
}

fn cmd_run() {
    if let Some(exe_path) = cmd_build() {
        println!("[L++] Running {}...", exe_path);
        let status = std::process::Command::new(&exe_path).status();
        if let Err(e) = status {
            eprintln!("[L++] Failed to execute target: {}", e);
        }
    }
}

fn cmd_bench() {
    println!("[L++] Launching lpp-bench...");
    let bench_bin = current_binary_dir()
        .map(|dir| dir.join(format!("lpp-bench{}", std::env::consts::EXE_SUFFIX)))
        .filter(|p| p.exists());
    if let Some(bench) = bench_bin {
        let args: Vec<String> = std::env::args().skip(2).collect();
        let status = std::process::Command::new(&bench).args(&args).status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => std::process::exit(s.code().unwrap_or(1)),
            Err(e) => {
                eprintln!("[L++] Failed to launch lpp-bench: {e}");
                std::process::exit(1);
            }
        }
    } else {
        eprintln!(
            "[L++] lpp-bench not found. Build it with: cargo build --release --bin lpp-bench"
        );
        std::process::exit(1);
    }
}

fn cmd_test() {
    println!("[L++] Running tests...");
    let test_dir = if Path::new("tests").exists() {
        "tests"
    } else if Path::new("test").exists() {
        "test"
    } else {
        println!("[L++] No tests/ or test/ directory found.");
        return;
    };

    let paths = match fs::read_dir(test_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to read tests directory: {}", e);
            return;
        }
    };

    let mut test_files = Vec::new();
    for entry in paths {
        if let Ok(entry) = entry {
            let path = entry.path();
            if path.is_file() && path.extension().map_or(false, |ext| ext == "lpp") {
                test_files.push(path);
            }
        }
    }

    if test_files.is_empty() {
        println!("[L++] No test files found in directory '{}'.", test_dir);
        return;
    }

    let mut passed = 0;
    let mut failed = 0;

    let target_test_dir = Path::new("target").join("test");
    let _ = fs::create_dir_all(&target_test_dir);

    for test_path in test_files {
        let test_name = test_path.file_name().unwrap().to_str().unwrap();
        print!("  test {} ... ", test_name);

        let base_name = format!("test_{}", test_name.replace(".lpp", ""));
        let temp_exe = output_path_for_name(&target_test_dir, &base_name);

        match compile_source_to_object(&test_path) {
            Ok(temp_obj) => {
                let link_result = link_native_binary(&temp_obj, &temp_exe);
                let _ = fs::remove_file(&temp_obj);

                if link_result.is_ok() && temp_exe.exists() {
                    let run_output = std::process::Command::new(&temp_exe).output();
                    let _ = fs::remove_file(&temp_exe);

                    match run_output {
                        Ok(out) if out.status.success() => {
                            println!("ok");
                            passed += 1;
                        }
                        _ => {
                            println!("FAILED (execution error)");
                            failed += 1;
                        }
                    }
                } else {
                    println!("FAILED (linking failed)");
                    failed += 1;
                }
            }
            Err(_) => {
                println!("FAILED (compilation failed)");
                failed += 1;
            }
        }
    }

    println!(
        "\ntest result: {}. {} passed; {} failed",
        if failed == 0 { "ok" } else { "FAILED" },
        passed,
        failed
    );
}
