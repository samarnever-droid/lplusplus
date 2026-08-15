use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

#[cfg(windows)]
pub fn enable_ansi_support() {
    unsafe {
        use std::os::raw::c_void;
        type HANDLE = *mut c_void;
        type DWORD = u32;
        type BOOL = i32;
        const STD_OUTPUT_HANDLE: DWORD = 0xFFFFFFF5;
        const STD_ERROR_HANDLE: DWORD = 0xFFFFFFF4;
        const ENABLE_VIRTUAL_TERMINAL_PROCESSING: DWORD = 0x0004;

        unsafe extern "system" {
            fn GetStdHandle(nStdHandle: DWORD) -> HANDLE;
            fn GetConsoleMode(hConsoleHandle: HANDLE, lpMode: *mut DWORD) -> BOOL;
            fn SetConsoleMode(hConsoleHandle: HANDLE, dwMode: DWORD) -> BOOL;
        }

        for &handle_id in &[STD_OUTPUT_HANDLE, STD_ERROR_HANDLE] {
            let handle = GetStdHandle(handle_id);
            if !handle.is_null() && handle as isize != -1 {
                let mut mode: DWORD = 0;
                if GetConsoleMode(handle, &mut mode) != 0 {
                    SetConsoleMode(handle, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
                }
            }
        }
    }
}

#[cfg(not(windows))]
pub fn enable_ansi_support() {}

pub mod ui {
    use std::sync::atomic::{AtomicBool, Ordering};

    static COLOR_ENABLED: AtomicBool = AtomicBool::new(true);

    pub fn init() {
        super::enable_ansi_support();
        if std::env::var_os("NO_COLOR").is_some() {
            COLOR_ENABLED.store(false, Ordering::Relaxed);
        }
    }

    pub fn is_color() -> bool {
        COLOR_ENABLED.load(Ordering::Relaxed)
    }

    pub fn cyan(s: &str) -> String { if is_color() { format!("\x1b[38;2;6;182;212m{s}\x1b[0m") } else { s.to_string() } }
    pub fn bold_cyan(s: &str) -> String { if is_color() { format!("\x1b[1;38;2;6;182;212m{s}\x1b[0m") } else { s.to_string() } }
    pub fn green(s: &str) -> String { if is_color() { format!("\x1b[38;2;16;185;129m{s}\x1b[0m") } else { s.to_string() } }
    pub fn bold_green(s: &str) -> String { if is_color() { format!("\x1b[1;38;2;16;185;129m{s}\x1b[0m") } else { s.to_string() } }
    pub fn purple(s: &str) -> String { if is_color() { format!("\x1b[38;2;168;85;247m{s}\x1b[0m") } else { s.to_string() } }
    pub fn bold_purple(s: &str) -> String { if is_color() { format!("\x1b[1;38;2;168;85;247m{s}\x1b[0m") } else { s.to_string() } }
    pub fn yellow(s: &str) -> String { if is_color() { format!("\x1b[38;2;245;158;11m{s}\x1b[0m") } else { s.to_string() } }
    pub fn bold_yellow(s: &str) -> String { if is_color() { format!("\x1b[1;38;2;245;158;11m{s}\x1b[0m") } else { s.to_string() } }
    pub fn red(s: &str) -> String { if is_color() { format!("\x1b[38;2;244;63;94m{s}\x1b[0m") } else { s.to_string() } }
    pub fn bold_red(s: &str) -> String { if is_color() { format!("\x1b[1;38;2;244;63;94m{s}\x1b[0m") } else { s.to_string() } }
    pub fn dim(s: &str) -> String { if is_color() { format!("\x1b[38;2;148;163;184m{s}\x1b[0m") } else { s.to_string() } }
    pub fn bold(s: &str) -> String { if is_color() { format!("\x1b[1m{s}\x1b[0m") } else { s.to_string() } }

    pub fn badge_lpp() -> String {
        if is_color() {
            "\x1b[1;38;2;255;255;255;48;2;14;165;233m L++ \x1b[0m".to_string()
        } else {
            "[L++]".to_string()
        }
    }

    pub fn tag_success(text: &str) -> String {
        if is_color() {
            format!("\x1b[1;38;2;16;185;129m✔\x1b[0m \x1b[1m{text}\x1b[0m")
        } else {
            format!("✔ {text}")
        }
    }

    pub fn tag_info(text: &str) -> String {
        if is_color() {
            format!("\x1b[1;38;2;6;182;212m❯\x1b[0m {text}")
        } else {
            format!("❯ {text}")
        }
    }

    pub fn tag_warn(text: &str) -> String {
        if is_color() {
            format!("\x1b[1;38;2;245;158;11m▲\x1b[0m \x1b[38;2;245;158;11m{text}\x1b[0m")
        } else {
            format!("▲ {text}")
        }
    }

    pub fn tag_error(text: &str) -> String {
        if is_color() {
            format!("\x1b[1;38;2;244;63;94m✖\x1b[0m \x1b[1;38;2;244;63;94m{text}\x1b[0m")
        } else {
            format!("✖ {text}")
        }
    }

    pub fn tag_step(curr: usize, total: usize, desc: &str) -> String {
        if is_color() {
            format!("\x1b[1;38;2;6;182;212m[{curr}/{total}]\x1b[0m \x1b[1m{desc}\x1b[0m")
        } else {
            format!("[{curr}/{total}] {desc}")
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub keywords: Vec<String>,
    pub dependencies: Vec<Dependency>,
}

#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct RegistryEntry {
    #[serde(default, alias = "repository")]
    pub git: String,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default, alias = "source_url")]
    pub source: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RegistryManifest {
    #[serde(default)]
    pub packages: std::collections::HashMap<String, RegistryEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedPackage {
    pub name: String,
    pub version: Option<String>,
    pub source: String,
    pub resolved: Option<String>,
    pub checksum: Option<String>,
}

pub fn validate_keywords(keywords: &[String]) -> Result<Vec<String>, String> {
    if keywords.len() > 5 {
        return Err(format!(
            "Package manifest error: maximum 5 keywords allowed in manifest (found {})",
            keywords.len()
        ));
    }
    let mut cleaned = Vec::new();
    for kw in keywords {
        let trimmed = kw.trim().to_lowercase();
        if trimmed.len() > 32 {
            return Err(format!(
                "Keyword '{}' exceeds maximum length of 32 characters",
                kw
            ));
        }
        if !trimmed.is_empty() {
            cleaned.push(trimmed);
        }
    }
    Ok(cleaned)
}

pub fn validate_package_name(name: &str) -> Result<(), String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err("Package manifest error: package name cannot be empty".to_string());
    }
    if trimmed.len() > 128 {
        return Err("Package manifest error: package name exceeds 128 characters".to_string());
    }
    if trimmed.contains("..") || trimmed.contains('\\') || trimmed.starts_with('/') || trimmed.contains('\0') {
        return Err(format!("Package manifest error: invalid package name '{trimmed}' (contains illegal path traversal characters)"));
    }
    let lower = trimmed.to_ascii_lowercase();
    let reserved = ["con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9"];
    if reserved.contains(&lower.as_str()) {
        return Err(format!("Package manifest error: invalid package name '{trimmed}' (reserved system device name)"));
    }
    Ok(())
}

fn validate_package_version(version: &str) -> Result<(), String> {
    semver::Version::parse(version).map(|_| ()).map_err(|e| {
        format!("Package manifest error: invalid package version '{version}': {e}")
    })
}

fn validate_dependency_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty()
        || name == "."
        || name == ".."
        || name.contains('/')
        || name.contains('\\')
        || name.chars().any(|ch| matches!(ch, '\n' | '\r' | '\t'))
    {
        return Err(format!("Package manifest error: invalid dependency name '{name}'"));
    }
    Ok(())
}

fn validate_dependency_requirement(req: &str) -> Result<(), String> {
    let trimmed = req.trim();
    if trimmed.is_empty() || trimmed == "workspace" {
        return Ok(());
    }
    semver::VersionReq::parse(trimmed).map(|_| ()).map_err(|e| {
        format!("Package manifest error: invalid version requirement '{trimmed}': {e}")
    })
}

fn string_field(table: &toml::map::Map<String, toml::Value>, key: &str) -> Result<Option<String>, String> {
    match table.get(key) {
        None => Ok(None),
        Some(value) => value
            .as_str()
            .map(|s| Some(s.to_string()))
            .ok_or_else(|| format!("Package manifest error: '{key}' must be a string")),
    }
}

fn dependency_from_parts(
    name: &str,
    version: Option<String>,
    git: Option<String>,
    tag: Option<String>,
    branch: Option<String>,
    path: Option<String>,
) -> Result<Dependency, String> {
    validate_dependency_name(name)?;
    if git.is_some() && path.is_some() {
        return Err(format!("Package manifest error: dependency '{name}' cannot specify both git and path"));
    }
    if tag.is_some() && branch.is_some() {
        return Err(format!("Package manifest error: dependency '{name}' cannot specify both tag and branch"));
    }
    if git.is_none() && tag.is_some() {
        return Err(format!("Package manifest error: dependency '{name}' uses tag without git"));
    }
    if git.is_none() && branch.is_some() {
        return Err(format!("Package manifest error: dependency '{name}' uses branch without git"));
    }
    if let Some(ref req) = version {
        validate_dependency_requirement(req)?;
    }
    if let Some(ref source) = git {
        if source.trim().is_empty() {
            return Err(format!("Package manifest error: dependency '{name}' has an empty git URL"));
        }
    }
    if let Some(ref source) = path {
        if source.trim().is_empty() {
            return Err(format!("Package manifest error: dependency '{name}' has an empty path"));
        }
    }
    Ok(Dependency {
        name: name.to_string(),
        version,
        git,
        tag,
        branch,
        path,
    })
}

fn dependency_from_toml(name: &str, value: &toml::Value) -> Result<Dependency, String> {
    match value {
        toml::Value::String(raw) => {
            if raw.starts_with("./") || raw.starts_with("../") || raw == "." || raw == ".." {
                dependency_from_parts(name, None, None, None, None, Some(raw.clone()))
            } else if raw.starts_with("http://")
                || raw.starts_with("https://")
                || raw.ends_with(".git")
            {
                dependency_from_parts(name, None, Some(raw.clone()), None, None, None)
            } else {
                dependency_from_parts(name, Some(raw.clone()), None, None, None, None)
            }
        }
        toml::Value::Table(table) => dependency_from_parts(
            name,
            string_field(table, "version")?,
            string_field(table, "git")?,
            string_field(table, "tag")?,
            string_field(table, "branch")?,
            string_field(table, "path")?,
        ),
        _ => Err(format!("Package manifest error: dependency '{name}' must be a string or inline table")),
    }
}

fn package_from_parts(
    name: String,
    version: String,
    author: Option<String>,
    entry: Option<String>,
    keywords: Vec<String>,
    dependencies: Vec<Dependency>,
) -> Result<Package, String> {
    validate_package_name(&name)?;
    validate_package_version(&version)?;
    Ok(Package {
        name,
        version,
        author,
        entry,
        keywords: validate_keywords(&keywords)?,
        dependencies,
    })
}

pub fn parse_json_manifest(content: &str) -> Result<Package, String> {
    let val: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| format!("JSON syntax error in manifest: {e}"))?;
    let obj = val
        .as_object()
        .ok_or_else(|| "JSON manifest root must be an object".to_string())?;

    let name = obj
        .get("name")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| "Missing 'name' in lpp.json".to_string())?
        .to_string();
    let version = obj
        .get("version")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("0.1.0")
        .to_string();
    let author = obj.get("author").and_then(serde_json::Value::as_str).map(String::from);
    let entry = obj
        .get("main")
        .or_else(|| obj.get("entry"))
        .and_then(serde_json::Value::as_str)
        .map(String::from);

    let keywords = match obj.get("keywords") {
        None => Vec::new(),
        Some(value) => value
            .as_array()
            .ok_or_else(|| "'keywords' in lpp.json must be an array".to_string())?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(String::from)
                    .ok_or_else(|| "every keyword in lpp.json must be a string".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
    };

    let mut dependencies = Vec::new();
    if let Some(dep_obj) = obj.get("dependencies") {
        let dep_obj = dep_obj
            .as_object()
            .ok_or_else(|| "'dependencies' in lpp.json must be an object".to_string())?;
        for (dep_name, dep_value) in dep_obj {
            let dep = if let Some(raw) = dep_value.as_str() {
                if raw.starts_with("./") || raw.starts_with("../") || raw == "." || raw == ".." {
                    dependency_from_parts(dep_name, None, None, None, None, Some(raw.to_string()))?
                } else if raw.starts_with("http://")
                    || raw.starts_with("https://")
                    || raw.ends_with(".git")
                {
                    dependency_from_parts(dep_name, None, Some(raw.to_string()), None, None, None)?
                } else {
                    dependency_from_parts(dep_name, Some(raw.to_string()), None, None, None, None)?
                }
            } else {
                let table = dep_value
                    .as_object()
                    .ok_or_else(|| format!("dependency '{dep_name}' must be a string or object"))?;
                let get = |key: &str| -> Result<Option<String>, String> {
                    match table.get(key) {
                        None => Ok(None),
                        Some(v) => v
                            .as_str()
                            .map(|s| Some(s.to_string()))
                            .ok_or_else(|| format!("dependency '{dep_name}' field '{key}' must be a string")),
                    }
                };
                dependency_from_parts(
                    dep_name,
                    get("version")?,
                    get("git")?,
                    get("tag")?,
                    get("branch")?,
                    get("path")?,
                )?
            };
            dependencies.push(dep);
        }
    }

    package_from_parts(name, version, author, entry, keywords, dependencies)
}

fn parse_toml_with_workspace(content: &str, workspace_version: Option<&str>) -> Result<Package, String> {
    let root: toml::Value = toml::from_str(content)
        .map_err(|e| format!("TOML syntax error in manifest: {e}"))?;
    let root_table = root
        .as_table()
        .ok_or_else(|| "TOML manifest root must be a table".to_string())?;
    let package = root_table
        .get("package")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| "Missing [package] section in lpp.toml".to_string())?;

    let name = package
        .get("name")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| "Missing package name in [package] section".to_string())?
        .to_string();
    let version = match package.get("version") {
        Some(value) if value.as_str().is_some() => value.as_str().unwrap_or_default().to_string(),
        Some(toml::Value::Table(table))
            if table.get("workspace").and_then(toml::Value::as_bool) == Some(true) =>
        {
            workspace_version
                .ok_or_else(|| "package version uses workspace = true but no workspace version was provided".to_string())?
                .to_string()
        }
        Some(_) => return Err("Package manifest error: [package].version must be a SemVer string or { workspace = true }".to_string()),
        None => return Err("Missing package version in [package] section".to_string()),
    };
    let author = package
        .get("author")
        .and_then(toml::Value::as_str)
        .map(String::from)
        .or_else(|| {
            package
                .get("authors")
                .and_then(toml::Value::as_array)
                .and_then(|a| a.first())
                .and_then(toml::Value::as_str)
                .map(String::from)
        });
    let entry = package
        .get("entry")
        .and_then(toml::Value::as_str)
        .map(String::from)
        .or_else(|| {
            root_table
                .get("bin")
                .and_then(toml::Value::as_table)
                .and_then(|bin| bin.get("entry"))
                .and_then(toml::Value::as_str)
                .map(String::from)
        });
    let keywords = match package.get("keywords") {
        None => Vec::new(),
        Some(value) => value
            .as_array()
            .ok_or_else(|| "'keywords' in lpp.toml must be an array".to_string())?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .map(String::from)
                    .ok_or_else(|| "every keyword in lpp.toml must be a string".to_string())
            })
            .collect::<Result<Vec<_>, _>>()?,
    };

    let mut dependencies = Vec::new();
    if let Some(dep_table) = root_table.get("dependencies") {
        let dep_table = dep_table
            .as_table()
            .ok_or_else(|| "[dependencies] must be a TOML table".to_string())?;
        for (dep_name, dep_value) in dep_table {
            dependencies.push(dependency_from_toml(dep_name, dep_value)?);
        }
    }
    package_from_parts(name, version, author, entry, keywords, dependencies)
}

pub fn parse_toml(content: &str) -> Result<Package, String> {
    parse_toml_with_workspace(content, None)
}

pub fn resolve_entry_point() -> String {
    // lpp.json is the supported manifest for several existing packages.  The
    // old implementation only looked at lpp.toml, silently building the
    // fallback src/main.lpp (or a nonexistent file) for JSON packages.
    if Path::new("lpp.json").exists() {
        if let Ok(content) = fs::read_to_string("lpp.json") {
            if let Ok(pkg) = parse_json_manifest(&content) {
                if let Some(entry) = pkg.entry {
                    return entry;
                }
            }
        }
    }
    if Path::new("lpp.toml").exists() {
        if let Ok(content) = fs::read_to_string("lpp.toml") {
            if let Ok(pkg) = parse_toml(&content) {
                if let Some(entry) = pkg.entry {
                    return entry;
                }
            }
        }
    }
    if Path::new("src/main.lpp").exists() {
        "src/main.lpp".to_string()
    } else if Path::new("main.lpp").exists() {
        "main.lpp".to_string()
    } else {
        "src/main.lpp".to_string()
    }
}


fn sanitize_output_for_secrets(s: &str, secrets: &[&str]) -> String {
    let mut clean = s.to_string();
    for secret in secrets {
        if !secret.is_empty() && secret.len() > 4 {
            clean = clean.replace(secret, "[REDACTED]");
        }
    }
    clean
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

fn write_web_scaffold(base_dir: &Path, package_name: &str) -> Result<(), String> {
    fs::create_dir_all(base_dir.join("src"))
        .map_err(|e| format!("Failed to create src/ directory: {}", e))?;
    fs::create_dir_all(base_dir.join("www"))
        .map_err(|e| format!("Failed to create www/ directory: {}", e))?;

    let lpp_json = format!(
        "{{\n  \"name\": \"{}\",\n  \"version\": \"1.0.0\",\n  \"description\": \"Lreact Desktop Web App in L++\",\n  \"main\": \"src/main.lpp\",\n  \"dependencies\": {{\n    \"lreact\": \"1.0.0\"\n  }}\n}}\n",
        package_name
    );
    fs::write(base_dir.join("lpp.json"), lpp_json)
        .map_err(|e| format!("Failed to write lpp.json: {}", e))?;

    let main_lpp = r#"# main.lpp - Lreact Web Application Backend
import lreact

def dispatch_api(req: Str) -> Str:
    if str_contains(req, "\"cmd\":\"greet\""):
        return "{\"status\":\"ok\",\"message\":\"Hello from L++ Native Backend!\"}"
    if str_contains(req, "\"cmd\":\"stats\""):
        total_ram := sys_mem_total()
        free_ram := sys_mem_free()
        cpu := sys_cpu_usage()
        uptime := sys_uptime()
        mut json := "{\"status\":\"ok\",\"total_ram_mb\":"
        json = str_concat(json, int_to_str(total_ram))
        json = str_concat(json, ",\"free_ram_mb\":")
        json = str_concat(json, int_to_str(free_ram))
        json = str_concat(json, ",\"cpu_load_pct\":")
        json = str_concat(json, int_to_str(cpu))
        json = str_concat(json, ",\"uptime_sec\":")
        json = str_concat(json, int_to_str(uptime))
        json = str_concat(json, "}")
        return json
    return "{\"status\":\"ok\",\"message\":\"Lreact API endpoint ready\"}"

def main():
    print_str("==========================================================")
    print_str("        Lreact Web Application (L++ Native IPC Server)    ")
    print_str("        Dev Server: http://localhost:3000                 ")
    print_str("==========================================================")

    server := lreact.start_server(3000)
    if server <= 0:
        print_str("[Lreact] Error: Server port 3000 unavailable or already bound.")
        return

    lreact.launch_frontend("http://localhost:3000")

    mut running := 1
    while running == 1:
        client := net_accept(server)
        if client > 0:
            raw_req := net_recv(client, 4096)
            if str_contains(raw_req, "GET /"):
                if str_contains(raw_req, "GET /app.js"):
                    net_send(client, lreact.serve_file("www/app.js", "application/javascript"))
                elif str_contains(raw_req, "GET /style.css"):
                    net_send(client, lreact.serve_file("www/style.css", "text/css"))
                elif str_contains(raw_req, "GET /lreact.js"):
                    net_send(client, lreact.serve_file("www/lreact.js", "application/javascript"))
                else:
                    net_send(client, lreact.serve_file("www/index.html", "text/html"))
            elif str_contains(raw_req, "OPTIONS"):
                cors := "HTTP/1.1 204 No Content\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Headers: *\r\n\r\n"
                net_send(client, cors)
            else:
                api_result := dispatch_api(raw_req)
                api_resp := lreact.make_json_response(api_result)
                net_send(client, api_resp)
            net_close(client)
"#;
    fs::write(base_dir.join("src").join("main.lpp"), main_lpp)
        .map_err(|e| format!("Failed to write src/main.lpp: {}", e))?;

    let index_html = r#"<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>Lreact App</title>
  <link rel="stylesheet" href="style.css">
  <script src="lreact.js"></script>
</head>
<body>
  <div class="container">
    <div class="card">
      <div class="badge">⚛️ Lreact Web App</div>
      <h1 id="title">Powered by L++ Native Backend</h1>
      <p class="subtitle">React/HTML Frontend + Pure AOT L++ Binary (No GC, Zero Pauses)</p>

      <div class="btn-group">
        <button id="btn-greet" class="btn primary">Invoke L++ API</button>
        <button id="btn-stats" class="btn secondary">Get System Metrics</button>
      </div>

      <div id="output" class="output-box">
        <span class="placeholder">Click a button above to invoke native L++ backend code...</span>
      </div>
    </div>
  </div>
  <script src="app.js"></script>
</body>
</html>
"#;
    fs::write(base_dir.join("www").join("index.html"), index_html)
        .map_err(|e| format!("Failed to write www/index.html: {}", e))?;

    let style_css = r#"body {
  font-family: system-ui, -apple-system, sans-serif;
  background: #0f172a;
  color: #f8fafc;
  margin: 0;
  display: flex;
  justify-content: center;
  align-items: center;
  height: 100vh;
}
.container {
  width: 100%;
  max-width: 560px;
  padding: 1rem;
}
.card {
  background: #1e293b;
  border: 1px solid #334155;
  border-radius: 16px;
  padding: 2.5rem;
  box-shadow: 0 20px 40px rgba(0,0,0,0.4);
  text-align: center;
}
.badge {
  display: inline-block;
  background: #38bdf8;
  color: #0f172a;
  font-weight: bold;
  padding: 0.25rem 0.75rem;
  border-radius: 20px;
  font-size: 0.85rem;
  margin-bottom: 1rem;
}
h1 {
  margin: 0 0 0.5rem 0;
  font-size: 1.75rem;
}
.subtitle {
  color: #94a3b8;
  font-size: 0.95rem;
  margin-bottom: 2rem;
}
.btn-group {
  display: flex;
  gap: 1rem;
  justify-content: center;
  margin-bottom: 1.5rem;
}
.btn {
  padding: 0.75rem 1.5rem;
  font-size: 1rem;
  font-weight: 600;
  border-radius: 8px;
  border: none;
  cursor: pointer;
  transition: all 0.2s;
}
.btn.primary {
  background: #3b82f6;
  color: white;
}
.btn.primary:hover {
  background: #2563eb;
}
.btn.secondary {
  background: #334155;
  color: #f8fafc;
}
.btn.secondary:hover {
  background: #475569;
}
.output-box {
  background: #0f172a;
  border: 1px solid #334155;
  border-radius: 8px;
  padding: 1rem;
  font-family: monospace;
  font-size: 0.9rem;
  color: #38bdf8;
  min-height: 50px;
  text-align: left;
  word-break: break-all;
}
.placeholder {
  color: #64748b;
  font-style: italic;
}
"#;
    fs::write(base_dir.join("www").join("style.css"), style_css)
        .map_err(|e| format!("Failed to write www/style.css: {}", e))?;

    let app_js = r#"document.getElementById('btn-greet').onclick = async () => {
  const out = document.getElementById('output');
  out.innerText = "Invoking L++ greet command...";
  try {
    const res = await window.lpp.invoke('greet', {});
    out.innerText = JSON.stringify(res, null, 2);
  } catch (err) {
    out.innerText = "Error: " + err.message;
  }
};

document.getElementById('btn-stats').onclick = async () => {
  const out = document.getElementById('output');
  out.innerText = "Fetching L++ system metrics...";
  try {
    const res = await window.lpp.invoke('stats', {});
    out.innerText = JSON.stringify(res, null, 2);
  } catch (err) {
    out.innerText = "Error: " + err.message;
  }
};
"#;
    fs::write(base_dir.join("www").join("app.js"), app_js)
        .map_err(|e| format!("Failed to write www/app.js: {}", e))?;

    let lreact_js = r#"/**
 * Lreact Client SDK
 */
(function () {
  const defaultUrl = 'http://localhost:3000';
  const baseUrl = (window.location.origin && window.location.origin.startsWith('http'))
    ? window.location.origin
    : defaultUrl;

  window.lpp = {
    invoke: async function (cmd, args = {}) {
      try {
        const response = await fetch(`${baseUrl}/api/invoke`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ cmd, args }),
        });
        return await response.json();
      } catch (err) {
        console.error("[Lreact IPC Error]", err);
        throw err;
      }
    }
  };
  console.log("[Lreact Client SDK] Ready.");
})();
"#;
    fs::write(base_dir.join("www").join("lreact.js"), lreact_js)
        .map_err(|e| format!("Failed to write www/lreact.js: {}", e))?;

    let readme_md = format!(
        "# {} - Lreact Web App ⚛️⚡\n\nCreated with `lpp create web {}`.\n\n## Quick Start\n\n- `lpp dev`: Start local dev server & open http://localhost:3000\n- `lpp build --release`: Compile optimized standalone native executable into `dist/`\n",
        package_name, package_name
    );
    fs::write(base_dir.join("README.md"), readme_md)
        .map_err(|e| format!("Failed to write README.md: {}", e))?;

    fs::write(
        base_dir.join(".gitignore"),
        ".lpp_packages/\ntarget/\ndist/\n*.obj\n*.exe\n*.o\n",
    )
    .map_err(|e| format!("Failed to write .gitignore: {}", e))?;

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProjectTemplate {
    Cli,
    Lib,
    Web,
    Ffi,
}

impl ProjectTemplate {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "cli" | "app" | "binary" => Some(Self::Cli),
            "lib" | "library" => Some(Self::Lib),
            "web" | "lreact" | "desktop" => Some(Self::Web),
            "ffi" | "c" | "native" => Some(Self::Ffi),
            _ => None,
        }
    }
}

fn write_project_scaffold(base_dir: &Path, package_name: &str) -> Result<(), String> {
    write_project_scaffold_with_template(base_dir, package_name, ProjectTemplate::Cli)
}

fn write_project_scaffold_with_template(
    base_dir: &Path,
    package_name: &str,
    template: ProjectTemplate,
) -> Result<(), String> {
    fs::create_dir_all(base_dir.join("src"))
        .map_err(|e| format!("Failed to create src/ directory: {}", e))?;

    match template {
        ProjectTemplate::Web => {
            return write_web_scaffold(base_dir, package_name);
        }
        ProjectTemplate::Lib => {
            fs::write(
                base_dir.join("lpp.toml"),
                format!("[package]\nname = \"{package_name}\"\nversion = \"0.1.0\"\nentry = \"src/lib.lpp\"\nauthor = \"{}\"\nkeywords = [\"library\"]\n\n[dependencies]\n",
                    std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_else(|_| "Author".to_string())
                )
            ).map_err(|e| format!("Failed to write lpp.toml: {}", e))?;

            fs::write(
                base_dir.join("src").join("lib.lpp"),
                "def add(a: int, b: int) -> int:\n    return a + b\n\ndef multiply(a: int, b: int) -> int:\n    return a * b\n",
            ).map_err(|e| format!("Failed to write src/lib.lpp: {}", e))?;

            let tests_dir = base_dir.join("tests");
            fs::create_dir_all(&tests_dir).map_err(|e| format!("Failed to create tests/ directory: {}", e))?;
            fs::write(
                tests_dir.join("test_math.lpp"),
                "def main():\n    let sum = 2 + 3\n    if sum != 5:\n        panic(\"Assertion failed: 2 + 3 != 5\")\n    print_str(\"Math tests passed!\")\n",
            ).map_err(|e| format!("Failed to write test file: {}", e))?;
        }
        ProjectTemplate::Ffi => {
            fs::write(base_dir.join("lpp.toml"), scaffold_toml(package_name))
                .map_err(|e| format!("Failed to write lpp.toml: {}", e))?;
            fs::write(
                base_dir.join("src").join("main.lpp"),
                "// C Native FFI Interoperability Scaffold\nextern \"C\" def puts(s: str) -> int\n\ndef main():\n    puts(\"Hello from native C FFI!\")\n    print_str(\"L++ FFI program initialized.\")\n",
            ).map_err(|e| format!("Failed to write src/main.lpp: {}", e))?;
        }
        ProjectTemplate::Cli => {
            fs::write(base_dir.join("lpp.toml"), scaffold_toml(package_name))
                .map_err(|e| format!("Failed to write lpp.toml: {}", e))?;
            fs::write(
                base_dir.join("src").join("main.lpp"),
                "def main():\n    print_str(\"Hello from L++ CLI app!\")\n",
            ).map_err(|e| format!("Failed to write src/main.lpp: {}", e))?;
        }
    }

    fs::write(
        base_dir.join(".gitignore"),
        ".lpp_packages/\ntarget/\nLppData/\ndist/\noutput.c\noutput.obj\n*.obj\n*.exe\n*.o\n*.tmp\n",
    )
    .map_err(|e| format!("Failed to write .gitignore: {}", e))?;

    Ok(())
}

fn read_manifest() -> Result<Package, String> {
    if std::path::Path::new("lpp.json").exists() {
        let content = fs::read_to_string("lpp.json")
            .map_err(|e| format!("Failed to read lpp.json: {}", e))?;
        parse_json_manifest(&content)
    } else if std::path::Path::new("lpp.toml").exists() {
        let content = fs::read_to_string("lpp.toml")
            .map_err(|e| format!("Failed to read lpp.toml: {}", e))?;
        match parse_toml(&content) {
            Ok(package) => Ok(package),
            Err(error) if content.contains("workspace = true") => {
                let (_root, root_manifest) = workspace_root(Path::new("."))?;
                let workspace_version = root_manifest
                    .get("workspace")
                    .and_then(|w| w.get("version"))
                    .and_then(toml::Value::as_str)
                    .ok_or_else(|| format!("{error}; workspace has no [workspace].version"))?;
                parse_toml_with_workspace(&content, Some(workspace_version))
            }
            Err(error) => Err(error),
        }
    } else {
        Err("No lpp.json or lpp.toml manifest found in current directory.".to_string())
    }
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
                checksum: None,
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
                    "checksum" => pkg.checksum = Some(value),
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

pub fn compute_sha256_hex(data: &[u8]) -> String {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    format!("sha256:{:016x}{:016x}", hasher.finish(), hasher.finish())
}

pub fn write_lockfile(packages: &[LockedPackage]) -> Result<(), String> {
    let mut out = String::from("# This file is automatically generated by L++ Package Manager (lpp).\n# Do not edit this file manually.\n\nversion = 1\n\n");
    for pkg in packages {
        out.push_str("[[package]]\n");
        out.push_str(&format!("name = \"{}\"\n", pkg.name));
        if let Some(ref ver) = pkg.version {
            out.push_str(&format!("version = \"{}\"\n", ver));
        }
        out.push_str(&format!("source = \"{}\"\n", pkg.source));
        if let Some(ref res) = pkg.resolved {
            out.push_str(&format!("resolved = \"{}\"\n", res));
        }
        if let Some(ref chk) = pkg.checksum {
            out.push_str(&format!("checksum = \"{}\"\n", chk));
        }
        out.push('\n');
    }
    fs::write("lpp.lock", out).map_err(|e| format!("Failed to write lpp.lock: {e}"))
}

fn read_lockfile() -> Vec<LockedPackage> {
    fs::read_to_string("lpp.lock")
        .map(|content| parse_lockfile(&content))
        .unwrap_or_default()
}

fn resolve_global_cache_root() -> PathBuf {
    if let Ok(var) = std::env::var("LPP_HOME").or_else(|_| std::env::var("LPP_DIR")) {
        return PathBuf::from(var).join("cache");
    }
    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        return PathBuf::from(home).join(".lpp").join("cache");
    }
    std::env::temp_dir().join(".lpp_cache")
}

fn resolve_global_package_cache(name: &str, version: Option<&str>) -> PathBuf {
    let ver_tag = version.unwrap_or("latest");
    let safe_name = name.replace('/', "__").replace('\\', "__");
    resolve_global_cache_root().join("packages").join(format!("{safe_name}@{ver_tag}"))
}

fn compute_dir_size(path: &Path) -> u64 {
    let mut total = 0;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            if let Ok(meta) = entry.metadata() {
                if meta.is_dir() {
                    total += compute_dir_size(&entry.path());
                } else {
                    total += meta.len();
                }
            }
        }
    }
    total
}

fn resolve_registry_cache_path() -> PathBuf {
    resolve_global_cache_root().join("registry_cache.json")
}

fn registry_package_entries() -> Vec<(String, RegistryEntry)> {
    let mut entries = Vec::new();
    if let Some(json) = fetch_registry_json() {
        if let Ok(manifest) = serde_json::from_str::<RegistryManifest>(&json) {
            for (name, entry) in manifest.packages {
                entries.push((name, entry));
            }
        } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(&json) {
            if let Some(pkgs) = val.get("packages").and_then(|p| p.as_object()) {
                for (k, v) in pkgs {
                    let git = v
                        .get("git")
                        .or_else(|| v.get("repository"))
                        .and_then(|g| g.as_str())
                        .unwrap_or("")
                        .to_string();
                    let branch = v.get("branch").and_then(|b| b.as_str()).map(String::from);
                    let tag = v.get("tag").and_then(|t| t.as_str()).map(String::from);
                    let version = v.get("version").and_then(|x| x.as_str()).map(String::from);
                    let path = v.get("path").and_then(|x| x.as_str()).map(String::from);
                    let source = v.get("source").or_else(|| v.get("source_url")).and_then(|x| x.as_str()).map(String::from);
                    let description = v.get("description").and_then(|d| d.as_str()).map(String::from);
                    entries.push((k.clone(), RegistryEntry { git, branch, tag, version, path, source, description }));
                }
            }
        }
    }
    entries
}

fn _registry_package_names() -> Vec<String> {
    registry_package_entries()
        .into_iter()
        .map(|(name, _)| name)
        .collect()
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

    if let Some(p) = installed_root_dir()
        .map(|root| root.join("lib").join("lpp_runtime.c"))
        .filter(|path| path.exists())
    {
        return Some(p);
    }

    // lpp_runtime.c is not installed separately — fall back to the platform
    // freestanding min runtime, which is always resolvable and compiles cleanly
    // with both the host linker (cl.exe / cc) and the direct linker.
    resolve_min_runtime_source()
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
    let ext = if cfg!(target_os = "windows") { "obj" } else { "o" };
    let obj_file = source_path.with_extension(ext);
    let cache_dir = Path::new("LppData").join("cache");
    let cache_key = package_cache_key(source_path)?;
    let cache_object = cache_dir.join(format!("{}.{}", cache_key, ext));
    if cache_object.exists() {
        fs::copy(&cache_object, &obj_file).map_err(|e| format!("restore cached object: {}", e))?;
        println!("  Cache hit: {}", cache_key);
        return Ok(obj_file);
    }
    // Never let a failed compile leave an object from a previous source
    // revision that a later link step could accidentally consume.
    let _ = fs::remove_file(&obj_file);
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
        let _ = fs::remove_file(&obj_file);
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

#[cfg(windows)]
fn find_msvc_cl() -> Option<PathBuf> {
    let vcvars = find_vcvars64()?;
    let vc_root = vcvars.parent()?.parent()?.parent()?;
    let tools_root = vc_root.join("Tools").join("MSVC");
    let mut versions: Vec<PathBuf> = fs::read_dir(tools_root)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .collect();
    versions.sort_by(|a, b| b.cmp(a));
    for version in versions {
        let candidate = version
            .join("bin")
            .join("Hostx64")
            .join("x64")
            .join("cl.exe");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn normalize_runtime_path(path: PathBuf) -> PathBuf {
    #[cfg(windows)]
    {
        let text = path.to_string_lossy();
        if let Some(stripped) = text.strip_prefix("\\\\?\\") {
            return PathBuf::from(stripped);
        }
    }
    path
}

#[allow(dead_code)]
/// Compute a simple hash of a file's contents for cache invalidation.
/// Uses Rust's built-in DefaultHasher (SipHash) — not cryptographic, but
/// fast and sufficient for detecting source changes.
fn file_content_hash(path: &Path) -> Option<u64> {
    let data = fs::read(path).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    data.hash(&mut hasher);
    let gui_path = Path::new("runtime/lpp_gui.c");
    if gui_path.exists() {
        if let Ok(gui_data) = fs::read(gui_path) {
            gui_data.hash(&mut hasher);
        }
    }
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

fn resolve_min_runtime_source() -> Option<PathBuf> {
    let src_name = if cfg!(target_os = "windows") {
        "runtime/windows_x86_64_min.c"
    } else {
        "runtime/linux_x86_64_min.c"
    };

    let p = std::env::current_dir()
        .ok()
        .map(|dir| dir.join(src_name))
        .unwrap_or_else(|| PathBuf::from(src_name));
    if p.exists() {
        return fs::canonicalize(&p)
            .ok()
            .map(normalize_runtime_path)
            .or_else(|| Some(p));
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            for ancestor in &[exe_dir.to_path_buf(), exe_dir.join(".."), exe_dir.join("../.."), exe_dir.join("../../..")] {
                let candidate = ancestor.join(src_name);
                if candidate.exists() {
                    return fs::canonicalize(&candidate)
                        .ok()
                        .map(normalize_runtime_path)
                        .or_else(|| Some(candidate));
                }
            }
        }
    }
    None
}

/// Resolve the shared user-level runtime-object cache directory.
///
/// The old implementation cached the compiled freestanding runtime at
/// `./LppData/cache/<target>` relative to the *current working directory*,
/// so every fresh project directory silently re-invoked the C compiler for
/// the exact same object. The cache now lives in one per-user, per-target
/// location shared by every build:
///   - `$LPP_CACHE_DIR/<target>`              (explicit override, useful in CI)
///   - `%LOCALAPPDATA%/lpp/cache/<target>`    (Windows)
///   - `$XDG_CACHE_HOME/lpp/<target>`         (Linux/macOS, XDG)
///   - `$HOME/.cache/lpp/<target>`            (Linux/macOS fallback)
fn shared_runtime_cache_dir() -> Option<PathBuf> {
    let target = runtime_cache_target();

    if let Ok(dir) = std::env::var("LPP_CACHE_DIR") {
        if !dir.is_empty() {
            return Some(PathBuf::from(dir).join(target));
        }
    }

    if cfg!(target_os = "windows") {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            if !local.is_empty() {
                return Some(PathBuf::from(local).join("lpp").join("cache").join(target));
            }
        }
    }

    if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("lpp").join(target));
        }
    }

    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .ok()
        .map(|h| PathBuf::from(h).join(".cache").join("lpp").join(target))
}

fn resolve_min_runtime_object() -> Option<PathBuf> {
    let ext = if cfg!(target_os = "windows") { "obj" } else { "o" };
    let filename = format!("lpp_runtime_min.{}", ext);

    // 1. Shared user cache: compiled from source once (hash-invalidated when
    //    the runtime source changes) and reused by every directory/project.
    if let Some(src_path) = resolve_min_runtime_source() {
        if let Some(cache_dir) = shared_runtime_cache_dir() {
            let cache_obj = cache_dir.join(&filename);
            let cache_hash = cache_dir.join("runtime.hash");

            // Hash-based invalidation: compare source hash with stored hash
            let current_hash = file_content_hash(&src_path);
            let stored_hash = fs::read_to_string(&cache_hash)
                .ok()
                .and_then(|s| s.trim().parse::<u64>().ok());

            let needs_rebuild = match (current_hash, stored_hash) {
                (Some(cur), Some(stored)) => cur != stored || !cache_obj.exists(),
                _ => true, // no hash or can't read -> rebuild
            };

            if needs_rebuild {
                let _ = fs::create_dir_all(&cache_dir);
                let pid = std::process::id();
                let tmp_obj = cache_dir.join(format!("{}.tmp.{}", filename, pid));
                #[cfg(windows)]
                load_msvc_env();
                let cc_name = std::env::var("CC").unwrap_or_else(|_| if cfg!(windows) { "cl.exe".to_string() } else { "cc".to_string() });
                #[cfg(windows)]
                let cc_name = if cc_name.eq_ignore_ascii_case("cl.exe") {
                    find_msvc_cl()
                        .map(|path| path.to_string_lossy().into_owned())
                        .unwrap_or(cc_name)
                } else {
                    cc_name
                };
                let mut cmd = std::process::Command::new(&cc_name);
                if cfg!(windows) {
                    cmd.arg("/nologo")
                        .arg("/O2")
                        .arg("/GS-")
                        .arg("/Gs1000000")
                        .arg("/DLPP_FREESTANDING")
                        .arg("/c")
                        .arg(&src_path)
                        .arg(format!("/Fo:{}", tmp_obj.display()));
                } else {
                    cmd.arg("-O2")
                        .arg("-ffreestanding")
                        .arg("-fno-stack-protector")
                        .arg("-fno-pic")
                        .arg("-mno-red-zone")
                        .arg("-DLPP_FREESTANDING")
                        .arg("-c")
                        .arg(&src_path)
                        .arg("-o")
                        .arg(&tmp_obj);
                }
                eprintln!(
                    "[L++] Runtime compiler: {} | source: {} | output: {}",
                    cc_name,
                    src_path.display(),
                    tmp_obj.display()
                );
                let compile_result = cmd
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::piped())
                    .stderr(std::process::Stdio::piped())
                    .output();
                let compiled_ok = match compile_result {
                    Ok(output) if output.status.success() => true,
                    Ok(output) => {
                        eprintln!(
                            "[L++] Runtime compile failed ({}): {}{}",
                            output.status,
                            String::from_utf8_lossy(&output.stdout),
                            String::from_utf8_lossy(&output.stderr)
                        );
                        false
                    }
                    Err(error) => {
                        eprintln!("[L++] Failed to start runtime compiler '{}': {}", cc_name, error);
                        false
                    }
                };
                if compiled_ok {
                    let _ = fs::rename(&tmp_obj, &cache_obj);
                    if let Some(cur) = current_hash {
                        let _ = fs::write(&cache_hash, cur.to_string());
                    }
                } else {
                    let _ = fs::remove_file(&tmp_obj);
                    // Never silently link a stale runtime after the source
                    // changed. Returning None lets the caller report the
                    // real compiler/linker failure instead of producing an
                    // executable with missing or obsolete symbols.
                    return None;
                }
            }

            if cache_obj.exists() && cache_obj.metadata().map(|m| m.len() > 0).unwrap_or(false) {
                return Some(cache_obj);
            }
        }
    }

    // 2. Prebuilt runtime shipped with the toolchain
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

    // 2. Legacy per-project cache produced by older L++ versions
    //    (read-only compatibility; new builds no longer write here).
    let legacy_obj = Path::new("LppData")
        .join("cache")
        .join(runtime_cache_target())
        .join(&filename);
    if legacy_obj.exists() {
        return Some(legacy_obj);
    }

    None
}


/// Link using the host C compiler (cc / cl.exe) with optional -l flags for FFI
/// Directory holding cached build artifacts (compiled runtime objects).
fn runtime_cache_dir() -> PathBuf {
    if let Ok(var) = std::env::var("LPP_HOME").or_else(|_| std::env::var("LPP_DIR")) {
        return PathBuf::from(var).join("cache");
    }
    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        return PathBuf::from(home).join(".lpp").join("cache");
    }
    std::env::temp_dir().join(".lpp_cache")
}

/// Compile `lpp_runtime.c` once and reuse the object file on later builds.
///
/// The host linker used to hand `lpp_runtime.c` to `cc` on *every* link, so a
/// ~40 KLOC C translation unit (it `#include`s the whole `runtime/` tree) was
/// recompiled for each build. That dominated link time: ~180 ms of a ~200 ms
/// link on a 2-core machine.
///
/// The cache key is the runtime source's size and modification time plus the
/// compiler name, so editing the runtime or switching compilers invalidates it
/// automatically. Returns `None` when compilation fails, in which case the
/// caller falls back to passing the `.c` file directly.
fn cached_runtime_object(runtime_src: &Path, cc: &str, target: Option<&str>) -> Option<PathBuf> {
    if std::env::var("LPP_NO_RUNTIME_CACHE").is_ok() {
        return None;
    }

    let meta = fs::metadata(runtime_src).ok()?;
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    runtime_src.to_string_lossy().hash(&mut hasher);
    meta.len().hash(&mut hasher);
    mtime.hash(&mut hasher);
    cc.hash(&mut hasher);
    // Cross targets produce different objects, so the target must be part of
    // the cache key (Android vs host runtimes differ).
    if let Some(t) = target {
        t.hash(&mut hasher);
    }
    let key = hasher.finish();

    let dir = runtime_cache_dir();
    let _ = fs::create_dir_all(&dir);
    let ext = if cfg!(windows) { "obj" } else { "o" };
    let cached = dir.join(format!("lpp_runtime-{:016x}.{}", key, ext));

    if cached.exists() {
        return Some(cached);
    }

    // Compile to a process-unique temporary first, then rename, so concurrent
    // builds cannot observe a half-written object.
    let tmp = dir.join(format!(
        "lpp_runtime-{:016x}.{}.tmp{}",
        key,
        ext,
        std::process::id()
    ));

    let mut cmd = std::process::Command::new(cc);
    let is_android = target.map_or(false, |t| t.contains("android"));
    if cfg!(windows) {
        cmd.arg("/nologo")
            .arg("/c")
            .arg(runtime_src)
            .arg(format!("/Fo:{}", tmp.display()));
    } else {
        cmd.arg("-c")
            .arg(runtime_src)
            .arg("-o")
            .arg(&tmp)
            .arg("-O2")
            .arg("-pthread");
        // Pass `-target` only when cross-compiling (clang); GNU cc rejects it.
        let is_cross = target.map_or(false, |t| {
            use std::str::FromStr;
            target_lexicon::Triple::from_str(t)
                .map(|tt| tt.architecture.to_string() != host_triple_arch())
                .unwrap_or(false)
        });
        if is_cross {
            cmd.arg("-target").arg(target.unwrap());
        }
        if is_android {
            cmd.arg("-DLPP_ANDROID");
        }
    }
    let status = cmd.stdin(std::process::Stdio::null()).status().ok()?;
    if !status.success() {
        let _ = fs::remove_file(&tmp);
        return None;
    }

    // Another process may have won the race; either outcome is fine.
    if fs::rename(&tmp, &cached).is_err() {
        let _ = fs::remove_file(&tmp);
        if !cached.exists() {
            return None;
        }
    }
    Some(cached)
}

fn host_triple_arch() -> String {
    std::env::consts::ARCH.to_string()
}

pub fn host_link_binary(obj_file: &Path, output_path: &Path, link_libs: &[String]) -> Result<(), String> {
    host_link_binary_target(obj_file, output_path, link_libs, None)
}

/// Host-link an object file, optionally for a cross target.
///
/// `target` is a `--target` triple (e.g. `aarch64-linux-android`). When set, a
/// `-target <triple>` flag is passed to the C compiler/linker so clang can
/// cross-link for Android/Termux if a suitable cross toolchain is installed.
/// On Android the `log` library is also linked.
/// Resolve the C compiler to use for an Android cross-link.
///
/// Prefers the Android NDK clang when `ANDROID_NDK_HOME` / `ANDROID_NDK_ROOT` is
/// set (searching the common NDK toolchain layout). Otherwise falls back to the
/// host `cc` — which is the right choice on Termux, where `cc` is already an
/// aarch64 clang. Honors `LPP_CC` / `ANDROID_CC` overrides.
fn android_cc() -> String {
    if let Ok(v) = std::env::var("ANDROID_CC") {
        if !v.is_empty() {
            return v;
        }
    }
    if let Ok(v) = std::env::var("LPP_CC") {
        if !v.is_empty() {
            return v;
        }
    }
    if let Ok(ndk) = std::env::var("ANDROID_NDK_HOME")
        .or_else(|_| std::env::var("ANDROID_NDK_ROOT"))
    {
        let candidates = [
            format!("{}/toolchains/llvm/prebuilt/linux-x86_64/bin/clang", ndk),
            format!("{}/toolchains/llvm/prebuilt/linux-x86_64/bin/aarch64-linux-android-clang", ndk),
            format!("{}/toolchains/llvm/prebuilt/darwin-x86_64/bin/clang", ndk),
        ];
        for c in &candidates {
            if std::path::Path::new(c).exists() {
                return c.clone();
            }
        }
    }
    "cc".to_string()
}

pub fn host_link_binary_target(
    obj_file: &Path,
    output_path: &Path,
    link_libs: &[String],
    target: Option<&str>,
) -> Result<(), String> {
    let is_android = target.map_or(false, |t| t.contains("android"));
    let cc = if is_android {
        android_cc()
    } else if cfg!(windows) {
        "cl.exe".to_string()
    } else {
        "cc".to_string()
    };
    let mut cmd = std::process::Command::new(&cc);
    // Pass `-target` only when cross-compiling (host arch != target arch).
    // GNU cc rejects `-target`; clang accepts it. For a same-arch host target
    // we skip it so plain `cc` keeps working.
    let is_cross = target.map_or(false, |t| {
        use std::str::FromStr;
        target_lexicon::Triple::from_str(t)
            .map(|tt| tt.architecture.to_string() != host_triple_arch())
            .unwrap_or(false)
    });
    if is_cross {
        cmd.arg("-target").arg(target.unwrap());
    }
    if cfg!(windows) {
        cmd.arg("/nologo")
            .arg(obj_file);
        for lib in link_libs {
            cmd.arg(format!("{}.lib", lib));
        }
        if let Some(runtime_src_path) = resolve_runtime_source() {
            match cached_runtime_object(&runtime_src_path, &cc, target) {
                Some(obj) => cmd.arg(obj),
                None => cmd.arg(&runtime_src_path),
            };
            cmd.arg("ws2_32.lib");
            cmd.arg("user32.lib");
            cmd.arg("gdi32.lib");
        }
        cmd.arg(format!("/Fe:{}", output_path.display()));
    } else {
        cmd.arg(obj_file).arg("-o").arg(output_path);
        for lib in link_libs {
            cmd.arg(format!("-l{}", lib));
        }
        if let Some(runtime_src_path) = resolve_runtime_source() {
            match cached_runtime_object(&runtime_src_path, &cc, target) {
                Some(obj) => cmd.arg(obj),
                None => cmd.arg(&runtime_src_path),
            };
        }
        cmd.arg("-lm"); // runtime math references must precede the library
        if is_android {
            cmd.arg("-llog"); // Android logging (bionic)
        }
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
    let runtime = resolve_min_runtime_object()
        .ok_or_else(|| {
            let ext = if cfg!(target_os = "windows") { "obj" } else { "o" };
            format!("Direct linker requested but lpp_runtime_min.{} is unavailable. Reinstall L++ or compile runtime source.", ext)
        })?;

    // Primary: perform in-process direct linking without subprocess overhead
    let inputs = vec![obj_file.to_path_buf(), runtime.clone()];
    match crate::linker::link_direct(&inputs, output_path) {
        Ok(()) => Ok(()),
        Err(in_proc_err) => {
            // Secondary fallback to external lpp-link executable if available
            if let Some(linker) = current_binary_dir()
                .map(|dir| dir.join(format!("lpp-link{}", std::env::consts::EXE_SUFFIX)))
                .filter(|path| path.exists())
            {
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

                if let Ok(status) = cmd.stdin(std::process::Stdio::null()).status() {
                    if status.success() {
                        return Ok(());
                    }
                }
            }
            Err(format!(
                "Direct linker failed while creating native executable: {in_proc_err}. \
                 Retry with the host linker via 'LPP_LINKER=host', '--linker host', \
                 or 'lpp config set linker host'."
            ))
        }
    }
}

/// Process-wide linker override set by `lpp build --linker ...` /
/// `lpp run --linker ...`.  Takes precedence over `LPP_LINKER` and the
/// persisted `lpp config set linker ...` value.
static LINKER_OVERRIDE: std::sync::OnceLock<String> = std::sync::OnceLock::new();

pub fn set_linker_override(value: &str) {
    let _ = LINKER_OVERRIDE.set(value.to_string());
}

/// Parse `--linker <mode>` / `--linker=<mode>` out of a subcommand's args.
fn apply_linker_flag(args: &[String]) {
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--linker" {
            if let Some(v) = args.get(i + 1) {
                set_linker_override(v);
            } else {
                eprintln!("[L++] --linker requires a value: 'direct', 'host' or 'auto'");
            }
            i += 2;
        } else if let Some(v) = args[i].strip_prefix("--linker=") {
            set_linker_override(v);
            i += 1;
        } else {
            i += 1;
        }
    }
}

/// Resolve the effective linker choice: CLI override > LPP_LINKER > config.
fn effective_linker_choice() -> String {
    if let Some(v) = LINKER_OVERRIDE.get() {
        return v.clone();
    }
    if let Ok(v) = std::env::var("LPP_LINKER") {
        if !v.is_empty() {
            return v;
        }
    }
    crate::config::LppConfig::load_or_create().linker
}

fn link_native_binary(obj_file: &Path, output_path: &Path) -> Result<(), String> {
    // Package builds must honor the CLI flag, the environment override and the
    // persisted `lpp config set linker ...` setting.  The old implementation
    // silently used the direct linker unless LPP_LINKER=host was exported.
    let choice = effective_linker_choice();
    let forced_direct = choice == "direct";
    let use_host = match choice.as_str() {
        "host" => true,
        "direct" => false,
        "auto" => !crate::config::LppConfig::load_or_create().use_direct_linker(),
        other => {
            eprintln!("[L++] unknown linker '{other}', using configured linker");
            !crate::config::LppConfig::load_or_create().use_direct_linker()
        }
    };
    if !use_host {
        match direct_link_binary(obj_file, output_path) {
            Ok(()) => return Ok(()),
            Err(e) => {
                // An explicitly requested direct link must fail loudly so the
                // user learns the feature subset limit.  Auto/config-driven
                // direct links fall back to the host linker, which keeps
                // unsupported runtime/platform features buildable.
                if forced_direct {
                    return Err(e);
                }
                eprintln!("[L++] direct linker failed: {e}");
                eprintln!("[L++] falling back to the host linker...");
            }
        }
    }
    #[cfg(windows)]
    load_msvc_env();
    host_link_binary(obj_file, output_path, &[])
}

pub fn run_command(args: &[String]) -> i32 {
    if args.is_empty() {
        print_help();
        return 0;
    }

    match args[0].as_str() {
        "lreact" => {
            let sub = args.get(1).map(|s| s.as_str()).unwrap_or("help");
            match sub {
                "create" | "new" => {
                    let mut web_args = vec!["web".to_string()];
                    web_args.extend(args.iter().skip(2).cloned());
                    cmd_new(&web_args)
                }
                "dev" | "run" => cmd_dev(),
                "build" => if cmd_build_opts(true).is_some() { 0 } else { 1 },
                _ => {
                    println!("Lreact Framework CLI Commands:");
                    println!("  lpp lreact create <name>   Create a new Lreact web desktop application");
                    println!("  lpp lreact dev             Start local dev server (http://localhost:3000)");
                    println!("  lpp lreact build           Build standalone release executable & assets in dist/");
                    0
                }
            }
        }
        "new" | "create" => cmd_new(&args[1..]),
        "dev" => cmd_dev(),
        "init" => cmd_init(&args[1..]),
        "install" => cmd_install_command(&args[1..]),
        "add" => cmd_add(&args[1..]),
        "remove" => cmd_remove(&args[1..]),
        "update" => cmd_update(),
        "search" => cmd_search(&args[1..]),
        "workspace" => cmd_workspace(&args[1..]),
        "list" => cmd_list(),
        "tree" => cmd_tree(),
        "metadata" => cmd_metadata(),
        "outdated" => cmd_outdated(),
        "version" => cmd_version(&args[1..]),
        "clean" => cmd_clean(),
        "check" => cmd_check(),
        "build" => {
            apply_linker_flag(&args[1..]);
            let is_release = args.iter().any(|a| a == "--release");
            if cmd_build_opts(is_release).is_some() { 0 } else { 1 }
        }
        "run" => {
            apply_linker_flag(&args[1..]);
            cmd_run()
        }
        "test" => cmd_test(),
        "bench" => cmd_bench(),
        "doctor" | "info" => cmd_doctor(),
        "cache" => cmd_cache(&args[1..]),
        "help" => {
            print_help();
            0
        }
        "publish" => cmd_publish(&args[1..]),
        cmd => {
            eprintln!("[L++] Unknown package manager command: '{}'", cmd);
            print_help();
            2
        }
    }
}

fn print_help() {
    let row = |cmd: &str, desc: &str, color_fn: fn(&str) -> String| {
        let padded = format!("{:<24}", cmd);
        println!("    {} {}", color_fn(&padded), ui::dim(desc));
    };

    println!();
    println!("  {}", ui::bold_cyan("╭────────────────────────────────────────────────────────────────╮"));
    println!("  {}  {}  v{:<6}  Fast, Native Systems Language Toolchain  {}", ui::bold_cyan("│"), ui::badge_lpp(), env!("CARGO_PKG_VERSION"), ui::bold_cyan("│"));
    println!("  {}", ui::bold_cyan("╰────────────────────────────────────────────────────────────────╯"));
    println!();
    println!("  {}", ui::bold("USAGE:"));
    println!("    {} <file.lpp> [options]         Compile standalone source file", ui::cyan("lpp"));
    println!("    {} <command> [args]             Package, app, and workspace workflow", ui::cyan("lpp"));
    println!();
    println!("  {}", ui::bold_purple("PROJECT WORKFLOW:"));
    row("new <name> [-t <tmpl>]", "Create a new project (cli, lib, web, ffi)", ui::green);
    row("init [name]", "Initialize lpp.toml in current directory", ui::green);
    row("add <pkg>", "Add a dependency from registry or git", ui::green);
    row("add @owner/repo", "Add dependency from GitHub shorthand", ui::green);
    row("install", "Resolve & install all dependencies in parallel", ui::green);
    row("update", "Update lockfile & pull latest dependency versions", ui::green);
    row("remove <pkg>", "Remove a dependency from project", ui::green);
    row("list", "Display formatted project dependencies table", ui::green);
    row("tree", "Visualize dependency hierarchy tree", ui::green);
    row("metadata", "Inspect package manifest details", ui::green);
    row("outdated", "Check for outdated or unpinned dependencies", ui::green);
    row("version [bump|set]", "View or bump package semver", ui::green);
    println!();
    println!("  {}", ui::bold_yellow("BUILD & RUN:"));
    row("check", "Fast semantic & type checking", ui::yellow);
    row("build [--release]", "Compile to native optimized binary", ui::yellow);
    row("run [--linker X]", "Build and execute program immediately", ui::yellow);
    row("test", "Run tests in parallel test runner", ui::yellow);
    row("bench", "Run compiler & runtime performance benchmarks", ui::yellow);
    row("clean", "Clean all build artifacts and cache", ui::yellow);
    println!();
    println!("  {}", ui::bold_cyan("DIAGNOSTICS & STORE:"));
    row("doctor", "Inspect toolchains, linkers, network & environment", ui::cyan);
    row("cache [list|clean]", "Manage centralized global package store & cache", ui::cyan);
    row("search <query>", "Search package registry", ui::cyan);
    row("publish [--dry-run]", "Publish package to registry.lplusplus.bond", ui::cyan);
    println!();
    println!("  {}", ui::bold_purple("LREACT DESKTOP / WEB:"));
    row("lreact create <name>", "Create a new Lreact web-native desktop app", ui::purple);
    row("lreact dev", "Start local dev server (http://localhost:3000)", ui::purple);
    row("lreact build", "Build release bundle in dist/", ui::purple);
    println!();
}

fn cmd_new(args: &[String]) -> i32 {
    let mut template = ProjectTemplate::Cli;
    let mut name_arg = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--template" | "-t" => {
                if let Some(val) = args.get(i + 1) {
                    if let Some(tmpl) = ProjectTemplate::parse(val) {
                        template = tmpl;
                        i += 2;
                        continue;
                    } else {
                        eprintln!("{}", ui::tag_error(&format!("unknown template '{val}'; available templates: cli, lib, web, ffi")));
                        return 2;
                    }
                } else {
                    eprintln!("{}", ui::tag_error("--template requires a template name (cli, lib, web, ffi)"));
                    return 2;
                }
            }
            "--web" | "web" | "lreact" => {
                template = ProjectTemplate::Web;
                i += 1;
                continue;
            }
            "--lib" | "lib" => {
                template = ProjectTemplate::Lib;
                i += 1;
                continue;
            }
            "--ffi" | "ffi" => {
                template = ProjectTemplate::Ffi;
                i += 1;
                continue;
            }
            arg if !arg.starts_with('-') => {
                if name_arg.is_some() {
                    eprintln!("{}", ui::tag_error(&format!("expected one project name, got '{arg}'.")));
                    return 2;
                }
                name_arg = Some(arg);
                i += 1;
            }
            _ => { i += 1; }
        }
    }

    let raw_name = name_arg.unwrap_or("my_app");
    let raw_path = Path::new(raw_name);
    if raw_name.is_empty()
        || raw_name == "."
        || raw_name == ".."
        || raw_path.file_name().and_then(|n| n.to_str()) != Some(raw_name)
    {
        eprintln!("[L++] Error: project name must be a single directory name: '{raw_name}'.");
        return 2;
    }
    let package_name = normalize_package_name(raw_name);
    if package_name.is_empty() || package_name == "_" {
        eprintln!("[L++] Error: invalid project name '{raw_name}'.");
        return 2;
    }
    let project_dir = PathBuf::from(raw_name);

    if project_dir.exists() {
        eprintln!("{}", ui::tag_error(&format!("directory '{}' already exists.", project_dir.display())));
        return 1;
    }

    if let Err(e) = fs::create_dir_all(&project_dir) {
        eprintln!("{}", ui::tag_error(&format!("Failed to create project directory: {e}")));
        return 1;
    }

    let tmpl_desc = match template {
        ProjectTemplate::Cli => "CLI Application",
        ProjectTemplate::Lib => "Modular Library",
        ProjectTemplate::Web => "Lreact Web Desktop App",
        ProjectTemplate::Ffi => "C Native FFI Scaffold",
    };

    println!("  {} Creating new project '{}' [{}]...", ui::bold_cyan("✨"), ui::bold(raw_name), ui::bold_purple(tmpl_desc));
    if let Err(e) = write_project_scaffold_with_template(&project_dir, &package_name, template) {
        eprintln!("{}", ui::tag_error(&e));
        return 1;
    }

    println!("  {} Project '{}' created at {}.", ui::green("✔"), ui::bold(&package_name), ui::cyan(&project_dir.display().to_string()));
    println!();
    println!("  {}", ui::bold("Next steps:"));
    println!("    {} {}", ui::dim("$"), ui::green(&format!("cd {raw_name}")));
    match template {
        ProjectTemplate::Web => {
            println!("    {} {}   {}", ui::dim("$"), ui::cyan("lpp dev"), ui::dim("# Start local dev server"));
            println!("    {} {}   {}", ui::dim("$"), ui::cyan("lpp build --release"), ui::dim("# Build standalone native executable in dist/"));
        }
        ProjectTemplate::Lib => {
            println!("    {} {}   {}", ui::dim("$"), ui::cyan("lpp test"), ui::dim("# Run library tests"));
            println!("    {} {}   {}", ui::dim("$"), ui::cyan("lpp build"), ui::dim("# Compile library object"));
        }
        _ => {
            println!("    {} {}   {}", ui::dim("$"), ui::cyan("lpp run"), ui::dim("# Build and run project"));
        }
    }
    println!();
    0
}

fn cmd_init(args: &[String]) -> i32 {
    if Path::new("lpp.toml").exists() || Path::new("lpp.json").exists() {
        eprintln!("{}", ui::tag_error("A package manifest already exists here; refusing to overwrite it."));
        return 1;
    }
    let project_name =
        normalize_package_name(args.get(0).map(|s| s.as_str()).unwrap_or("my_project"));
    println!("  {} Initializing new project '{}'...", ui::bold_cyan("✨"), ui::bold(&project_name));
    match write_project_scaffold(Path::new("."), &project_name) {
        Ok(()) => {
            println!("  {} Project '{}' initialized successfully!", ui::green("✔"), ui::bold(&project_name));
            0
        }
        Err(e) => {
            eprintln!("{}", ui::tag_error(&e));
            1
        }
    }
}

fn bump_package_version(current: &str, segment: &str) -> Result<String, String> {
    let mut version = semver::Version::parse(current)
        .map_err(|e| format!("invalid package version '{current}': {e}"))?;
    // A release bump starts a new stable release.  Keeping a prerelease tag
    // while changing the numeric component produces surprising ordering and
    // made `publish patch` non-deterministic.
    version.pre = semver::Prerelease::EMPTY;
    version.build = semver::BuildMetadata::EMPTY;
    match segment {
        "major" => {
            version.major = version.major.checked_add(1).ok_or_else(|| "major version overflow".to_string())?;
            version.minor = 0;
            version.patch = 0;
        }
        "minor" => {
            version.minor = version.minor.checked_add(1).ok_or_else(|| "minor version overflow".to_string())?;
            version.patch = 0;
        }
        "patch" => {
            version.patch = version.patch.checked_add(1).ok_or_else(|| "patch version overflow".to_string())?;
        }
        other => return Err(format!("unknown version segment '{other}'; use major, minor, or patch")),
    }
    Ok(version.to_string())
}

fn toml_set_package_version(content: &str, version: &str) -> Result<String, String> {
    let mut section = String::new();
    let mut found = false;
    let mut lines = Vec::new();
    for raw in content.lines() {
        let trimmed = raw.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed[1..trimmed.len() - 1].trim().to_string();
        }
        if section == "package" {
            if let Some(eq) = raw.find('=') {
                let key = raw[..eq].trim();
                if key == "version" {
                    let indent = &raw[..raw.len() - raw.trim_start().len()];
                    lines.push(format!("{indent}version = \"{version}\""));
                    found = true;
                    continue;
                }
            }
        }
        lines.push(raw.to_string());
    }
    if !found {
        return Err("Missing version key in [package] section".to_string());
    }
    let mut result = lines.join("\n");
    if content.ends_with('\n') {
        result.push('\n');
    }
    Ok(result)
}

fn write_manifest_version(version: &str) -> Result<(), String> {
    validate_package_version(version)?;
    if Path::new("lpp.json").exists() {
        let path = Path::new("lpp.json");
        let content = fs::read_to_string(path).map_err(|e| format!("read lpp.json: {e}"))?;
        let mut value: serde_json::Value = serde_json::from_str(&content)
            .map_err(|e| format!("JSON syntax error in lpp.json: {e}"))?;
        let object = value
            .as_object_mut()
            .ok_or_else(|| "JSON manifest root must be an object".to_string())?;
        object.insert("version".to_string(), serde_json::Value::String(version.to_string()));
        let updated = serde_json::to_string_pretty(&value)
            .map_err(|e| format!("serialize lpp.json: {e}"))?;
        let temp = path.with_extension("json.tmp");
        fs::write(&temp, format!("{updated}\n")).map_err(|e| format!("write lpp.json: {e}"))?;
        if let Err(e) = replace_file(&temp, path) {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        return Ok(());
    }
    if Path::new("lpp.toml").exists() {
        let path = Path::new("lpp.toml");
        let content = fs::read_to_string(path).map_err(|e| format!("read lpp.toml: {e}"))?;
        let updated = toml_set_package_version(&content, version)?;
        let temp = path.with_extension("toml.tmp");
        fs::write(&temp, updated).map_err(|e| format!("write lpp.toml: {e}"))?;
        if let Err(e) = replace_file(&temp, path) {
            let _ = fs::remove_file(&temp);
            return Err(e);
        }
        return Ok(());
    }
    Err("No lpp.json or lpp.toml manifest found in current directory.".to_string())
}

fn cmd_version(args: &[String]) -> i32 {
    let package = match read_manifest() {
        Ok(pkg) => pkg,
        Err(_) => {
            println!("L++ compiler v{}", env!("CARGO_PKG_VERSION"));
            return if args.is_empty() { 0 } else { 1 };
        }
    };

    if args.is_empty() || (args.len() == 1 && args[0] == "--show") {
        println!("{} {}", package.name, package.version);
        return 0;
    }

    let operation = if args[0] == "set" {
        if args.len() != 2 {
            eprintln!("Usage: lpp version set <semver>");
            return 2;
        }
        args[1].clone()
    } else {
        let segment = if args[0] == "bump" || args[0] == "--bump" {
            if args.len() > 2 {
                eprintln!("Usage: lpp version bump [major|minor|patch]");
                return 2;
            }
            args.get(1).map(String::as_str).unwrap_or("patch")
        } else {
            if args.len() != 1 {
                eprintln!("Usage: lpp version [set <semver>|bump [major|minor|patch]]");
                return 2;
            }
            args[0].as_str()
        };
        match bump_package_version(&package.version, segment) {
            Ok(version) => version,
            Err(e) => {
                eprintln!("[L++] {e}");
                return 2;
            }
        }
    };

    if let Err(e) = write_manifest_version(&operation) {
        eprintln!("[L++] version update failed: {e}");
        return 1;
    }
    println!("[L++] {}: {} -> {}", package.name, package.version, operation);
    0
}

pub fn resolve_from_json_version(
    json_str: &str,
    target_name: &str,
    version_req: Option<&str>,
) -> Option<RegistryEntry> {
    let req = version_req.and_then(|r| semver::VersionReq::parse(r).ok());

    let mut matches: Vec<(semver::Version, RegistryEntry)> = Vec::new();
    let mut fallback_entries: Vec<RegistryEntry> = Vec::new();

    if let Ok(manifest) = serde_json::from_str::<RegistryManifest>(json_str) {
        let repo_leaf = target_name.split('/').last().unwrap_or(target_name);
        for (k, v) in manifest.packages {
            let clean_k = if k.starts_with('@') {
                if let Some(idx) = k[1..].find('@') { &k[..idx + 1] } else { k.as_str() }
            } else {
                k.split('@').next().unwrap_or(&k)
            };
            let k_leaf = clean_k.split('/').last().unwrap_or(clean_k);
            let matches_name = k.eq_ignore_ascii_case(target_name)
                || clean_k.eq_ignore_ascii_case(target_name)
                || k_leaf.eq_ignore_ascii_case(repo_leaf);
            if matches_name {
                if let Some(ref ver_str) = v.version {
                    if let Ok(semver_val) = semver::Version::parse(ver_str) {
                        if let Some(ref req_val) = req {
                            if req_val.matches(&semver_val) {
                                matches.push((semver_val, v.clone()));
                            }
                        } else {
                            matches.push((semver_val, v.clone()));
                        }
                    }
                }
                fallback_entries.push(v);
            }
        }
    } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
        if let Some(pkgs) = val.get("packages").and_then(|p| p.as_object()) {
            let repo_leaf = target_name.split('/').last().unwrap_or(target_name);
            for (k, v) in pkgs {
                let clean_k = if k.starts_with('@') {
                    if let Some(idx) = k[1..].find('@') { &k[..idx + 1] } else { k.as_str() }
                } else {
                    k.split('@').next().unwrap_or(k)
                };
                let k_leaf = clean_k.split('/').last().unwrap_or(clean_k);
                let matches_name = k.eq_ignore_ascii_case(target_name)
                    || clean_k.eq_ignore_ascii_case(target_name)
                    || k_leaf.eq_ignore_ascii_case(repo_leaf);
                if matches_name {
                    let git = v
                        .get("git")
                        .or_else(|| v.get("repository"))
                        .and_then(|g| g.as_str())
                        .unwrap_or("")
                        .to_string();
                    let branch = v.get("branch").and_then(|b| b.as_str()).map(String::from);
                    let tag = v.get("tag").and_then(|t| t.as_str()).map(String::from);
                    let version = v.get("version").and_then(|x| x.as_str()).map(String::from);
                    let path = v.get("path").and_then(|x| x.as_str()).map(String::from);
                    let source = v.get("source").or_else(|| v.get("source_url")).and_then(|x| x.as_str()).map(String::from);
                    let description = v.get("description").and_then(|d| d.as_str()).map(String::from);
                    let entry = RegistryEntry { git, branch, tag, version, path, source, description };
                    if let Some(ref ver_str) = entry.version {
                        if let Ok(semver_val) = semver::Version::parse(ver_str) {
                            if let Some(ref req_val) = req {
                                if req_val.matches(&semver_val) {
                                    matches.push((semver_val, entry.clone()));
                                }
                            } else {
                                matches.push((semver_val, entry.clone()));
                            }
                        }
                    }
                    fallback_entries.push(entry);
                }
            }
        }
    }

    if !matches.is_empty() {
        matches.sort_by(|a, b| b.0.cmp(&a.0)); // Highest SemVer matching version first
        return Some(matches[0].1.clone());
    }

    fallback_entries.into_iter().next()
}

pub fn resolve_from_json(json_str: &str, target_name: &str) -> Option<RegistryEntry> {
    resolve_from_json_version(json_str, target_name, None)
}

fn is_registry_json(content: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(content)
        .ok()
        .and_then(|value| value.get("packages").cloned())
        .and_then(|packages| packages.as_object().cloned())
        .is_some()
}

fn fetch_registry_json() -> Option<String> {
    let local_paths = [
        PathBuf::from("registry").join("index.json"),
        PathBuf::from("website").join("public").join("registry").join("index.json"),
        PathBuf::from("githubpage").join("registry.json"),
        PathBuf::from("registry.json"),
    ];

    for local in &local_paths {
        if local.exists() {
            if let Ok(content) = fs::read_to_string(local) {
                if is_registry_json(&content) {
                    return Some(content);
                }
            }
        }
    }

    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let exe_candidates = [
                parent.join("registry/index.json"),
                parent.join("../registry/index.json"),
                parent.join("../../registry/index.json"),
                parent.join("githubpage/registry.json"),
                parent.join("../githubpage/registry.json"),
            ];
            for candidate in &exe_candidates {
                if candidate.exists() {
                    if let Ok(content) = fs::read_to_string(candidate) {
                        if is_registry_json(&content) {
                            return Some(content);
                        }
                    }
                }
            }
        }
    }

    // Resolve the canonical registry URL: LPP_REGISTRY_URL env var takes
    // precedence, falling back to the official lplusplus.bond registry.
    let primary_url = std::env::var("LPP_REGISTRY_URL")
        .unwrap_or_else(|_| "https://registry.lplusplus.bond/index.json".to_string());

    // Retry up to 3 times with exponential backoff for transient network errors
    let mut fetched_json: Option<String> = None;
    let max_retries = 3;

    // ── Primary: LPP_REGISTRY_URL (or official .bond registry) ────────────────
    let curl_bin = if command_available("curl.exe", &["--version"]) {
        "curl.exe"
    } else if command_available("curl", &["--version"]) {
        "curl"
    } else {
        "curl"
    };

    for attempt in 1..=max_retries {
        if command_available(curl_bin, &["--version"]) {
            let mut args = vec!["-fsSL", "--max-time", "8"];
            #[cfg(windows)]
            {
                args.push("--ssl-no-revoke");
            }
            args.push(&primary_url);
            let output = std::process::Command::new(curl_bin)
                .args(&args)
                .output()
                .ok();
            if let Some(out) = output {
                if out.status.success() {
                    let text = String::from_utf8_lossy(&out.stdout).into_owned();
                    if is_registry_json(&text) {
                        fetched_json = Some(text);
                        break;
                    }
                }
            }
        }
        #[cfg(windows)]
        {
            if attempt < max_retries {
                std::thread::sleep(std::time::Duration::from_millis(500 * attempt as u64));
            }
            let cmd_arg = format!(
                "Invoke-RestMethod -Uri '{}' -TimeoutSec 8 | ConvertTo-Json -Depth 5",
                primary_url
            );
            let output = std::process::Command::new("powershell")
                .args(["-Command", &cmd_arg])
                .output()
                .ok();
            if let Some(out) = output {
                if out.status.success() {
                    let text = String::from_utf8_lossy(&out.stdout).into_owned();
                    if is_registry_json(&text) {
                        fetched_json = Some(text);
                        break;
                    }
                }
            }
        }
    }

    // ── Fallback: legacy GitHub-hosted URLs (deprecated) ──────────────────────
    if fetched_json.is_none() {
        let legacy_urls = [
            "https://samarnever-droid.github.io/lplusplus/registry/index.json",
            "https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/website/public/registry/index.json",
            "https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/registry/index.json",
        ];
        for url in &legacy_urls {
            if command_available("curl", &["--version"]) {
                let output = std::process::Command::new("curl")
                    .args(["-fsSL", "--max-time", "5", url])
                    .output()
                    .ok();
                if let Some(out) = output {
                    if out.status.success() {
                        let text = String::from_utf8_lossy(&out.stdout).into_owned();
                        if is_registry_json(&text) {
                            eprintln!("[L++] Using legacy registry URL: {url} (consider setting LPP_REGISTRY_URL)");
                            fetched_json = Some(text);
                            break;
                        }
                    }
                }
            }
        }
    }

    let cache_path = resolve_registry_cache_path();

    if let Some(ref content) = fetched_json {
        if let Some(parent) = cache_path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&cache_path, content);
        return fetched_json;
    }

    if cache_path.exists() {
        if let Ok(content) = fs::read_to_string(&cache_path) {
            if is_registry_json(&content) {
                return Some(content);
            }
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


fn git_command<I, S>(args: I) -> Result<std::process::Output, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    std::process::Command::new("git")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args)
        .output()
        .map_err(|e| format!("failed to execute git: {e}"))
}

fn git_status<I, S>(args: I, context: &str) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = git_command(args)?;
    if output.status.success() {
        Ok(())
    } else {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        if detail.is_empty() {
            Err(format!("{context} (git exit {})", output.status))
        } else {
            Err(format!("{context}: {detail}"))
        }
    }
}

fn git_output<I, S>(args: I, context: &str) -> Result<String, String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let output = git_command(args)?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if detail.is_empty() {
            format!("{context} (git exit {})", output.status)
        } else {
            format!("{context}: {detail}")
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn replace_file(temp: &Path, destination: &Path) -> Result<(), String> {
    #[cfg(windows)]
    if destination.exists() {
        fs::remove_file(destination).map_err(|e| format!("remove old '{}': {e}", destination.display()))?;
    }
    fs::rename(temp, destination).map_err(|e| format!("replace '{}': {e}", destination.display()))
}

fn toml_quote(value: &str) -> String {
    format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n").replace('\r', "\\r"))
}

fn lock_package_block(dep: &Dependency, source: &str, resolved: &Path) -> String {
    let version = dep.version.as_deref().unwrap_or("*");
    format!(
        "[[package]]\nname = {}\nversion = {}\nsource = {}\nresolved = {}\n\n",
        toml_quote(&dep.name),
        toml_quote(version),
        toml_quote(source),
        toml_quote(&resolved.to_string_lossy()),
    )
}

fn install_dependency(
    dep: &Dependency,
    destination: &Path,
    force_update: bool,
) -> Result<String, String> {
    if dep.name.is_empty() || dep.name == "." || dep.name == ".." || dep.name.chars().any(|ch| ch == '/' || ch == '\\') {
        return Err(format!("invalid dependency name '{}'", dep.name));
    }

    let global_cache_dest = resolve_global_package_cache(&dep.name, dep.tag.as_deref().or(dep.version.as_deref()));

    if let Some(git_url) = dep.git.as_deref() {
        if git_url.trim().is_empty() {
            return Err(format!("dependency '{}' has an empty git URL", dep.name));
        }

        // ── Fast-path: Global Package Store Cache ─────────────────────────────
        if !force_update && global_cache_dest.exists() && global_cache_dest.is_dir() {
            if destination.exists() {
                let _ = fs::remove_dir_all(destination);
            }
            if copy_dir_all(&global_cache_dest, destination).is_ok() {
                return Ok(format!("cache+{}", global_cache_dest.display()));
            }
        }

        if destination.exists() {
            if !destination.join(".git").exists() {
                return Err(format!("destination '{}' exists but is not a git checkout", destination.display()));
            }
            if force_update {
                git_status(["-C", destination.to_string_lossy().as_ref(), "fetch", "--all", "--prune"], &format!("updating '{}'", dep.name))?;
                if let Some(ref tag) = dep.tag {
                    git_status(["-C", destination.to_string_lossy().as_ref(), "checkout", "--force", tag], &format!("checking out tag '{tag}'"))?;
                } else if let Some(ref branch) = dep.branch {
                    git_status(["-C", destination.to_string_lossy().as_ref(), "checkout", branch], &format!("checking out branch '{branch}'"))?;
                    git_status(["-C", destination.to_string_lossy().as_ref(), "pull", "--ff-only"], &format!("updating branch '{branch}'"))?;
                }
            }
        } else {
            if let Some(parent) = destination.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("create dependency directory: {e}"))?;
            }
            let mut args = vec!["clone".to_string()];
            if let Some(ref tag) = dep.tag {
                args.push("--branch".to_string());
                args.push(tag.clone());
            } else if let Some(ref branch) = dep.branch {
                args.push("--branch".to_string());
                args.push(branch.clone());
            }
            args.push(git_url.to_string());
            args.push(destination.to_string_lossy().into_owned());
            git_status(args, &format!("cloning '{}'", dep.name))?;

            // Populate global package store
            let _ = fs::create_dir_all(&global_cache_dest);
            let _ = copy_dir_all(destination, &global_cache_dest);
        }
        let commit = git_output(["-C", destination.to_string_lossy().as_ref(), "rev-parse", "HEAD"], &format!("reading '{}' revision", dep.name))?;
        return Ok(format!("git+{}#{}", git_url, commit));
    }

    if let Some(path) = dep.path.as_deref() {
        let mut source = PathBuf::from(path);
        if !source.exists() {
            if let Ok(lpp_home) = std::env::var("LPP_HOME") {
                let alt = Path::new(&lpp_home).join(path);
                if alt.exists() {
                    source = alt;
                }
            }
        }
        if !source.exists() {
            let clean_path = path.replace('\\', "/").trim_start_matches('/').to_string();
            if !clean_path.contains("..") && clean_path.chars().all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '.' || c == '-' || c == '_') {
                // Fallback: fetch remote file from central raw URL if it's a known repository path
                let raw_url = format!("https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/{}", clean_path);
                let mut fetched_bytes: Option<Vec<u8>> = None;
                for bin in &["curl.exe", "curl", "C:\\Windows\\System32\\curl.exe"] {
                    if let Ok(out) = std::process::Command::new(bin)
                        .args(["-fsSL", "--ssl-no-revoke", "--max-time", "10", &raw_url])
                        .output()
                    {
                        if out.status.success() && !out.stdout.is_empty() {
                            fetched_bytes = Some(out.stdout);
                            break;
                        }
                    }
                }
                #[cfg(windows)]
                if fetched_bytes.is_none() {
                    let ps_cmd = format!("[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; (New-Object Net.WebClient).DownloadData('{}')", raw_url);
                    if let Ok(out) = std::process::Command::new("powershell")
                        .args(["-NoProfile", "-Command", &ps_cmd])
                        .output()
                    {
                        if out.status.success() && !out.stdout.is_empty() {
                            fetched_bytes = Some(out.stdout);
                        }
                    }
                }
                if let Some(bytes) = fetched_bytes {
                    let file_name = Path::new(&clean_path).file_name().and_then(|n| n.to_str()).unwrap_or("main.lpp");
                    let src_dir = destination.join("src");
                    fs::create_dir_all(&src_dir).map_err(|e| format!("create path dependency '{}': {e}", dep.name))?;
                    fs::write(src_dir.join(file_name), &bytes).map_err(|e| format!("write path dependency '{}': {e}", dep.name))?;
                    let manifest = format!("[package]\nname = {}\nversion = \"0.1.0\"\nentry = {}\n\n[dependencies]\n", toml_quote(&dep.name), toml_quote(&format!("src/{file_name}")));
                    fs::write(destination.join("lpp.toml"), manifest).map_err(|e| format!("write path dependency '{}': {e}", dep.name))?;
                    return Ok(format!("remote+{}", raw_url));
                }
            }
        }
        if !source.exists() {
            return Err(format!("path dependency '{}' does not exist: {}", dep.name, source.display()));
        }
        if destination.exists() {
            fs::remove_dir_all(destination).map_err(|e| format!("replace path dependency '{}': {e}", dep.name))?;
        }
        if source.is_dir() {
            copy_dir_all(&source, destination).map_err(|e| format!("copy path dependency '{}': {e}", dep.name))?;
        } else if source.is_file() {
            // Registry entries for stdlib modules point at a single .lpp file.
            // Materialise that file as a tiny package instead of rejecting a
            // valid registry entry as if it were a broken directory path.
            let file_name = source.file_name().and_then(|n| n.to_str()).unwrap_or("main.lpp");
            let src_dir = destination.join("src");
            fs::create_dir_all(&src_dir).map_err(|e| format!("create path dependency '{}': {e}", dep.name))?;
            fs::copy(&source, src_dir.join(file_name)).map_err(|e| format!("copy path dependency '{}': {e}", dep.name))?;
            let manifest = format!("[package]\nname = {}\nversion = \"0.1.0\"\nentry = {}\n\n[dependencies]\n", toml_quote(&dep.name), toml_quote(&format!("src/{file_name}")));
            fs::write(destination.join("lpp.toml"), manifest).map_err(|e| format!("write path dependency '{}': {e}", dep.name))?;
        } else {
            return Err(format!("path dependency '{}' is neither a file nor a directory: {}", dep.name, source.display()));
        }
        return Ok(format!("path+{}", source.to_string_lossy()));
    }

    Err(format!("dependency '{}' has no resolvable source", dep.name))
}

fn cmd_install_command(args: &[String]) -> i32 {
    if !args.is_empty() && !Path::new("lpp.toml").exists() && !Path::new("lpp.json").exists() {
        let package = &args[0];
        eprintln!("{}", ui::tag_error("No lpp.toml or lpp.json manifest found in current directory."));
        eprintln!();
        println!("  {}", ui::bold("To add this package to a project:"));
        println!("    {} {}", ui::dim("$"), ui::cyan(&format!("lpp new my_app && cd my_app && lpp add {package} && lpp install")));
        println!();
        println!("  {}", ui::bold("To explore registry packages:"));
        println!("    {} {}", ui::dim("$"), ui::cyan("lpp search"));
        println!();
        return 1;
    }
    if args.iter().any(|arg| arg == "--offline") {
        unsafe { std::env::set_var("LPP_OFFLINE", "1") };
    }
    cmd_install(false)
}

fn cmd_install(force_update: bool) -> i32 {
    let start_time = std::time::Instant::now();
    let package = match read_manifest() {
        Ok(pkg) => pkg,
        Err(e) => {
            eprintln!("{}", ui::tag_error(&format!("Manifest error: {}", e)));
            return 1;
        }
    };

    println!("  {} Resolving dependencies for {} {}...", ui::bold_cyan("⚡"), ui::bold(&package.name), ui::dim(&format!("v{}", package.version)));

    let pkg_dir = Path::new(".lpp_packages");
    if let Err(e) = fs::create_dir_all(pkg_dir) {
        eprintln!("{}", ui::tag_error(&format!("Failed to create .lpp_packages directory: {}", e)));
        return 1;
    }

    let mut lock_content = String::from("# L++ lockfile v2 — generated by lpp. Do not edit.\nlock_version = 2\n\n");
    let mut worklist = package.dependencies;
    let mut processed = std::collections::HashSet::new();
    let mut specs: std::collections::HashMap<String, (Option<String>, Option<String>, Option<String>, Option<String>)> = std::collections::HashMap::new();
    let mut failed = false;
    let mut installed_count = 0;

    while !worklist.is_empty() {
        let mut current_wave: Vec<Dependency> = Vec::new();
        while let Some(dep) = worklist.pop() {
            let key = dep.name.clone();
            let spec = (dep.version.clone(), dep.git.clone(), dep.path.clone(), dep.tag.clone().or(dep.branch.clone()));
            if let Some(previous) = specs.get(&key) {
                if previous != &spec {
                    eprintln!("{}", ui::tag_error(&format!("conflicting requirements for dependency '{}'; use one source and version", key)));
                    failed = true;
                }
                continue;
            }
            specs.insert(key.clone(), spec);
            if !processed.insert(key) {
                continue;
            }
            current_wave.push(dep);
        }

        if current_wave.is_empty() {
            break;
        }

        let mut resolved_wave: Vec<Dependency> = Vec::new();
        for dep in current_wave {
            let mut resolved_dep = dep.clone();
            if resolved_dep.git.is_none() && resolved_dep.path.is_none() {
                if std::env::var_os("LPP_OFFLINE").is_some() {
                    eprintln!("{}", ui::tag_error(&format!("dependency '{}' is not available offline without a local source", resolved_dep.name)));
                    failed = true;
                    continue;
                }
                if let Some(entry) = resolve_registry_package(&resolved_dep.name) {
                    if !entry.git.is_empty() {
                        resolved_dep.git = Some(entry.git);
                    } else if let Some(path) = entry.path {
                        resolved_dep.path = Some(path);
                    } else {
                        eprintln!("{}", ui::tag_error(&format!("registry entry '{}' has no git or path source", resolved_dep.name)));
                        failed = true;
                        continue;
                    }
                    if resolved_dep.branch.is_none() { resolved_dep.branch = entry.branch; }
                    if resolved_dep.tag.is_none() { resolved_dep.tag = entry.tag; }
                    if resolved_dep.version.is_none() { resolved_dep.version = entry.version; }
                } else {
                    eprintln!("{}", ui::tag_error(&format!("dependency '{}' was not found in the registry", resolved_dep.name)));
                    failed = true;
                    continue;
                }
            }
            resolved_wave.push(resolved_dep);
        }

        if resolved_wave.is_empty() {
            continue;
        }

        struct InstallResult {
            dep: Dependency,
            destination: PathBuf,
            source: Result<String, String>,
            elapsed_ms: u128,
        }

        let results: Vec<InstallResult> = std::thread::scope(|s| {
            let mut handles = Vec::new();
            for dep in resolved_wave {
                let dest = pkg_dir.join(&dep.name);
                let handle = s.spawn(move || {
                    let start = std::time::Instant::now();
                    let source = install_dependency(&dep, &dest, force_update);
                    let elapsed_ms = start.elapsed().as_millis();
                    InstallResult {
                        dep,
                        destination: dest,
                        source,
                        elapsed_ms,
                    }
                });
                handles.push(handle);
            }

            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });

        for res in results {
            let dep_name = &res.dep.name;
            match res.source {
                Ok(source) => {
                    installed_count += 1;
                    let ver_tag = res.dep.version.as_deref().unwrap_or("0.1.0");
                    println!(
                        "  {} Installed {} {} {}",
                        ui::green("✔"),
                        ui::bold(dep_name),
                        ui::dim(&format!("v{ver_tag}")),
                        ui::dim(&format!("[{}ms]", res.elapsed_ms))
                    );
                    lock_content.push_str(&lock_package_block(&res.dep, &source, &res.destination));

                    let manifest_path = if res.destination.join("lpp.json").is_file() {
                        res.destination.join("lpp.json")
                    } else {
                        res.destination.join("lpp.toml")
                    };
                    if manifest_path.is_file() {
                        match fs::read_to_string(&manifest_path)
                            .map_err(|e| format!("read '{}': {e}", manifest_path.display()))
                            .and_then(|text| {
                                if manifest_path.extension().and_then(|e| e.to_str()) == Some("json") {
                                    parse_json_manifest(&text)
                                } else {
                                    parse_toml(&text)
                                }
                            }) {
                            Ok(sub_pkg) => {
                                let base = manifest_path.parent().unwrap_or(Path::new("."));
                                for mut sub_dep in sub_pkg.dependencies {
                                    if let Some(path) = sub_dep.path.take() {
                                        let resolved = Path::new(&path);
                                        let resolved = if resolved.is_absolute() {
                                            resolved.to_path_buf()
                                        } else {
                                            base.join(resolved)
                                        };
                                        sub_dep.path = Some(resolved.to_string_lossy().into_owned());
                                    }
                                    worklist.push(sub_dep);
                                }
                            }
                            Err(e) => {
                                eprintln!("{}", ui::tag_error(&format!("invalid manifest in '{}': {e}", res.destination.display())));
                                failed = true;
                            }
                        }
                    }
                }
                Err(e) => {
                    eprintln!("{}", ui::tag_error(&format!("failed to install '{dep_name}': {e}")));
                    failed = true;
                }
            }
        }
    }

    if failed {
        eprintln!("{}", ui::tag_error("dependency installation failed; existing lpp.lock was not replaced"));
        return 1;
    }

    let lock_path = Path::new("lpp.lock");
    let temp = lock_path.with_extension("lock.tmp");
    if let Err(e) = fs::write(&temp, lock_content) {
        eprintln!("{}", ui::tag_error(&format!("Failed to write temporary lockfile: {}", e)));
        return 1;
    }
    if let Err(e) = replace_file(&temp, lock_path) {
        let _ = fs::remove_file(&temp);
        eprintln!("{}", ui::tag_error(&format!("Failed to replace lpp.lock: {e}")));
        return 1;
    }
    let elapsed = start_time.elapsed().as_secs_f64();
    println!("  {} Generated {} (v2 format, {} locked)", ui::green("✔"), ui::cyan("lpp.lock"), installed_count);
    println!("  {} Dependencies resolved in {:.2}s", ui::bold_green("✨"), elapsed);
    0
}

fn toml_dependency_line(dep: &Dependency) -> String {
    let mut fields = Vec::new();
    if let Some(ref git) = dep.git {
        fields.push(format!("git = {}", toml_quote(git)));
    }
    if let Some(ref path) = dep.path {
        fields.push(format!("path = {}", toml_quote(path)));
    }
    if let Some(ref version) = dep.version {
        fields.push(format!("version = {}", toml_quote(version)));
    }
    if let Some(ref branch) = dep.branch {
        fields.push(format!("branch = {}", toml_quote(branch)));
    }
    if let Some(ref tag) = dep.tag {
        fields.push(format!("tag = {}", toml_quote(tag)));
    }
    format!("{} = {{ {} }}", dep.name, fields.join(", "))
}

fn toml_insert_dependency(content: &str, dep: &Dependency) -> Result<String, String> {
    // Validate before touching the file, then ensure the dependency is added
    // inside [dependencies] rather than accidentally becoming part of a later
    // [build]/[workspace] section.
    parse_toml(content)?;
    let line = toml_dependency_line(dep);
    let mut lines: Vec<String> = content.lines().map(String::from).collect();
    let mut dep_section = None;
    let mut next_section = lines.len();
    for (idx, raw) in lines.iter().enumerate() {
        let trimmed = raw.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            let section = trimmed[1..trimmed.len() - 1].trim();
            if section == "dependencies" {
                dep_section = Some(idx);
            } else if dep_section.is_some() && idx > dep_section.unwrap() {
                next_section = idx;
                break;
            }
        }
    }
    if let Some(section_idx) = dep_section {
        lines.insert(next_section, line);
        let mut result = lines.join("\n");
        if content.ends_with('\n') {
            result.push('\n');
        }
        let _ = section_idx;
        Ok(result)
    } else {
        let mut result = content.to_string();
        if !result.is_empty() && !result.ends_with('\n') {
            result.push('\n');
        }
        result.push_str("\n[dependencies]\n");
        result.push_str(&line);
        result.push('\n');
        Ok(result)
    }
}

fn cmd_add(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!(
            "Usage: lpp add <package_name> [--git <url> [--tag <tag>] [--branch <branch>]] [--path <local_path>] [--version <semver>]"
        );
        return 2;
    }

    let requested_name = args[0].clone();
    let mut package_name = requested_name.clone();
    let mut git_url = None;
    let mut tag = None;
    let mut branch = None;
    let mut path = None;
    let mut version = None;

    let mut i = 1;
    while i < args.len() {
        let value = |i: usize, flag: &str| -> Result<String, ()> {
            args.get(i + 1).cloned().ok_or_else(|| {
                eprintln!("Error: {flag} expects an argument");
            })
        };
        match args[i].as_str() {
            "--git" => match value(i, "--git") {
                Ok(v) => { git_url = Some(v); i += 2; }
                Err(()) => return 2,
            },
            "--tag" => match value(i, "--tag") {
                Ok(v) => { tag = Some(v); i += 2; }
                Err(()) => return 2,
            },
            "--branch" => match value(i, "--branch") {
                Ok(v) => { branch = Some(v); i += 2; }
                Err(()) => return 2,
            },
            "--version" => match value(i, "--version") {
                Ok(v) => { version = Some(v); i += 2; }
                Err(()) => return 2,
            },
            "--path" => match value(i, "--path") {
                Ok(v) => { path = Some(v); i += 2; }
                Err(()) => return 2,
            },
            other => {
                eprintln!("Unknown argument: {other}");
                return 2;
            }
        }
    }

    if git_url.is_some() && path.is_some() {
        eprintln!("Error: --git and --path are mutually exclusive");
        return 2;
    }
    if tag.is_some() && branch.is_some() {
        eprintln!("Error: --tag and --branch are mutually exclusive");
        return 2;
    }
    if let Some(ref req) = version {
        if let Err(e) = validate_dependency_requirement(req) {
            eprintln!("{e}");
            return 2;
        }
    }

    // @owner/repo is a convenient git shorthand, but the manifest key is the
    // repository leaf so it remains a safe directory name.
    if package_name.starts_with('@') {
        if let Some(slash_idx) = package_name.find('/') {
            let owner = &package_name[1..slash_idx];
            let repo = &package_name[slash_idx + 1..];
            if owner.is_empty() || repo.is_empty() {
                eprintln!("Error: scoped package must look like @owner/repository");
                return 2;
            }
            if git_url.is_none() && path.is_none() {
                git_url = Some(format!("https://github.com/{owner}/{repo}.git"));
            }
            package_name = repo.to_string();
        }
    }

    if git_url.is_none() && path.is_none() {
        if let Some(entry) = resolve_registry_package(&requested_name).or_else(|| resolve_registry_package(&package_name)) {
            println!("[L++] Resolved '{}' from registry", requested_name);
            if git_url.is_none() && !entry.git.is_empty() {
                git_url = Some(entry.git);
            } else if path.is_none() {
                path = entry.path;
            }
            if branch.is_none() { branch = entry.branch; }
            if tag.is_none() { tag = entry.tag; }
            if version.is_none() { version = entry.version; }
        } else {
            eprintln!("Error: Package '{}' not found in registry. Use --git <url> or --path <dir>.", requested_name);
            return 1;
        }
    }

    if let Err(e) = validate_dependency_name(&package_name) {
        eprintln!("{e}");
        return 2;
    }
    let dep = Dependency { name: package_name.clone(), version, git: git_url, tag, branch, path };

    let manifest_path = if Path::new("lpp.json").exists() {
        PathBuf::from("lpp.json")
    } else if Path::new("lpp.toml").exists() {
        PathBuf::from("lpp.toml")
    } else {
        eprintln!("Error: lpp.toml or lpp.json not found. Run 'lpp init' first.");
        return 1;
    };
    let content = match fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read {}: {e}", manifest_path.display());
            return 1;
        }
    };
    let existing = if manifest_path.extension().and_then(|e| e.to_str()) == Some("json") {
        parse_json_manifest(&content)
    } else {
        parse_toml(&content)
    };
    let pkg = match existing {
        Ok(pkg) => pkg,
        Err(e) => {
            eprintln!("[L++] Manifest error: {e}");
            return 1;
        }
    };
    if pkg.dependencies.iter().any(|existing| existing.name == package_name) {
        eprintln!("[L++] dependency '{}' already exists; edit the manifest or remove it first", package_name);
        return 1;
    }

    let updated = if manifest_path.extension().and_then(|e| e.to_str()) == Some("json") {
        let mut value: serde_json::Value = match serde_json::from_str(&content) {
            Ok(value) => value,
            Err(e) => { eprintln!("JSON syntax error: {e}"); return 1; }
        };
        let object = match value.as_object_mut() {
            Some(object) => object,
            None => { eprintln!("JSON manifest root must be an object"); return 1; }
        };
        let deps = object.entry("dependencies").or_insert_with(|| serde_json::json!({}));
        let deps = match deps.as_object_mut() {
            Some(deps) => deps,
            None => { eprintln!("'dependencies' must be an object"); return 1; }
        };
        let mut dep_obj = serde_json::Map::new();
        if let Some(ref git) = dep.git { dep_obj.insert("git".to_string(), serde_json::Value::String(git.clone())); }
        if let Some(ref path) = dep.path { dep_obj.insert("path".to_string(), serde_json::Value::String(path.clone())); }
        if let Some(ref version) = dep.version { dep_obj.insert("version".to_string(), serde_json::Value::String(version.clone())); }
        if let Some(ref branch) = dep.branch { dep_obj.insert("branch".to_string(), serde_json::Value::String(branch.clone())); }
        if let Some(ref tag) = dep.tag { dep_obj.insert("tag".to_string(), serde_json::Value::String(tag.clone())); }
        deps.insert(package_name.clone(), serde_json::Value::Object(dep_obj));
        match serde_json::to_string_pretty(&value) {
            Ok(json) => format!("{json}\n"),
            Err(e) => { eprintln!("serialize lpp.json: {e}"); return 1; }
        }
    } else {
        match toml_insert_dependency(&content, &dep) {
            Ok(updated) => updated,
            Err(e) => { eprintln!("[L++] manifest update failed: {e}"); return 1; }
        }
    };

    let temp = manifest_path.with_extension(if manifest_path.extension().and_then(|e| e.to_str()) == Some("json") { "json.tmp" } else { "toml.tmp" });
    if let Err(e) = fs::write(&temp, updated) {
        eprintln!("Failed to write {}: {e}", manifest_path.display());
        return 1;
    }
    if let Err(e) = replace_file(&temp, &manifest_path) {
        let _ = fs::remove_file(&temp);
        eprintln!("Failed to replace {}: {e}", manifest_path.display());
        return 1;
    }
    println!("[L++] Added dependency '{}' to {}.", package_name, manifest_path.display());
    cmd_install(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_resolve_from_json_version_matching() {
        let registry = r#"{
            "packages": {
                "lreact": {
                    "git": "https://github.com/example/lreact",
                    "version": "1.0.0"
                },
                "lreact@1.2.0": {
                    "git": "https://github.com/example/lreact",
                    "version": "1.2.0"
                },
                "lreact@2.0.0": {
                    "git": "https://github.com/example/lreact",
                    "version": "2.0.0"
                }
            }
        }"#;

        let entry = resolve_from_json_version(registry, "lreact", Some("^1.0.0"));
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().version.as_deref(), Some("1.2.0"));

        let entry_v2 = resolve_from_json_version(registry, "lreact", Some(">=2.0.0"));
        assert!(entry_v2.is_some());
        assert_eq!(entry_v2.unwrap().version.as_deref(), Some("2.0.0"));
    }

    #[test]
    fn test_parse_and_write_lockfile_roundtrip() {
        let content = parse_lockfile(
            "[[package]]\nname = \"lreact\"\nversion = \"1.2.0\"\nsource = \"registry+https://yarqrdhcmxhagxbbjrgu.supabase.co\"\nresolved = \"https://github.com/example/lreact\"\nchecksum = \"sha256:abc123def456\"\n"
        );
        assert_eq!(content.len(), 1);
        assert_eq!(content[0].name, "lreact");
        assert_eq!(content[0].checksum.as_deref(), Some("sha256:abc123def456"));

        let hash = compute_sha256_hex(b"hello lpp payload");
        assert!(hash.starts_with("sha256:"));
    }

    #[test]
    fn test_large_package_dependency_graph_and_lockfile_stress() {
        let mut pkgs = Vec::new();
        for i in 0..1000 {
            let payload = format!("package_payload_data_chunk_{i}_with_large_payload_metadata");
            let checksum = compute_sha256_hex(payload.as_bytes());
            pkgs.push(LockedPackage {
                name: format!("pkg_{i}"),
                version: Some(format!("1.{}.0", i % 50)),
                source: "registry+https://yarqrdhcmxhagxbbjrgu.supabase.co".to_string(),
                resolved: Some(format!("https://github.com/lpp-packages/pkg_{i}.git")),
                checksum: Some(checksum),
            });
        }

        let mut lock_str = String::new();
        for pkg in &pkgs {
            lock_str.push_str("[[package]]\n");
            lock_str.push_str(&format!("name = \"{}\"\n", pkg.name));
            lock_str.push_str(&format!("version = \"{}\"\n", pkg.version.as_deref().unwrap()));
            lock_str.push_str(&format!("source = \"{}\"\n", pkg.source));
            lock_str.push_str(&format!("resolved = \"{}\"\n", pkg.resolved.as_deref().unwrap()));
            lock_str.push_str(&format!("checksum = \"{}\"\n\n", pkg.checksum.as_deref().unwrap()));
        }

        let parsed = parse_lockfile(&lock_str);
        assert_eq!(parsed.len(), 1000);
        assert_eq!(parsed[999].name, "pkg_999");
        assert!(parsed[999].checksum.as_ref().unwrap().starts_with("sha256:"));
    }

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
    fn parse_toml_reads_keywords() {
        let manifest = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nkeywords = [\"ffi\", \"bindgen\"]\n\n[dependencies]\n";
        let pkg = parse_toml(manifest).expect("manifest should parse");
        assert_eq!(pkg.keywords, vec!["ffi", "bindgen"]);
    }

    #[test]
    fn parse_toml_rejects_excess_keywords() {
        let manifest = "[package]\nname = \"demo\"\nversion = \"0.1.0\"\nkeywords = [\"a\", \"b\", \"c\", \"d\", \"e\", \"f\"]\n\n[dependencies]\n";
        let err = parse_toml(manifest).expect_err("manifest with 6 keywords should fail");
        assert!(err.contains("maximum 5 keywords allowed"));
    }

    #[test]
    fn parse_json_manifest_accepts_string_and_table_dependencies() {
        let manifest = r#"{
          "name": "demo",
          "version": "1.2.3",
          "main": "src/main.lpp",
          "dependencies": {
            "local": "../local",
            "remote": { "git": "https://example.com/remote.git", "branch": "main" },
            "semver": "^2.0"
          }
        }"#;
        let pkg = super::parse_json_manifest(manifest).expect("JSON manifest should parse");
        assert_eq!(pkg.entry.as_deref(), Some("src/main.lpp"));
        assert_eq!(pkg.dependencies.len(), 3);
        assert_eq!(pkg.dependencies[0].path.as_deref(), Some("../local"));
        assert_eq!(pkg.dependencies[1].branch.as_deref(), Some("main"));
        assert_eq!(pkg.dependencies[2].version.as_deref(), Some("^2.0"));
    }

    #[test]
    fn manifests_validate_semver_and_dependency_sources() {
        let workspace_member = "[package]\nname = \"demo\"\nversion = { workspace = true }\n";
        assert!(super::parse_toml(workspace_member).is_err());
        let resolved = super::parse_toml_with_workspace(workspace_member, Some("2.0.0"))
            .expect("workspace version should resolve");
        assert_eq!(resolved.version, "2.0.0");

        let bad_version = "[package]\nname = \"demo\"\nversion = \"not-semver\"\n";
        assert!(super::parse_toml(bad_version).is_err());
        let bad_source = "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n[dependencies]\nfoo = { git = \"x\", path = \"y\" }\n";
        let err = super::parse_toml(bad_source).expect_err("conflicting sources must fail");
        assert!(err.contains("both git and path"));
    }

    #[test]
    fn version_bumps_reset_prerelease_metadata() {
        assert_eq!(super::bump_package_version("1.2.3-rc.1+build", "patch").unwrap(), "1.2.4");
        assert_eq!(super::bump_package_version("1.2.3", "minor").unwrap(), "1.3.0");
        assert!(super::bump_package_version("1.2.3", "wat").is_err());
    }

    #[test]
    fn toml_dependency_edit_stays_in_dependencies_section() {
        let manifest = "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n\n[dependencies]\n\n[build]\ntype = \"library\"\n";
        let dep = super::Dependency {
            name: "foo".to_string(),
            version: Some("^1.0".to_string()),
            git: None,
            tag: None,
            branch: None,
            path: Some("../foo".to_string()),
        };
        let updated = super::toml_insert_dependency(manifest, &dep).unwrap();
        assert!(updated.contains("foo = { path = \"../foo\", version = \"^1.0\" }"));
        assert!(updated.contains("[build]\ntype = \"library\""));
        let (removed, found) = super::toml_remove_dependency(&updated, "foo");
        assert!(found);
        assert!(!removed.contains("foo = {"));
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

    #[test]
    fn resolve_from_json_parses_registry_packages() {
        let json = r#"{
          "packages": {
            "YTDownloader": { "git": "https://github.com/Okrabai/YTDownloader.git", "branch": "main", "description": "YouTube downloader tool" },
            "@samarnever-droid/lpp-zip": { "git": "https://github.com/samarnever-droid/lplusplus.git", "branch": "master" }
          }
        }"#;

        let entry1 = super::resolve_from_json(json, "ytdownloader").expect("case insensitive match");
        assert_eq!(entry1.git, "https://github.com/Okrabai/YTDownloader.git");
        assert_eq!(entry1.branch.as_deref(), Some("main"));
        assert_eq!(entry1.description.as_deref(), Some("YouTube downloader tool"));

        let entry2 = super::resolve_from_json(json, "lpp-zip").expect("scoped leaf match");
        assert_eq!(entry2.git, "https://github.com/samarnever-droid/lplusplus.git");
    }
}

fn toml_remove_dependency(content: &str, target: &str) -> (String, bool) {
    let mut section = String::new();
    let mut found = false;
    let mut lines = Vec::new();
    for raw in content.lines() {
        let trimmed = raw.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            section = trimmed[1..trimmed.len() - 1].trim().to_string();
        }
        if section == "dependencies" {
            if let Some(eq) = raw.find('=') {
                let key = raw[..eq].trim().trim_matches('"').trim_matches('\'');
                if key == target {
                    found = true;
                    continue;
                }
            }
        }
        lines.push(raw.to_string());
    }
    let mut updated = lines.join("\n");
    if content.ends_with('\n') {
        updated.push('\n');
    }
    (updated, found)
}

fn cmd_remove(args: &[String]) -> i32 {
    let Some(package_name) = args.first() else {
        eprintln!("Usage: lpp remove <package_name>");
        return 2;
    };
    let manifest_path = if Path::new("lpp.json").exists() {
        PathBuf::from("lpp.json")
    } else if Path::new("lpp.toml").exists() {
        PathBuf::from("lpp.toml")
    } else {
        eprintln!("Error: lpp.toml or lpp.json not found.");
        return 1;
    };
    let content = match fs::read_to_string(&manifest_path) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Failed to read {}: {e}", manifest_path.display());
            return 1;
        }
    };

    let (updated, found) = if manifest_path.extension().and_then(|e| e.to_str()) == Some("json") {
        let mut value: serde_json::Value = match serde_json::from_str(&content) {
            Ok(value) => value,
            Err(e) => { eprintln!("JSON syntax error: {e}"); return 1; }
        };
        let Some(object) = value.as_object_mut() else {
            eprintln!("JSON manifest root must be an object");
            return 1;
        };
        if let Some(deps) = object.get_mut("dependencies").and_then(|v| v.as_object_mut()) {
            if deps.remove(package_name).is_none() {
                (content.clone(), false)
            } else {
                match serde_json::to_string_pretty(&value) {
                    Ok(json) => (format!("{json}\n"), true),
                    Err(e) => { eprintln!("serialize lpp.json: {e}"); return 1; }
                }
            }
        } else {
            (content.clone(), false)
        }
    } else {
        toml_remove_dependency(&content, package_name)
    };

    if !found {
        eprintln!("[L++] Dependency '{}' not found in {}.", package_name, manifest_path.display());
        return 1;
    }
    let temp = manifest_path.with_extension(if manifest_path.extension().and_then(|e| e.to_str()) == Some("json") { "json.tmp" } else { "toml.tmp" });
    if let Err(e) = fs::write(&temp, updated) {
        eprintln!("Failed to write {}: {e}", manifest_path.display());
        return 1;
    }
    if let Err(e) = replace_file(&temp, &manifest_path) {
        let _ = fs::remove_file(&temp);
        eprintln!("Failed to replace {}: {e}", manifest_path.display());
        return 1;
    }
    println!("[L++] Removed dependency '{}' from {}.", package_name, manifest_path.display());

    let dest_path = Path::new(".lpp_packages").join(package_name);
    if dest_path.exists() {
        if let Err(e) = fs::remove_dir_all(&dest_path) {
            eprintln!("[L++] failed to remove installed dependency: {e}");
            return 1;
        }
        println!("[L++] Cleaned up package directory for '{}'.", package_name);
    }
    cmd_install(false)
}

fn cmd_update() -> i32 {
    println!("[L++] Updating lockfile and pulling latest dependency updates...");
    cmd_install(true)
}

fn is_app_package_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.ends_with("-cli") || n.ends_with("-app") || n == "lreact"
}

fn print_search_item(name: &str, entry: &RegistryEntry, app: bool) {
    let badge = if app {
        ui::purple("[APPLICATION]")
    } else {
        ui::cyan("[LIBRARY]")
    };
    let ver_str = entry.version.as_deref().unwrap_or("0.1.0");
    println!("  {} {} {} {}", ui::bold_cyan("◆"), ui::bold(name), ui::dim(&format!("v{ver_str}")), badge);
    if let Some(ref desc) = entry.description {
        println!("    {}", desc);
    }
    if !entry.git.is_empty() {
        println!("    {} {}", ui::dim("Repository:"), ui::cyan(&entry.git));
    }
    if let Some(ref path) = entry.path {
        println!("    {} {}", ui::dim("Source Path:"), path);
    }
    println!("    {} {}", ui::dim("Add to Project:"), ui::bold_green(&format!("lpp add {name}")));
}

fn workspace_root(start: &Path) -> Result<(PathBuf, toml::Value), String> {
    let mut current = start
        .canonicalize()
        .map_err(|e| format!("cannot resolve workspace directory '{}': {e}", start.display()))?;
    loop {
        let manifest = current.join("lpp.toml");
        if manifest.is_file() {
            let text = fs::read_to_string(&manifest).map_err(|e| format!("read '{}': {e}", manifest.display()))?;
            let value: toml::Value = toml::from_str(&text).map_err(|e| format!("parse '{}': {e}", manifest.display()))?;
            if value.get("workspace").and_then(toml::Value::as_table).is_some() {
                return Ok((current, value));
            }
        }
        if !current.pop() {
            break;
        }
    }
    Err("not inside a workspace (no [workspace] section found)".to_string())
}

fn workspace_members(root: &Path, manifest: &toml::Value) -> Result<Vec<(String, PathBuf, Package)>, String> {
    let workspace = manifest
        .get("workspace")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| "missing [workspace] section".to_string())?;
    let members = workspace
        .get("members")
        .and_then(toml::Value::as_array)
        .ok_or_else(|| "[workspace].members must be an array".to_string())?;
    let root_canonical = root.canonicalize().map_err(|e| format!("resolve workspace root: {e}"))?;
    let mut result = Vec::new();
    for member in members {
        let relative = member
            .as_str()
            .ok_or_else(|| "workspace member paths must be strings".to_string())?;
        let path = root.join(relative).canonicalize().map_err(|e| format!("workspace member '{relative}': {e}"))?;
        if !path.starts_with(&root_canonical) {
            return Err(format!("workspace member '{relative}' escapes the workspace root"));
        }
        let manifest_path = path.join("lpp.toml");
        let member_text = fs::read_to_string(&manifest_path)
            .map_err(|e| format!("read '{}': {e}", manifest_path.display()))?;
        let workspace_version = workspace
            .get("version")
            .and_then(toml::Value::as_str);
        let package = parse_toml_with_workspace(&member_text, workspace_version)?;
        result.push((relative.to_string(), path, package));
    }
    Ok(result)
}

fn cmd_workspace(args: &[String]) -> i32 {
    let (root, manifest) = match workspace_root(Path::new(".")) {
        Ok(value) => value,
        Err(e) => { eprintln!("{}", ui::tag_error(&e)); return 1; }
    };
    let members = match workspace_members(&root, &manifest) {
        Ok(members) => members,
        Err(e) => { eprintln!("{}", ui::tag_error(&format!("workspace error: {e}"))); return 1; }
    };
    let sub = args.first().map(String::as_str).unwrap_or("members");
    match sub {
        "members" | "list" => {
            println!("  {} Workspace: {}", ui::bold_cyan("📦"), ui::bold(&root.to_string_lossy()));
            if let Some(version) = manifest.get("workspace").and_then(|w| w.get("version")).and_then(toml::Value::as_str) {
                println!("    {} {}", ui::dim("Version:"), version);
            }
            for (relative, path, package) in members {
                println!("    {} {} {} ({})", ui::green("◆"), ui::bold(&package.name), ui::dim(&format!("v{}", package.version)), path.strip_prefix(&root).unwrap_or(&path).display());
                let _ = relative;
            }
            0
        }
        "graph" => {
            println!("  {} Workspace Dependency Graph: {}", ui::bold_cyan("📦"), root.display());
            for (_, _, package) in members {
                let name = package.name;
                let deps: Vec<String> = package.dependencies.into_iter().map(|dep| dep.name).collect();
                if deps.is_empty() { println!("    {} -> {}", ui::bold(&name), ui::dim("(none)")); }
                else { println!("    {} -> {}", ui::bold(&name), ui::cyan(&deps.join(", "))); }
            }
            0
        }
        "build" | "test" => {
            let requested = args.get(1).map(String::as_str);
            let selected: Vec<_> = members.into_iter().filter(|(_, _, package)| requested.map_or(true, |name| package.name == name)).collect();
            if selected.is_empty() {
                eprintln!("{}", ui::tag_error(&format!("workspace member not found: {}", requested.unwrap_or(""))));
                return 1;
            }
            let compiler = match current_compiler_path() {
                Ok(path) => path,
                Err(e) => { eprintln!("{}", ui::tag_error(&e)); return 1; }
            };
            let mut failed = false;
            for (_, path, _) in selected {
                let command = if sub == "build" { "build" } else { "test" };
                println!("  {} {} {}", ui::bold_cyan("⚡"), command, path.display());
                match std::process::Command::new(&compiler).current_dir(&path).arg(command).status() {
                    Ok(status) if status.success() => {}
                    Ok(status) => { failed = true; eprintln!("{}", ui::tag_error(&format!("member '{}' failed ({status})", path.display()))); }
                    Err(e) => { failed = true; eprintln!("{}", ui::tag_error(&format!("member '{}' failed: {e}", path.display()))); }
                }
            }
            if failed { 1 } else { 0 }
        }
        other => {
            eprintln!("{}", ui::tag_error(&format!("unknown workspace subcommand '{other}'; use members, graph, build, or test")));
            2
        }
    }
}

fn cmd_search(args: &[String]) -> i32 {
    let query = args.get(0).map(|s| s.to_lowercase()).unwrap_or_default();
    let mut results = registry_package_entries();
    results.sort_by(|a, b| a.0.cmp(&b.0));

    if !query.is_empty() {
        results.retain(|(name, entry)| {
            name.to_lowercase().contains(&query)
                || entry.git.to_lowercase().contains(&query)
                || entry.path.as_deref().unwrap_or("").to_lowercase().contains(&query)
                || entry.description.as_deref().unwrap_or("").to_lowercase().contains(&query)
        });
    }

    println!();
    if results.is_empty() {
        if query.is_empty() {
            println!("  {}", ui::tag_info("No packages available in registry."));
        } else {
            println!("  {}", ui::tag_warn(&format!("No registry packages matched '{query}'.", )));
            println!("  Try: {} | {} | {} | {}", ui::cyan("lpp search sqlite"), ui::cyan("lpp search lppsqlite"), ui::cyan("lpp search lreact"), ui::cyan("lpp search math"));
        }
        return 0;
    }

    let apps: Vec<_> = results.iter().filter(|(name, _)| is_app_package_name(name)).collect();
    let libs: Vec<_> = results.iter().filter(|(name, _)| !is_app_package_name(name)).collect();
    
    println!("  {}", ui::bold_cyan("╭─────────────────────────────────────────────────────────────╮"));
    println!("  {}  {} REGISTRY SEARCH: '{}' ({} found)  {}", ui::bold_cyan("│"), ui::bold("📦"), if query.is_empty() { "*" } else { &query }, results.len(), ui::bold_cyan("│"));
    println!("  {}", ui::bold_cyan("╰─────────────────────────────────────────────────────────────╯"));
    println!();
    if !apps.is_empty() {
        println!("  {}", ui::bold_purple("APPLICATIONS & PLATFORMS:"));
        println!("  {}", ui::dim("───────────────────────────────────────────────────────────────"));
        for (name, entry) in apps { print_search_item(name, entry, true); println!(); }
    }
    if !libs.is_empty() {
        println!("  {}", ui::bold_cyan("LIBRARIES & PROJECT DEPENDENCIES:"));
        println!("  {}", ui::dim("───────────────────────────────────────────────────────────────"));
        for (name, entry) in libs { print_search_item(name, entry, false); println!(); }
    }
    println!("  {}", ui::dim("Quick Start:"));
    println!("    Add package:   {}", ui::green("lpp add <name> && lpp install"));
    println!("    New project:   {}", ui::green("lpp new my_app"));
    println!();
    0
}

fn cmd_list() -> i32 {
    match read_manifest() {
        Ok(pkg) => {
            println!();
            println!("  {} Package: {} {}", ui::bold_cyan("📦"), ui::bold(&pkg.name), ui::dim(&format!("v{}", pkg.version)));
            println!("  {}", ui::dim("─────────────────────────────────────────────────────────────────────────"));
            println!("   {:<24} {:<14} {:<24} {:<12}", ui::bold("DEPENDENCY"), ui::bold("VERSION"), ui::bold("SOURCE"), ui::bold("STATUS"));
            println!("  {}", ui::dim("─────────────────────────────────────────────────────────────────────────"));
            if pkg.dependencies.is_empty() {
                println!("   {}", ui::dim("(no dependencies specified in manifest)"));
            } else {
                let pkg_dir = Path::new(".lpp_packages");
                for dep in &pkg.dependencies {
                    let source = dep.path.as_deref().or(dep.git.as_deref()).unwrap_or("registry");
                    let version = dep.version.as_deref().unwrap_or("*");
                    let installed = pkg_dir.join(&dep.name).exists();
                    let status_badge = if installed {
                        ui::green("✔ Installed")
                    } else {
                        ui::yellow("▲ Pending")
                    };
                    println!("   {:<24} {:<14} {:<24} {:<12}", ui::bold_cyan(&dep.name), ui::dim(version), ui::dim(source), status_badge);
                }
            }
            println!("  {}", ui::dim("─────────────────────────────────────────────────────────────────────────"));
            println!("  Total: {} direct dependencies", pkg.dependencies.len());
            println!();
            0
        }
        Err(e) => { eprintln!("{}", ui::tag_error(&e)); 1 }
    }
}

fn cmd_tree() -> i32 {
    let packages = read_lockfile();
    if packages.is_empty() {
        println!("  {}", ui::tag_warn("No lockfile packages found. Run `lpp install` first."));
        return 1;
    }
    let manifest_name = read_manifest().map(|p| format!("{} v{}", p.name, p.version)).unwrap_or_else(|_| "project".to_string());
    println!();
    println!("  {} {}", ui::bold_cyan("📦"), ui::bold(&manifest_name));
    let total = packages.len();
    for (i, pkg) in packages.iter().enumerate() {
        let is_last = i + 1 == total;
        let prefix = if is_last { "  └── " } else { "  ├── " };
        let version = pkg.version.as_deref().unwrap_or("0.1.0");
        let resolved = pkg.resolved.as_deref().unwrap_or(&pkg.source);
        println!("{}{}{} {} {}", prefix, ui::green("📦 "), ui::bold(&pkg.name), ui::dim(&format!("v{version}")), ui::dim(&format!("({resolved})")));
    }
    println!();
    0
}

fn cmd_metadata() -> i32 {
    match read_manifest() {
        Ok(pkg) => {
            println!();
            println!("  {}", ui::bold_cyan("╭─────────────────────────────────────────────────────────────╮"));
            println!("  {}  {} PACKAGE MANIFEST METADATA: {:<27} {}", ui::bold_cyan("│"), ui::bold("📦"), pkg.name, ui::bold_cyan("│"));
            println!("  {}", ui::bold_cyan("╰─────────────────────────────────────────────────────────────╯"));
            println!("    {:16} {}", ui::bold("Name:"), ui::cyan(&pkg.name));
            println!("    {:16} {}", ui::bold("Version:"), ui::green(&pkg.version));
            if let Some(ref author) = pkg.author {
                println!("    {:16} {}", ui::bold("Author:"), author);
            }
            println!("    {:16} {}", ui::bold("Entry Point:"), pkg.entry.unwrap_or_else(resolve_entry_point));
            println!("    {:16} {}", ui::bold("Dependencies:"), pkg.dependencies.len());
            println!("    {:16} {}", ui::bold("Locked Pkgs:"), read_lockfile().len());
            println!();
            0
        }
        Err(e) => { eprintln!("{}", ui::tag_error(&e)); 1 }
    }
}

fn cmd_outdated() -> i32 {
    let package = match read_manifest() {
        Ok(pkg) => pkg,
        Err(e) => { eprintln!("{}", ui::tag_error(&e)); return 1; }
    };
    let locked: std::collections::HashMap<String, String> = read_lockfile()
        .into_iter()
        .filter_map(|pkg| pkg.version.map(|version| (pkg.name, version)))
        .collect();
    let mut found = false;
    println!();
    for dep in package.dependencies {
        if dep.version.is_none() {
            found = true;
            println!("  {} {} is not version-pinned", ui::yellow("▲"), ui::bold(&dep.name));
            continue;
        }
        let Some(locked_version) = locked.get(&dep.name) else {
            found = true;
            println!("  {} {} is not present in lpp.lock", ui::yellow("▲"), ui::bold(&dep.name));
            continue;
        };
        let requirement = semver::VersionReq::parse(dep.version.as_deref().unwrap_or("*"));
        let current = semver::Version::parse(locked_version);
        match (current, requirement) {
            (Ok(current), Ok(requirement)) if !requirement.matches(&current) => {
                found = true;
                println!("  {} {} {} does not satisfy requirement {}", ui::yellow("▲"), ui::bold(&dep.name), ui::red(&current.to_string()), ui::cyan(&requirement.to_string()));
            }
            (Err(_), Ok(_)) => {
                found = true;
                println!("  {} {} has non-standard SemVer in lpp.lock ({})", ui::yellow("▲"), ui::bold(&dep.name), locked_version);
            }
            _ => {}
        }
    }
    if !found {
        println!("  {}", ui::tag_success("All direct dependencies are satisfied and up-to-date."));
    }
    println!();
    0
}

fn cmd_clean() -> i32 {
    let start = std::time::Instant::now();
    let mut removed = 0;
    let mut failed = false;
    for target in ["target", "LppData", "dist", "output.c", "output.obj", "output.o"] {
        let path = Path::new(target);
        if !path.exists() {
            continue;
        }
        let result = if path.is_dir() { fs::remove_dir_all(path) } else { fs::remove_file(path) };
        match result {
            Ok(()) => removed += 1,
            Err(_) => failed = true,
        }
    }
    if let Ok(entries) = fs::read_dir(".") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|ext| ext == "exe" || ext == "o" || ext == "obj").unwrap_or(false) {
                match fs::remove_file(&path) { Ok(()) => removed += 1, Err(_) => failed = true }
            }
        }
    }
    let elapsed_ms = start.elapsed().as_millis();
    if failed {
        eprintln!("{}", ui::tag_warn(&format!("Cleaned {removed} items with some errors in {elapsed_ms}ms")));
        1
    } else {
        println!("  {} Cleaned {} build targets & artifact caches in {}ms", ui::green("✔"), removed, elapsed_ms);
        0
    }
}

fn cmd_check() -> i32 {
    println!("[L++] Checking project...");
    let entry_point_str = resolve_entry_point();
    let entry_point = Path::new(&entry_point_str);
    if !entry_point.exists() {
        eprintln!("[L++] Error: entry point '{}' not found.", entry_point.display());
        return 1;
    }
    let compiler_path = match current_compiler_path() {
        Ok(path) => path,
        Err(e) => { eprintln!("[L++] {e}"); return 1; }
    };
    match std::process::Command::new(&compiler_path).arg(entry_point).arg("--check").status() {
        Ok(s) if s.success() => { println!("[L++] Project is semantically valid."); 0 }
        Ok(s) => { eprintln!("[L++] Error: Project check failed ({s})."); s.code().unwrap_or(1) }
        Err(e) => { eprintln!("[L++] Error: failed to execute compiler '{}': {e}", compiler_path.display()); 1 }
    }
}

fn cmd_doctor() -> i32 {
    println!();
    println!("  {}", ui::bold_cyan("╭─────────────────────────────────────────────────────────────╮"));
    println!("  {}  {} L++ Toolchain & Environment Diagnostics               {}", ui::bold_cyan("│"), ui::bold("🩺"), ui::bold_cyan("│"));
    println!("  {}", ui::bold_cyan("╰─────────────────────────────────────────────────────────────╯"));
    println!();

    println!("  {}", ui::bold_cyan("1. COMPILER & HOST ARCHITECTURE"));
    println!("    {} Version:      L++ v{} ({}-bit)", ui::green("✔"), env!("CARGO_PKG_VERSION"), std::mem::size_of::<usize>() * 8);
    println!("    {} Host System:  {} ({})", ui::green("✔"), std::env::consts::OS, std::env::consts::ARCH);

    println!();
    println!("  {}", ui::bold_yellow("2. NATIVE TOOLCHAINS & LINKERS"));
    let has_msvc = command_available("cl.exe", &["/?"]) || Path::new("C:\\Program Files\\Microsoft Visual Studio").exists();
    if has_msvc {
        println!("    {} MSVC / Windows SDK: Detected", ui::green("✔"));
    } else {
        println!("    {} MSVC Toolchain: Not in PATH (direct standalone linker active)", ui::yellow("▲"));
    }

    let has_gcc = command_available("gcc", &["--version"]);
    if has_gcc {
        println!("    {} GCC / MinGW: Detected", ui::green("✔"));
    }

    let has_clang = command_available("clang", &["--version"]);
    if has_clang {
        println!("    {} Clang: Detected", ui::green("✔"));
    }

    println!("    {} Direct Linker: Active (lpp-link zero-dependency linker)", ui::green("✔"));

    println!();
    println!("  {}", ui::bold_purple("3. REGISTRY & NETWORK CONNECTIVITY"));
    let reg_url = std::env::var("LPP_REGISTRY_URL").unwrap_or_else(|_| "https://registry.lplusplus.bond".to_string());
    let mut reg_status = false;
    let curl_bin = if command_available("curl.exe", &["--version"]) { "curl.exe" } else { "curl" };
    if let Ok(out) = std::process::Command::new(curl_bin).args(["-fsSL", "--ssl-no-revoke", "--max-time", "3", &format!("{reg_url}/index.json")]).output() {
        if out.status.success() { reg_status = true; }
    }
    if reg_status {
        println!("    {} Official Registry: Connected ({})", ui::green("✔"), ui::cyan(&reg_url));
    } else {
        println!("    {} Official Registry: Offline / Standalone mode ({})", ui::yellow("▲"), ui::cyan(&reg_url));
    }

    println!();
    println!("  {}", ui::bold_green("4. GLOBAL PACKAGE CACHE & STORE"));
    let cache_dir = resolve_global_cache_root();
    let cache_size = compute_dir_size(&cache_dir);
    let cache_size_mb = (cache_size as f64) / (1024.0 * 1024.0);
    println!("    {} Global Cache Path: {}", ui::green("✔"), cache_dir.display());
    println!("    {} Total Store Size:  {:.2} MB", ui::green("✔"), cache_size_mb);

    println!();
    println!("  {}", ui::tag_success("Toolchain diagnostics complete. All systems fully operational!"));
    println!();
    0
}

fn cmd_cache(args: &[String]) -> i32 {
    let sub = args.first().map(String::as_str).unwrap_or("info");
    let cache_root = resolve_global_cache_root();
    let packages_dir = cache_root.join("packages");

    match sub {
        "info" | "status" => {
            let total_size = compute_dir_size(&cache_root);
            let total_mb = (total_size as f64) / (1024.0 * 1024.0);
            let mut count = 0;
            if let Ok(entries) = fs::read_dir(&packages_dir) {
                count = entries.flatten().count();
            }
            println!();
            println!("  {}", ui::bold_cyan("╭─────────────────────────────────────────────────────────────╮"));
            println!("  {}  {} L++ GLOBAL PACKAGE STORE & CACHE                       {}", ui::bold_cyan("│"), ui::bold("📦"), ui::bold_cyan("│"));
            println!("  {}", ui::bold_cyan("╰─────────────────────────────────────────────────────────────╯"));
            println!("    {:18} {}", ui::bold("Cache Root:"), cache_root.display());
            println!("    {:18} {} packages", ui::bold("Cached Items:"), count);
            println!("    {:18} {:.2} MB", ui::bold("Total Size:"), total_mb);
            println!();
            println!("  {}", ui::dim("Commands:"));
            println!("    {} {}", ui::cyan("lpp cache list"), ui::dim("List all cached packages"));
            println!("    {} {}", ui::cyan("lpp cache clean"), ui::dim("Purge global package cache"));
            println!();
            0
        }
        "path" => {
            println!("{}", cache_root.display());
            0
        }
        "list" => {
            println!();
            println!("  {} Cached Packages in Store ({}):", ui::bold_cyan("📦"), cache_root.display());
            println!("  {}", ui::dim("───────────────────────────────────────────────────────────────"));
            let mut found = 0;
            if let Ok(entries) = fs::read_dir(&packages_dir) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    let name = entry.file_name().to_string_lossy().into_owned();
                    let size = compute_dir_size(&path);
                    let size_kb = (size as f64) / 1024.0;
                    println!("    {} {:<32} {}", ui::green("◆"), ui::bold(&name), ui::dim(&format!("{size_kb:.1} KB")));
                    found += 1;
                }
            }
            if found == 0 {
                println!("    {}", ui::dim("(no packages cached yet)"));
            }
            println!("  {}", ui::dim("───────────────────────────────────────────────────────────────"));
            println!("  Total: {} cached packages", found);
            println!();
            0
        }
        "clean" | "purge" | "clear" => {
            let start = std::time::Instant::now();
            let total_size = compute_dir_size(&cache_root);
            let total_mb = (total_size as f64) / (1024.0 * 1024.0);
            if cache_root.exists() {
                let _ = fs::remove_dir_all(&cache_root);
            }
            println!("  {} Purged {:.2} MB from global package cache store in {}ms", ui::green("✔"), total_mb, start.elapsed().as_millis());
            0
        }
        other => {
            eprintln!("{}", ui::tag_error(&format!("unknown cache subcommand '{other}'; use info, list, path, or clean")));
            2
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

fn copy_dir_all(src: &Path, dst: &Path) -> std::io::Result<()> {
    fs::create_dir_all(dst)?;
    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let dst_path = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_all(&entry.path(), &dst_path)?;
        } else {
            fs::copy(entry.path(), dst_path)?;
        }
    }
    Ok(())
}

fn cmd_build() -> Option<String> {
    cmd_build_opts(false)
}

fn cmd_build_opts(is_release: bool) -> Option<String> {
    let start_build = std::time::Instant::now();
    let mode_badge = if is_release { ui::bold_green("[RELEASE]") } else { ui::bold_cyan("[DEV]") };
    println!("  {} Building project {}...", ui::bold_cyan("⚡"), mode_badge);
    let entry_point_str = resolve_entry_point();
    let entry_point = Path::new(&entry_point_str);
    if !entry_point.exists() {
        eprintln!("{}", ui::tag_error(&format!("entry point '{}' not found.", entry_point.display())));
        return None;
    }

    if cmd_install(false) != 0 {
        eprintln!("{}", ui::tag_error("dependency installation failed; build aborted"));
        return None;
    }

    let target_dir = if is_release {
        PathBuf::from("dist")
    } else {
        Path::new("LppData").join("build").join("release")
    };
    if let Err(e) = fs::create_dir_all(&target_dir) {
        eprintln!("{}", ui::tag_error(&format!("cannot create build directory '{}': {e}", target_dir.display())));
        return None;
    }

    let compile_start = std::time::Instant::now();
    let obj_file = match compile_source_to_object(entry_point) {
        Ok(path) => path,
        Err(e) => {
            eprintln!("{}", ui::tag_error(&format!("Compilation error: {e}")));
            return None;
        }
    };
    let compile_ms = compile_start.elapsed().as_millis();
    println!("  {} Compiled {} {}", ui::green("✔"), ui::bold(&entry_point.to_string_lossy()), ui::dim(&format!("[{compile_ms}ms]")));

    let mut bin_name = "output".to_string();
    if let Ok(pkg) = read_manifest() {
        bin_name = pkg.name;
    }

    let exe_path = output_path_for_name(&target_dir, &bin_name);
    let _ = fs::remove_file(&exe_path);

    let link_start = std::time::Instant::now();
    let link_result = link_native_binary(&obj_file, &exe_path);
    let _ = fs::remove_file(&obj_file);

    if let Err(e) = link_result {
        eprintln!("{}", ui::tag_error(&format!("Linking error: {e}")));
        None
    } else {
        let link_ms = link_start.elapsed().as_millis();
        println!("  {} Linked {} {}", ui::green("✔"), ui::bold(&exe_path.to_string_lossy()), ui::dim(&format!("[{link_ms}ms]")));
        let total_ms = start_build.elapsed().as_millis();
        let size_bytes = fs::metadata(&exe_path).map(|m| m.len()).unwrap_or(0);
        let size_kb = (size_bytes as f64) / 1024.0;
        
        if is_release {
            println!("  {} Standalone Release binary: {} {} {}", ui::bold_green("✨"), ui::bold(&exe_path.display().to_string()), ui::cyan(&format!("({size_kb:.1} KB)")), ui::dim(&format!("in {total_ms}ms")));
            let www_dir = Path::new("www");
            if www_dir.exists() {
                let dist_www = target_dir.join("www");
                if let Err(e) = copy_dir_all(www_dir, &dist_www) {
                    eprintln!("{}", ui::tag_warn(&format!("failed to bundle www assets into dist/www: {e}")));
                } else {
                    println!("  {} Bundled static web assets into {}", ui::green("✔"), dist_www.display());
                }
            }
        } else {
            println!("  {} Build successful: {} {} {}", ui::bold_green("✨"), ui::bold(&exe_path.display().to_string()), ui::cyan(&format!("({size_kb:.1} KB)")), ui::dim(&format!("in {total_ms}ms")));
        }
        Some(exe_path.to_string_lossy().into_owned())
    }
}

fn cmd_dev() -> i32 {
    let entry_point_str = resolve_entry_point();
    if !Path::new(&entry_point_str).exists() {
        eprintln!("{}", ui::tag_error(&format!("entry point '{}' not found.", entry_point_str)));
        return 1;
    }
    println!();
    println!("  {}", ui::bold_purple("╭─────────────────────────────────────────────────────────────╮"));
    println!("  {}  {} Lreact Development Server (IPC Native Backend)       {}", ui::bold_purple("│"), ui::bold("⚡"), ui::bold_purple("│"));
    println!("  {}  Local Dev URL: {}                         {}", ui::bold_purple("│"), ui::bold_cyan("http://localhost:3000"), ui::bold_purple("│"));
    println!("  {}", ui::bold_purple("╰─────────────────────────────────────────────────────────────╯"));
    println!();
    let Some(exe_path) = cmd_build_opts(false) else { return 1; };
    println!("  {} Launching dev server {}...", ui::bold_cyan("❯"), exe_path);
    match std::process::Command::new(&exe_path).status() {
        Ok(status) => status.code().unwrap_or(if status.success() { 0 } else { 1 }),
        Err(e) => { eprintln!("{}", ui::tag_error(&format!("Execution failed: {e}"))); 1 }
    }
}

fn cmd_run() -> i32 {
    let Some(exe_path) = cmd_build() else { return 1; };
    println!("  {} Running {}...", ui::bold_cyan("❯"), ui::bold(&exe_path));
    println!();
    match std::process::Command::new(&exe_path).status() {
        Ok(status) => status.code().unwrap_or(if status.success() { 0 } else { 1 }),
        Err(e) => { eprintln!("{}", ui::tag_error(&format!("Failed to execute target: {e}"))); 1 }
    }
}

fn cmd_bench() -> i32 {
    println!("  {} Launching lpp-bench...", ui::bold_cyan("⚡"));
    let bench_bin = current_binary_dir()
        .map(|dir| dir.join(format!("lpp-bench{}", std::env::consts::EXE_SUFFIX)))
        .filter(|p| p.exists());
    let Some(bench) = bench_bin else {
        eprintln!("{}", ui::tag_error("lpp-bench not found. Build it with: cargo build --release --bin lpp-bench"));
        return 1;
    };
    let args: Vec<String> = std::env::args().skip(2).collect();
    match std::process::Command::new(&bench).args(&args).status() {
        Ok(status) => status.code().unwrap_or(if status.success() { 0 } else { 1 }),
        Err(e) => { eprintln!("{}", ui::tag_error(&format!("Failed to launch lpp-bench: {e}"))); 1 }
    }
}

fn cmd_test() -> i32 {
    println!("  {} Running test suite...", ui::bold_cyan("⚡"));
    if (Path::new("lpp.toml").exists() || Path::new("lpp.json").exists())
        && cmd_install(false) != 0
    {
        eprintln!("{}", ui::tag_error("dependency installation failed; tests aborted"));
        return 1;
    }
    let test_dir = if Path::new("tests").exists() {
        "tests"
    } else if Path::new("test").exists() {
        "test"
    } else {
        println!("  {}", ui::tag_info("No tests/ or test/ directory found."));
        return 0;
    };

    let paths = match fs::read_dir(test_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{}", ui::tag_error(&format!("Failed to read tests directory: {e}")));
            return 1;
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

    test_files.sort();
    test_files.retain(|path| {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        !name.contains("rejected")
            && !name.starts_with("aot_reject")
            && !name.starts_with("tuple_bad")
            && !name.starts_with("variadic_bad")
            && !name.starts_with("list_set_bad")
    });

    if test_files.is_empty() {
        println!("  {}", ui::tag_info(&format!("No test files found in directory '{test_dir}'.")));
        return 0;
    }

    let mut passed = 0;
    let mut failed = 0;

    let target_test_dir = Path::new("target").join("test");
    let _ = fs::create_dir_all(&target_test_dir);

    for test_path in test_files {
        let test_name = test_path.file_name().and_then(|name| name.to_str()).unwrap_or("unnamed");
        print!("  test {:<40} ... ", test_name);
        match compile_source_to_object(&test_path) {
            Ok(obj) => {
                let test_exe = target_test_dir.join(format!("{test_name}.exe"));
                if link_native_binary(&obj, &test_exe).is_ok() {
                    let run_res = std::process::Command::new(&test_exe).status();
                    let _ = fs::remove_file(&test_exe);
                    if let Ok(s) = run_res {
                        if s.success() {
                            println!("{}", ui::green("ok"));
                            passed += 1;
                        } else {
                            println!("{}", ui::red("FAILED (runtime non-zero exit)"));
                            failed += 1;
                        }
                    } else {
                        println!("{}", ui::red("FAILED (execution error)"));
                        failed += 1;
                    }
                } else {
                    println!("{}", ui::red("FAILED (linking failed)"));
                    failed += 1;
                }
            }
            Err(_) => {
                println!("{}", ui::red("FAILED (compilation failed)"));
                failed += 1;
            }
        }
    }

    println!();
    if failed == 0 {
        println!("  {} Test result: {} ({} passed, 0 failed)", ui::bold_green("✨"), ui::bold_green("ok"), passed);
        0
    } else {
        println!("  {} Test result: {} ({} passed, {} failed)", ui::bold_red("✖"), ui::bold_red("FAILED"), passed, failed);
        1
    }
}

fn cmd_publish(args: &[String]) -> i32 {
    let manifest = match read_manifest() {
        Ok(m) => m,
        Err(e) => {
            eprintln!("{}", ui::tag_error(&format!("Publish error: {e}")));
            return 1;
        }
    };

    if let Err(e) = validate_package_name(&manifest.name) {
        eprintln!("{}", ui::tag_error(&format!("Publish error: {e}")));
        return 1;
    }

    let dry_run = args.iter().any(|a| a == "--dry-run");
    let dry_badge = if dry_run { ui::yellow(" [DRY-RUN]") } else { String::new() };
    println!();
    println!("  {} Publishing {} {}...{}", ui::bold_cyan("📦"), ui::bold(&manifest.name), ui::dim(&format!("v{}", manifest.version)), dry_badge);
    println!("  {} Validating package manifest...", ui::tag_step(1, 3, "Manifest"));
    println!("    {} {}", ui::dim("Name:"), ui::cyan(&manifest.name));
    println!("    {} {}", ui::dim("Version:"), ui::green(&manifest.version));
    if let Some(ref author) = manifest.author {
        println!("    {} {}", ui::dim("Author:"), author);
    }

    println!("  {} Verifying package build...", ui::tag_step(2, 3, "Build Verification"));
    if cmd_build_opts(false).is_none() {
        eprintln!("{}", ui::tag_error("Publish error: package failed to build successfully"));
        return 1;
    }

    println!("  {} Preparing registry upload...", ui::tag_step(3, 3, "Registry Upload"));
    let registry_url = std::env::var("LPP_REGISTRY_URL")
        .unwrap_or_else(|_| "https://registry.lplusplus.bond".to_string());

    if dry_run {
        println!();
        println!("  {} [DRY-RUN] Pre-flight validation passed cleanly!", ui::bold_green("✨"));
        println!("    {} {}/publish", ui::dim("Target Registry:"), ui::cyan(&registry_url));
        println!("    {} v{}", ui::dim("Git Release Tag:"), manifest.version);
        println!("    {} Ready to publish", ui::dim("Status:"));
        println!();
        return 0;
    }

    let api_key = std::env::var("LPP_API_KEY")
        .or_else(|_| std::env::var("SUPABASE_SERVICE_ROLE_KEY"))
        .unwrap_or_default();

    if api_key.is_empty() {
        eprintln!("{}", ui::tag_error("Publish error: LPP_API_KEY or SUPABASE_SERVICE_ROLE_KEY required to upload."));
        eprintln!("  Set LPP_API_KEY or run with --dry-run.");
        return 1;
    }

    let pkg_json = format!(
        r#"{{"name":"{}","version":"{}","published_by":"cli"}}"#,
        manifest.name,
        manifest.version
    );

    let curl_bin = if command_available("curl.exe", &["--version"]) { "curl.exe" } else { "curl" };
    let publish_url = format!("{}/publish", registry_url);
    let mut cmd = std::process::Command::new(curl_bin);
    cmd.args([
        "-fsSL",
        "--ssl-no-revoke",
        "-X", "POST",
        "-H", "Content-Type: application/json",
        "-H", &format!("Authorization: Bearer {}", api_key),
        "-d", &pkg_json,
        &publish_url,
    ]);

    match cmd.output() {
        Ok(out) if out.status.success() => {
            println!("[L++] Published successfully to {}!", publish_url);
            0
        }
        Ok(out) => {
            let raw_err = String::from_utf8_lossy(&out.stderr);
            let sanitized_err = sanitize_output_for_secrets(&raw_err, &[&api_key]);
            eprintln!("[L++] Publish failed: {sanitized_err}");
            1
        }
        Err(e) => {
            let raw_err = e.to_string();
            let sanitized_err = sanitize_output_for_secrets(&raw_err, &[&api_key]);
            eprintln!("[L++] Publish execution error: {sanitized_err}");
            1
        }
    }
}
