use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

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

fn validate_package_name(name: &str) -> Result<(), String> {
    if name.trim().is_empty() {
        return Err("Package manifest error: package name cannot be empty".to_string());
    }
    if name.len() > 128 {
        return Err("Package manifest error: package name exceeds 128 characters".to_string());
    }
    if name.chars().any(|ch| matches!(ch, '\\' | '\n' | '\r' | '\t')) {
        return Err(format!("Package manifest error: invalid package name '{name}'"));
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

pub fn parse_toml(content: &str) -> Result<Package, String> {
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
    let version = package
        .get("version")
        .and_then(toml::Value::as_str)
        .ok_or_else(|| "Missing package version in [package] section".to_string())?
        .to_string();
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
    if std::path::Path::new("lpp.json").exists() {
        let content = fs::read_to_string("lpp.json")
            .map_err(|e| format!("Failed to read lpp.json: {}", e))?;
        parse_json_manifest(&content)
    } else if std::path::Path::new("lpp.toml").exists() {
        let content = fs::read_to_string("lpp.toml")
            .map_err(|e| format!("Failed to read lpp.toml: {}", e))?;
        parse_toml(&content)
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

fn resolve_registry_cache_path() -> PathBuf {
    if let Ok(var) = std::env::var("LPP_HOME").or_else(|_| std::env::var("LPP_DIR")) {
        return PathBuf::from(var).join("cache").join("registry_cache.json");
    }
    if let Ok(home) = std::env::var("HOME").or_else(|_| std::env::var("USERPROFILE")) {
        return PathBuf::from(home).join(".lpp").join("cache").join("registry_cache.json");
    }
    std::env::temp_dir().join(".lpp_registry_cache.json")
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

    let p = PathBuf::from(src_name);
    if p.exists() {
        return Some(p);
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(exe_dir) = exe_path.parent() {
            for ancestor in &[exe_dir.to_path_buf(), exe_dir.join(".."), exe_dir.join("../.."), exe_dir.join("../../..")] {
                let candidate = ancestor.join(src_name);
                if candidate.exists() {
                    return Some(candidate);
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
                #[cfg(windows)]
                load_msvc_env();
                let cc = if cfg!(windows) { "cl.exe" } else { "gcc" };
                let mut cmd = std::process::Command::new(cc);
                if cfg!(windows) {
                    cmd.arg("/nologo")
                        .arg("/O2")
                        .arg("/GS-")
                        .arg("/Gs1000000")
                        .arg("/DLPP_FREESTANDING")
                        .arg("/c")
                        .arg(&src_path)
                        .arg(format!("/Fo:{}", cache_obj.display()));
                } else {
                    cmd.arg("-Os")
                        .arg("-fno-stack-protector")
                        .arg("-DLPP_FREESTANDING")
                        .arg("-c")
                        .arg(&src_path)
                        .arg("-o")
                        .arg(&cache_obj);
                }
                if let Ok(st) = cmd.status() {
                    if st.success() {
                        if let Some(cur) = current_hash {
                            let _ = fs::write(&cache_hash, cur.to_string());
                        }
                    }
                }
            }

            if cache_obj.exists() {
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
    let use_host = std::env::var("LPP_LINKER").as_deref() == Ok("host");
    if use_host {
        #[cfg(windows)]
        load_msvc_env();
        host_link_binary(obj_file, output_path, &[])
    } else {
        direct_link_binary(obj_file, output_path)
    }
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
            let is_release = args.iter().any(|a| a == "--release");
            if cmd_build_opts(is_release).is_some() { 0 } else { 1 }
        }
        "run" => cmd_run(),
        "test" => cmd_test(),
        "bench" => cmd_bench(),
        "help" => {
            print_help();
            0
        }
        "publish" => {
            eprintln!("[L++] 'publish' requires the self-hosted PM (lpp-pm). Ensure LPP_HOME is set.");
            1
        }
        cmd => {
            eprintln!("[L++] Unknown package manager command: '{}'", cmd);
            print_help();
            2
        }
    }
}

fn print_help() {
    println!("L++ Package Manager v{}", env!("CARGO_PKG_VERSION"));
    println!("Usage:");
    println!("  lpp <file.lpp> [options]          Compile one source file");
    println!("  lpp <command> [args]              Package/app workflow");
    println!();
    println!("Project workflow:");
    println!("  new <name>                        Create a new L++ package directory");
    println!("  init [name]                       Initialize lpp.toml in current directory");
    println!("  add <pkg>                         Add a dependency to lpp.toml");
    println!("  add @owner/repo                   Add dependency from GitHub");
    println!("  add <pkg> --git <url>             Add dependency from explicit git URL");
    println!("  add <pkg> --path <dir>            Add local path dependency");
    println!("  install                           Install dependencies from lpp.toml");
    println!("  update                            Refresh dependencies and lockfile");
    println!("  remove <pkg>                      Remove dependency from lpp.toml");
    println!("  list                              List direct dependencies");
    println!("  tree                              Print lockfile dependency tree");
    println!("  metadata                          Print package manifest");
    println!("  outdated                          Show unpinned or incompatible dependencies");
    println!("  version                           Show package version");
    println!("  version set <semver>              Set package version");
    println!("  version bump [major|minor|patch]  Bump package version");
    println!();
    println!("Global app workflow:");
    println!("  install lpp-opencode              Install lpp-opencode command globally");
    println!("  install opencode                  Alias for lpp-opencode");
    println!("  install openclaude                Alias for lpp-opencode");
    println!();
    println!("Build/test workflow:");
    println!("  check                             Type-check project");
    println!("  build                             Build project to native binary");
    println!("  run                               Compile and run project");
    println!("  test                              Run tests in tests/ directory");
    println!("  clean                             Remove build output/artifacts");
    println!("  bench                             Run benchmarks");
    println!();
    println!("Registry:");
    println!("  search <query>                    Search package registry");
    println!("  publish                           Publish package to registry (requires git)");
    println!();
    println!("Lreact/web workflow:");
    println!("  create web <name>                 Create a new Lreact desktop web app");
    println!("  lpp lreact create <name>          Create a new Lreact web app");
    println!("  lpp lreact dev                    Start Lreact development server");
    println!("  lpp lreact build                  Build release bundle in dist/");
    println!();
    println!("Single-file options:");
    println!("  lpp <file.lpp> --check            Type-check without compiling");
    println!("  lpp <file.lpp> --emit-obj         Emit native object file (.o/.obj)");
    println!("  lpp --checkall                    Check all .lpp files in directory");
    println!("  lpp --checkall --fix              Check and automatically repair source files");
    println!();
    println!("Difference between add and install:");
    println!("  lpp add <pkg>                     Writes dependency into this project's lpp.toml");
    println!("  lpp install                       Installs dependencies listed in lpp.toml");
    println!("  lpp install lpp-opencode          Installs a known app globally, no project needed");
}

fn cmd_new(args: &[String]) -> i32 {
    let mut is_web = false;
    let mut name_arg = None;

    for arg in args {
        if arg == "web" || arg == "--web" || arg == "lreact" {
            is_web = true;
        } else if !arg.starts_with('-') {
            if name_arg.is_some() {
                eprintln!("[L++] Error: expected one project name, got '{}'.", arg);
                return 2;
            }
            name_arg = Some(arg.as_str());
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
        eprintln!(
            "[L++] Error: directory '{}' already exists.",
            project_dir.display()
        );
        return 1;
    }

    if let Err(e) = fs::create_dir_all(&project_dir) {
        eprintln!("Failed to create project directory: {}", e);
        return 1;
    }

    if is_web {
        println!("[Lreact] Creating new Lreact Web App '{}'...", raw_name);
        match write_web_scaffold(&project_dir, &package_name) {
            Ok(()) => {
                println!("[Lreact] Web App '{}' created at {}.", package_name, project_dir.display());
                // Scaffold creation must not report success if the dependency
                // install failed. Restore the caller's cwd even on errors.
                let old_cwd = match std::env::current_dir() {
                    Ok(cwd) => cwd,
                    Err(e) => {
                        eprintln!("[L++] cannot read current directory: {e}");
                        return 1;
                    }
                };
                if let Err(e) = std::env::set_current_dir(&project_dir) {
                    eprintln!("[L++] cannot enter new project: {e}");
                    return 1;
                }
                let install_status = cmd_install(false);
                let restore_status = std::env::set_current_dir(old_cwd);
                if let Err(e) = restore_status {
                    eprintln!("[L++] warning: cannot restore current directory: {e}");
                }
                if install_status != 0 {
                    eprintln!("[Lreact] dependency install was not completed; run `lpp install` when network access is available");
                }
                println!("\nNext steps:");
                println!("  cd {}", raw_name);
                println!("  lpp dev              # Start local Lreact dev server at http://localhost:3000");
                println!("  lpp build --release  # Build standalone native executable in dist/");
            }
            Err(e) => {
                eprintln!("{}", e);
                return 1;
            }
        }
    } else {
        println!("[L++] Creating new project '{}'...", raw_name);
        if let Err(e) = write_project_scaffold(&project_dir, &package_name) {
            eprintln!("{}", e);
            return 1;
        }
        println!(
            "[L++] Project '{}' created at {}.",
            package_name,
            project_dir.display()
        );
    }
    0
}

fn cmd_init(args: &[String]) -> i32 {
    let project_name =
        normalize_package_name(args.get(0).map(|s| s.as_str()).unwrap_or("my_project"));
    println!("[L++] Initializing new project '{}'...", project_name);
    match write_project_scaffold(Path::new("."), &project_name) {
        Ok(()) => {
            println!("[L++] Project '{}' initialized successfully!", project_name);
            0
        }
        Err(e) => {
            eprintln!("{}", e);
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

    if args.is_empty() || args[0] == "--show" {
        println!("{} {}", package.name, package.version);
        return 0;
    }

    let operation = if args[0] == "set" {
        let Some(version) = args.get(1) else {
            eprintln!("Usage: lpp version set <semver>");
            return 2;
        };
        version.clone()
    } else {
        let segment = if args[0] == "bump" {
            args.get(1).map(String::as_str).unwrap_or("patch")
        } else if args[0] == "--bump" {
            args.get(1).map(String::as_str).unwrap_or("patch")
        } else {
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

pub fn resolve_from_json(json_str: &str, target_name: &str) -> Option<RegistryEntry> {
    if let Ok(manifest) = serde_json::from_str::<RegistryManifest>(json_str) {
        if let Some(entry) = manifest.packages.get(target_name) {
            return Some(entry.clone());
        }
        let lower_target = target_name.to_lowercase();
        for (k, v) in &manifest.packages {
            if k.to_lowercase() == lower_target {
                return Some(v.clone());
            }
        }
        let repo_leaf = target_name.split('/').last().unwrap_or(target_name);
        for (k, v) in &manifest.packages {
            let k_leaf = k.split('/').last().unwrap_or(k);
            if k_leaf.eq_ignore_ascii_case(repo_leaf) {
                return Some(v.clone());
            }
        }
    } else if let Ok(val) = serde_json::from_str::<serde_json::Value>(json_str) {
        if let Some(pkgs) = val.get("packages").and_then(|p| p.as_object()) {
            let repo_leaf = target_name.split('/').last().unwrap_or(target_name);
            for (k, v) in pkgs {
                let k_leaf = k.split('/').last().unwrap_or(k);
                if k.eq_ignore_ascii_case(target_name) || k_leaf.eq_ignore_ascii_case(repo_leaf) {
                    let git = v
                        .get("git")
                        .or_else(|| v.get("repository"))
                        .and_then(|g| g.as_str())?
                        .to_string();
                    let branch = v.get("branch").and_then(|b| b.as_str()).map(String::from);
                    let tag = v.get("tag").and_then(|t| t.as_str()).map(String::from);
                    let version = v.get("version").and_then(|x| x.as_str()).map(String::from);
                    let path = v.get("path").and_then(|x| x.as_str()).map(String::from);
                    let source = v.get("source").or_else(|| v.get("source_url")).and_then(|x| x.as_str()).map(String::from);
                    let description = v.get("description").and_then(|d| d.as_str()).map(String::from);
                    return Some(RegistryEntry { git, branch, tag, version, path, source, description });
                }
            }
        }
    }
    None
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
                return Some(content);
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
                        return Some(content);
                    }
                }
            }
        }
    }

    let registry_urls = [
        "https://samarnever-droid.github.io/lplusplus/registry/index.json",
        "https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/website/public/registry/index.json",
        "https://raw.githubusercontent.com/samarnever-droid/lplusplus/master/registry/index.json",
    ];
    let mut fetched_json: Option<String> = None;

    for url in &registry_urls {
        if command_available("curl", &["--version"]) {
            let output = std::process::Command::new("curl")
                .args(["-fsSL", "--max-time", "5", url])
                .output()
                .ok();
            if let Some(out) = output {
                if out.status.success() {
                    let text = String::from_utf8_lossy(&out.stdout).into_owned();
                    if !text.trim().is_empty() && text.contains("packages") {
                        fetched_json = Some(text);
                        break;
                    }
                }
            }
        }

        #[cfg(windows)]
        {
            let cmd_arg = format!("Invoke-RestMethod -Uri '{}' -TimeoutSec 5 | ConvertTo-Json -Depth 5", url);
            let output = std::process::Command::new("powershell")
                .args(["-Command", &cmd_arg])
                .output()
                .ok();
            if let Some(out) = output {
                if out.status.success() {
                    let text = String::from_utf8_lossy(&out.stdout).into_owned();
                    if !text.trim().is_empty() && text.contains("packages") {
                        fetched_json = Some(text);
                        break;
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
            if !content.trim().is_empty() {
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

fn is_lpp_opencode_alias(name: &str) -> bool {
    matches!(
        name.to_ascii_lowercase().as_str(),
        "lpp-opencode" | "opencode" | "openclaude" | "lpp-openclaude"
    )
}

fn install_lpp_opencode_global() -> i32 {
    println!("[L++] Installing lpp-opencode globally...");
    #[cfg(windows)]
    {
        let script = "irm https://raw.githubusercontent.com/samarnever-droid/lpp-opencode/main/scripts/install.ps1 | iex";
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script])
            .status();
        return match status {
            Ok(s) if s.success() => {
                println!("[L++] lpp-opencode installed. Run: lpp-opencode /provider");
                0
            }
            Ok(s) => {
                eprintln!("[L++] lpp-opencode installer exited with status {}", s);
                s.code().unwrap_or(1)
            }
            Err(e) => {
                eprintln!("[L++] Failed to run PowerShell installer: {}", e);
                1
            }
        };
    }
    #[cfg(not(windows))]
    {
        let script = "curl -fsSL https://raw.githubusercontent.com/samarnever-droid/lpp-opencode/main/scripts/install.sh | sh";
        let status = std::process::Command::new("sh").args(["-c", script]).status();
        match status {
            Ok(s) if s.success() => {
                println!("[L++] lpp-opencode installed. Run: lpp-opencode /provider");
                0
            }
            Ok(s) => {
                eprintln!("[L++] lpp-opencode installer exited with status {}", s);
                s.code().unwrap_or(1)
            }
            Err(e) => {
                eprintln!("[L++] Failed to run shell installer: {}", e);
                1
            }
        }
    }
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

    if let Some(git_url) = dep.git.as_deref() {
        if git_url.trim().is_empty() {
            return Err(format!("dependency '{}' has an empty git URL", dep.name));
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
        }
        let commit = git_output(["-C", destination.to_string_lossy().as_ref(), "rev-parse", "HEAD"], &format!("reading '{}' revision", dep.name))?;
        return Ok(format!("git+{}#{}", git_url, commit));
    }

    if let Some(path) = dep.path.as_deref() {
        let source = Path::new(path);
        if !source.exists() {
            return Err(format!("path dependency '{}' does not exist: {}", dep.name, source.display()));
        }
        if destination.exists() {
            fs::remove_dir_all(destination).map_err(|e| format!("replace path dependency '{}': {e}", dep.name))?;
        }
        if source.is_dir() {
            copy_dir_all(source, destination).map_err(|e| format!("copy path dependency '{}': {e}", dep.name))?;
        } else if source.is_file() {
            // Registry entries for stdlib modules point at a single .lpp file.
            // Materialise that file as a tiny package instead of rejecting a
            // valid registry entry as if it were a broken directory path.
            let file_name = source.file_name().and_then(|n| n.to_str()).unwrap_or("main.lpp");
            let src_dir = destination.join("src");
            fs::create_dir_all(&src_dir).map_err(|e| format!("create path dependency '{}': {e}", dep.name))?;
            fs::copy(source, src_dir.join(file_name)).map_err(|e| format!("copy path dependency '{}': {e}", dep.name))?;
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
        if is_lpp_opencode_alias(package) {
            return install_lpp_opencode_global();
        }
        eprintln!("[L++] No lpp.toml or lpp.json found here, so this is not a project dependency install.");
        eprintln!("[L++] '{}' is not a known global app alias.", package);
        eprintln!();
        eprintln!("What you probably want:");
        eprintln!("  Global app:        lpp install lpp-opencode");
        eprintln!("  Search registry:   lpp search {}", package);
        eprintln!("  New project:       lpp new my_app && cd my_app && lpp add {}", package);
        eprintln!();
        eprintln!("Known global app aliases: lpp-opencode, opencode, openclaude, lpp-openclaude");
        return 1;
    }
    if args.iter().any(|arg| arg == "--offline") {
        // The PM is single-process at this point; use the environment only to
        // pass the flag to the existing install implementation.
        unsafe { std::env::set_var("LPP_OFFLINE", "1") };
    }
    cmd_install(false)
}

fn cmd_install(force_update: bool) -> i32 {
    println!("[L++] Resolving dependencies...");
    let package = match read_manifest() {
        Ok(pkg) => pkg,
        Err(e) => {
            eprintln!("[L++] Manifest error: {}", e);
            return 1;
        }
    };

    let pkg_dir = Path::new(".lpp_packages");
    if let Err(e) = fs::create_dir_all(pkg_dir) {
        eprintln!("Failed to create .lpp_packages directory: {}", e);
        return 1;
    }

    let mut lock_content = String::from("# L++ lockfile v2 — generated by lpp. Do not edit.\nlock_version = 2\n\n");
    let mut worklist = package.dependencies;
    let mut processed = std::collections::HashSet::new();
    let mut specs: std::collections::HashMap<String, (Option<String>, Option<String>, Option<String>, Option<String>)> = std::collections::HashMap::new();
    let mut failed = false;

    while let Some(dep) = worklist.pop() {
        let key = dep.name.clone();
        let spec = (dep.version.clone(), dep.git.clone(), dep.path.clone(), dep.tag.clone().or(dep.branch.clone()));
        if let Some(previous) = specs.get(&key) {
            if previous != &spec {
                eprintln!("[L++] conflicting requirements for dependency '{}'; use one source and version", key);
                failed = true;
            }
            continue;
        }
        specs.insert(key.clone(), spec);
        if !processed.insert(key) {
            continue;
        }

        let mut resolved_dep = dep.clone();
        if resolved_dep.git.is_none() && resolved_dep.path.is_none() {
            if std::env::var_os("LPP_OFFLINE").is_some() {
                eprintln!("[L++] dependency '{}' is not available offline without a local source", resolved_dep.name);
                failed = true;
                continue;
            }
            if let Some(entry) = resolve_registry_package(&resolved_dep.name) {
                if !entry.git.is_empty() {
                    resolved_dep.git = Some(entry.git);
                } else if let Some(path) = entry.path {
                    resolved_dep.path = Some(path);
                } else {
                    eprintln!("[L++] registry entry '{}' has no git or path source", resolved_dep.name);
                    failed = true;
                    continue;
                }
                if resolved_dep.branch.is_none() {
                    resolved_dep.branch = entry.branch;
                }
                if resolved_dep.tag.is_none() {
                    resolved_dep.tag = entry.tag;
                }
                if resolved_dep.version.is_none() {
                    resolved_dep.version = entry.version;
                }
            } else {
                eprintln!("[L++] dependency '{}' was not found in the registry", resolved_dep.name);
                failed = true;
                continue;
            }
        }

        println!("[L++] Installing '{}'...", resolved_dep.name);
        let destination = pkg_dir.join(&resolved_dep.name);
        let source = match install_dependency(&resolved_dep, &destination, force_update) {
            Ok(source) => source,
            Err(e) => {
                eprintln!("[L++] {e}");
                failed = true;
                continue;
            }
        };
        lock_content.push_str(&lock_package_block(&resolved_dep, &source, &destination));

        let manifest_path = if destination.join("lpp.json").is_file() {
            destination.join("lpp.json")
        } else {
            destination.join("lpp.toml")
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
                    eprintln!("[L++] invalid manifest in '{}': {e}", destination.display());
                    failed = true;
                }
            }
        }
    }

    if failed {
        eprintln!("[L++] dependency installation failed; existing lpp.lock was not replaced");
        return 1;
    }

    let lock_path = Path::new("lpp.lock");
    let temp = lock_path.with_extension("lock.tmp");
    if let Err(e) = fs::write(&temp, lock_content) {
        eprintln!("Failed to write temporary lockfile: {}", e);
        return 1;
    }
    if let Err(e) = replace_file(&temp, lock_path) {
        let _ = fs::remove_file(&temp);
        eprintln!("Failed to replace lpp.lock: {e}");
        return 1;
    }
    println!("[L++] lpp.lock file generated.");
    println!("[L++] Dependencies resolved successfully.");
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
        assert!(updated.contains("[dependencies]\nfoo = { path = \"../foo\", version = \"^1.0\" }") );
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
    n == "lpp-opencode" || n == "opencode" || n == "openclaude" || n == "lpp-openclaude"
}

fn print_search_item(name: &str, entry: &RegistryEntry, app: bool) {
    println!("  {}", name);
    if let Some(ref desc) = entry.description {
        println!("      {}", desc);
    }
    if !entry.git.is_empty() {
        println!("      repo: {}", entry.git);
    }
    if let Some(ref path) = entry.path {
        println!("      path: {}", path);
    }
    if let Some(ref version) = entry.version {
        println!("      version: {}", version);
    }
    if let Some(ref b) = entry.branch {
        println!("      branch: {}", b);
    }
    if let Some(ref t) = entry.tag {
        println!("      tag: {}", t);
    }
    if app {
        println!("      install: lpp install {}", name);
        println!("      run:     lpp-opencode");
    } else {
        println!("      add:     lpp add {}", name);
        println!("      install: lpp install   # inside your project");
    }
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
        let package = parse_toml(&fs::read_to_string(&manifest_path).map_err(|e| format!("read '{}': {e}", manifest_path.display()))?)?;
        result.push((relative.to_string(), path, package));
    }
    Ok(result)
}

fn cmd_workspace(args: &[String]) -> i32 {
    let (root, manifest) = match workspace_root(Path::new(".")) {
        Ok(value) => value,
        Err(e) => { eprintln!("[L++] {e}"); return 1; }
    };
    let members = match workspace_members(&root, &manifest) {
        Ok(members) => members,
        Err(e) => { eprintln!("[L++] workspace error: {e}"); return 1; }
    };
    let sub = args.first().map(String::as_str).unwrap_or("members");
    match sub {
        "members" | "list" => {
            println!("[L++] Workspace: {}", root.display());
            if let Some(version) = manifest.get("workspace").and_then(|w| w.get("version")).and_then(toml::Value::as_str) {
                println!("  version: {version}");
            }
            for (relative, path, package) in members {
                println!("  {} @ {} ({})", package.name, package.version, path.strip_prefix(&root).unwrap_or(&path).display());
                let _ = relative;
            }
            0
        }
        "graph" => {
            println!("[L++] Workspace dependency graph: {}", root.display());
            for (_, _, package) in members {
                let name = package.name;
                let deps: Vec<String> = package.dependencies.into_iter().map(|dep| dep.name).collect();
                if deps.is_empty() { println!("  {} -> (none)", name); }
                else { println!("  {} -> {}", name, deps.join(", ")); }
            }
            0
        }
        "build" | "test" => {
            let requested = args.get(1).map(String::as_str);
            let selected: Vec<_> = members.into_iter().filter(|(_, _, package)| requested.map_or(true, |name| package.name == name)).collect();
            if selected.is_empty() {
                eprintln!("[L++] workspace member not found: {}", requested.unwrap_or(""));
                return 1;
            }
            let compiler = match current_compiler_path() {
                Ok(path) => path,
                Err(e) => { eprintln!("[L++] {e}"); return 1; }
            };
            let mut failed = false;
            for (_, path, _) in selected {
                let command = if sub == "build" { "build" } else { "test" };
                println!("[L++] {} {}", command, path.display());
                match std::process::Command::new(&compiler).current_dir(&path).arg(command).status() {
                    Ok(status) if status.success() => {}
                    Ok(status) => { failed = true; eprintln!("[L++] member '{}' failed ({status})", path.display()); }
                    Err(e) => { failed = true; eprintln!("[L++] member '{}' failed: {e}", path.display()); }
                }
            }
            if failed { 1 } else { 0 }
        }
        other => {
            eprintln!("[L++] unknown workspace subcommand '{other}'; use members, graph, build, or test");
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

    if results.is_empty() {
        if query.is_empty() {
            println!("[L++] No packages available in registry.");
        } else {
            println!("[L++] No registry packages matched '{}'.", query);
            println!("Try: lpp search opencode | lpp search sqlite | lpp search math");
        }
        return 0;
    }

    let apps: Vec<_> = results.iter().filter(|(name, _)| is_app_package_name(name)).collect();
    let libs: Vec<_> = results.iter().filter(|(name, _)| !is_app_package_name(name)).collect();
    println!("[L++] Registry search");
    println!("  query: {}", if query.is_empty() { "*" } else { &query });
    println!("  results: {}", results.len());
    println!();
    if !apps.is_empty() {
        println!("Applications / global commands\n──────────────────────────────");
        for (name, entry) in apps { print_search_item(name, entry, true); println!(); }
    }
    if !libs.is_empty() {
        println!("Libraries / project dependencies\n────────────────────────────────");
        for (name, entry) in libs { print_search_item(name, entry, false); println!(); }
    }
    println!("Usage guide\n───────────");
    println!("  Global app:  lpp install lpp-opencode");
    println!("  Project dep: lpp add <name> && lpp install");
    0
}

fn cmd_list() -> i32 {
    match read_manifest() {
        Ok(pkg) => {
            println!("[L++] Package: {} {}", pkg.name, pkg.version);
            if pkg.dependencies.is_empty() {
                println!("  (no dependencies)");
            } else {
                for dep in pkg.dependencies {
                    let source = dep.path.or(dep.git).unwrap_or_else(|| "registry".to_string());
                    let version = dep.version.unwrap_or_else(|| "unbounded".to_string());
                    println!("  {} {} [{}]", dep.name, version, source);
                }
            }
            0
        }
        Err(e) => { eprintln!("[L++] {e}"); 1 }
    }
}

fn cmd_tree() -> i32 {
    let packages = read_lockfile();
    if packages.is_empty() {
        println!("[L++] No lockfile packages found. Run `lpp install` first.");
        return 1;
    }
    println!("[L++] Dependency tree:");
    for pkg in packages {
        let version = pkg.version.unwrap_or_else(|| "unknown".to_string());
        println!("  {} {}", pkg.name, version);
        println!("    source: {}", pkg.source);
        if let Some(resolved) = pkg.resolved { println!("    resolved: {}", resolved); }
    }
    0
}

fn cmd_metadata() -> i32 {
    match read_manifest() {
        Ok(pkg) => {
            println!("name = {}", pkg.name);
            println!("version = {}", pkg.version);
            if let Some(author) = pkg.author { println!("author = {}", author); }
            println!("entry = {}", pkg.entry.unwrap_or_else(resolve_entry_point));
            println!("dependencies = {}", pkg.dependencies.len());
            println!("locked_packages = {}", read_lockfile().len());
            0
        }
        Err(e) => { eprintln!("[L++] {e}"); 1 }
    }
}

fn cmd_outdated() -> i32 {
    let package = match read_manifest() {
        Ok(pkg) => pkg,
        Err(e) => { eprintln!("[L++] {e}"); return 1; }
    };
    let locked: std::collections::HashMap<String, String> = read_lockfile()
        .into_iter()
        .filter_map(|pkg| pkg.version.map(|version| (pkg.name, version)))
        .collect();
    let mut found = false;
    for dep in package.dependencies {
        if dep.version.is_none() {
            found = true;
            println!("{} is not version-pinned", dep.name);
            continue;
        }
        if let Some(locked_version) = locked.get(&dep.name) {
            if let (Ok(current), Ok(requirement)) = (
                semver::Version::parse(locked_version),
                semver::VersionReq::parse(dep.version.as_deref().unwrap_or("*")),
            ) {
                if !requirement.matches(&current) {
                    found = true;
                    println!("{} {} does not satisfy {}", dep.name, current, requirement);
                }
            }
        }
    }
    if !found { println!("[L++] No outdated direct dependencies found."); }
    0
}

fn cmd_clean() -> i32 {
    let mut removed = 0;
    let mut failed = false;
    for target in ["target", "output.c", "output.obj", "output.o"] {
        let path = Path::new(target);
        let result = if path.is_dir() { fs::remove_dir_all(path) } else if path.is_file() { fs::remove_file(path) } else { Ok(()) };
        if result.is_ok() && !path.exists() { removed += 1; }
        if result.is_err() { failed = true; }
    }
    if let Ok(entries) = fs::read_dir(".") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map(|ext| ext == "exe" || ext == "o" || ext == "obj").unwrap_or(false) {
                match fs::remove_file(&path) { Ok(()) => removed += 1, Err(_) => failed = true }
            }
        }
    }
    println!("[L++] Cleaned {} generated artifact(s).", removed);
    if failed { 1 } else { 0 }
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
    println!("[L++] Building project (release={})...", is_release);
    let entry_point_str = resolve_entry_point();
    let entry_point = Path::new(&entry_point_str);
    if !entry_point.exists() {
        eprintln!(
            "[L++] Error: entry point '{}' not found.",
            entry_point.display()
        );
        return None;
    }

    if cmd_install(false) != 0 {
        eprintln!("[L++] dependency installation failed; build aborted");
        return None;
    }

    let target_dir = if is_release {
        PathBuf::from("dist")
    } else {
        Path::new("LppData").join("build").join("release")
    };
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
    if let Ok(pkg) = read_manifest() {
        bin_name = pkg.name;
    }

    let exe_path = output_path_for_name(&target_dir, &bin_name);

    println!("  Linking {}...", exe_path.display());
    let link_result = link_native_binary(&obj_file, &exe_path);
    let _ = fs::remove_file(&obj_file);

    if let Err(e) = link_result {
        eprintln!("[L++] Error: {}", e);
        None
    } else {
        if is_release {
            println!("[L++] Standalone Release build successful: {}", exe_path.display());
            let www_dir = Path::new("www");
            if www_dir.exists() {
                let dist_www = target_dir.join("www");
                if let Err(e) = copy_dir_all(www_dir, &dist_www) {
                    eprintln!("  Warning: failed to bundle www assets into dist/www: {}", e);
                } else {
                    println!("[Lreact] Bundled static web assets into {}", dist_www.display());
                }
            }
        } else {
            println!("[L++] Build successful: {}", exe_path.display());
        }
        Some(exe_path.to_string_lossy().into_owned())
    }
}

fn cmd_dev() -> i32 {
    let entry_point_str = resolve_entry_point();
    if !Path::new(&entry_point_str).exists() {
        eprintln!("[Lreact Dev] Error: entry point '{}' not found.", entry_point_str);
        return 1;
    }
    println!("==========================================================");
    println!("        Lreact Dev Server (L++ Native IPC Backend)       ");
    println!("        Dev URL: http://localhost:3000                   ");
    println!("==========================================================");
    let Some(exe_path) = cmd_build_opts(false) else { return 1; };
    println!("[Lreact Dev] Running native dev server {}...", exe_path);
    match std::process::Command::new(&exe_path).status() {
        Ok(status) => status.code().unwrap_or(if status.success() { 0 } else { 1 }),
        Err(e) => { eprintln!("[Lreact Dev] Execution failed: {e}"); 1 }
    }
}

fn cmd_run() -> i32 {
    let Some(exe_path) = cmd_build() else { return 1; };
    println!("[L++] Running {}...", exe_path);
    match std::process::Command::new(&exe_path).status() {
        Ok(status) => status.code().unwrap_or(if status.success() { 0 } else { 1 }),
        Err(e) => { eprintln!("[L++] Failed to execute target: {e}"); 1 }
    }
}

fn cmd_bench() -> i32 {
    println!("[L++] Launching lpp-bench...");
    let bench_bin = current_binary_dir()
        .map(|dir| dir.join(format!("lpp-bench{}", std::env::consts::EXE_SUFFIX)))
        .filter(|p| p.exists());
    let Some(bench) = bench_bin else {
        eprintln!("[L++] lpp-bench not found. Build it with: cargo build --release --bin lpp-bench");
        return 1;
    };
    let args: Vec<String> = std::env::args().skip(2).collect();
    match std::process::Command::new(&bench).args(&args).status() {
        Ok(status) => status.code().unwrap_or(if status.success() { 0 } else { 1 }),
        Err(e) => { eprintln!("[L++] Failed to launch lpp-bench: {e}"); 1 }
    }
}

fn cmd_test() -> i32 {
    println!("[L++] Running tests...");
    let test_dir = if Path::new("tests").exists() {
        "tests"
    } else if Path::new("test").exists() {
        "test"
    } else {
        println!("[L++] No tests/ or test/ directory found.");
        return 0;
    };

    let paths = match fs::read_dir(test_dir) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Failed to read tests directory: {}", e);
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
        println!("[L++] No test files found in directory '{}'.", test_dir);
        return 0;
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
    if failed == 0 { 0 } else { 1 }
}
