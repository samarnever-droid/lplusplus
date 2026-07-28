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

#[derive(Debug, Clone, serde::Deserialize)]
pub struct RegistryEntry {
    #[serde(alias = "repository")]
    pub git: String,
    pub branch: Option<String>,
    pub tag: Option<String>,
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

pub fn parse_json_manifest(content: &str) -> Result<Package, String> {
    let val: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| format!("JSON syntax error in manifest: {e}"))?;

    let name = val
        .get("name")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "Missing 'name' in lpp.json".to_string())?
        .to_string();

    let version = val
        .get("version")
        .and_then(|v| v.as_str())
        .unwrap_or("0.1.0")
        .to_string();

    let author = val.get("author").and_then(|v| v.as_str()).map(String::from);

    let entry = val
        .get("main")
        .or_else(|| val.get("entry"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let mut dependencies = Vec::new();
    if let Some(deps) = val.get("dependencies").and_then(|d| d.as_object()) {
        for (dep_name, dep_val) in deps {
            let mut version = None;
            let mut git = None;
            let mut tag = None;
            let mut branch = None;
            let mut path = None;

            if let Some(v_str) = dep_val.as_str() {
                if v_str.starts_with("http://")
                    || v_str.starts_with("https://")
                    || v_str.ends_with(".git")
                {
                    git = Some(v_str.to_string());
                } else if v_str.starts_with("./") || v_str.starts_with("../") {
                    path = Some(v_str.to_string());
                } else {
                    version = Some(v_str.to_string());
                }
            } else if let Some(obj) = dep_val.as_object() {
                version = obj.get("version").and_then(|v| v.as_str()).map(String::from);
                git = obj.get("git").and_then(|v| v.as_str()).map(String::from);
                tag = obj.get("tag").and_then(|v| v.as_str()).map(String::from);
                branch = obj.get("branch").and_then(|v| v.as_str()).map(String::from);
                path = obj.get("path").and_then(|v| v.as_str()).map(String::from);
            }

            dependencies.push(Dependency {
                name: dep_name.clone(),
                version,
                git,
                tag,
                branch,
                path,
            });
        }
    }

    Ok(Package {
        name,
        version,
        author,
        entry,
        dependencies,
    })
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
                    let description = v.get("description").and_then(|d| d.as_str()).map(String::from);
                    entries.push((k.clone(), RegistryEntry { git, branch, tag, description }));
                }
            }
        }
    }
    entries
}

fn registry_package_names() -> Vec<String> {
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

    // 1. Prebuilt runtime shipped with the toolchain (release tarball / install
    //    layout). These objects are never rebuilt, so an installed toolchain
    //    never needs a C compiler on the direct-link path.
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

    // 3. Shared user cache: compiled from source once (hash-invalidated when
    //    the runtime source changes) and reused by every directory/project.
    let cache_dir = shared_runtime_cache_dir()?;
    let cache_obj = cache_dir.join(&filename);
    let cache_hash = cache_dir.join("runtime.hash");

    if let Some(src_path) = resolve_min_runtime_source() {
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
                    .arg("-ffreestanding")
                    .arg("-fno-pic")
                    .arg("-mno-red-zone")
                    .arg("-fno-reorder-blocks-and-partition")
                    .arg("-DLPP_FREESTANDING")
                    .arg("-c")
                    .arg(&src_path)
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

    None
}


/// Link using the host C compiler (cc / cl.exe) with optional -l flags for FFI
pub fn host_link_binary(obj_file: &Path, output_path: &Path, link_libs: &[String]) -> Result<(), String> {
    let cc = if cfg!(windows) { "cl.exe" } else { "cc" };
    let mut cmd = std::process::Command::new(cc);
    if cfg!(windows) {
        cmd.arg("/nologo")
            .arg(obj_file);
        for lib in link_libs {
            cmd.arg(format!("{}.lib", lib));
        }
        if let Some(runtime_src_path) = resolve_runtime_source() {
            cmd.arg(&runtime_src_path);
            cmd.arg("ws2_32.lib");
            cmd.arg("user32.lib");
            cmd.arg("gdi32.lib");
        }
        cmd.arg(format!("/Fe:{}", output_path.display()));
    } else {
        cmd.arg(obj_file)
            .arg("-o")
            .arg(output_path)
            .arg("-lm"); // always link math
        for lib in link_libs {
            cmd.arg(format!("-l{}", lib));
        }
        if let Some(runtime_src_path) = resolve_runtime_source() {
            cmd.arg(&runtime_src_path);
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

pub fn run_command(args: &[String]) {
    if args.is_empty() {
        print_help();
        return;
    }

    match args[0].as_str() {
        "lreact" => {
            let sub = args.get(1).map(|s| s.as_str()).unwrap_or("help");
            match sub {
                "create" | "new" => {
                    let mut web_args = vec!["web".to_string()];
                    web_args.extend(args.iter().skip(2).cloned());
                    cmd_new(&web_args);
                }
                "dev" | "run" => cmd_dev(),
                "build" => {
                    let _ = cmd_build_opts(true);
                }
                _ => {
                    println!("Lreact Framework CLI Commands:");
                    println!("  lpp lreact create <name>   Create a new Lreact web desktop application");
                    println!("  lpp lreact dev             Start local dev server (http://localhost:3000)");
                    println!("  lpp lreact build           Build standalone release executable & assets in dist/");
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
        "list" => cmd_list(),
        "tree" => cmd_tree(),
        "metadata" => cmd_metadata(),
        "outdated" => cmd_outdated(),
        "clean" => cmd_clean(),
        "check" => cmd_check(),
        "build" => {
            let is_release = args.iter().any(|a| a == "--release");
            let _ = cmd_build_opts(is_release);
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
    println!("L++ Package Manager v4.4.0");
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
    println!("  outdated                          Show unpinned dependencies");
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

fn cmd_new(args: &[String]) {
    let mut is_web = false;
    let mut name_arg = None;

    for arg in args {
        if arg == "web" || arg == "--web" || arg == "lreact" {
            is_web = true;
        } else if !arg.starts_with('-') {
            name_arg = Some(arg.as_str());
        }
    }

    let raw_name = name_arg.unwrap_or("my_app");
    let package_name = normalize_package_name(raw_name);
    let project_dir = PathBuf::from(raw_name);

    if project_dir.exists() {
        eprintln!(
            "[L++] Error: directory '{}' already exists.",
            project_dir.display()
        );
        return;
    }

    if is_web {
        println!("[Lreact] Creating new Lreact Web App '{}'...", raw_name);
        if let Err(e) = fs::create_dir_all(&project_dir) {
            eprintln!("Failed to create project directory: {}", e);
            return;
        }
        match write_web_scaffold(&project_dir, &package_name) {
            Ok(()) => {
                println!("[Lreact] Web App '{}' created at {}.", package_name, project_dir.display());
                println!("[L++] Installing lreact dependency into {}...", project_dir.display());
                let old_cwd = std::env::current_dir().ok();
                if std::env::set_current_dir(&project_dir).is_ok() {
                    cmd_install(false);
                    if let Some(old) = old_cwd {
                        let _ = std::env::set_current_dir(old);
                    }
                }
                println!("\nNext steps:");
                println!("  cd {}", raw_name);
                println!("  lpp dev              # Start local Lreact dev server at http://localhost:3000");
                println!("  lpp build --release  # Build standalone native executable in dist/");
            }
            Err(e) => eprintln!("{}", e),
        }
    } else {
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
                    let description = v.get("description").and_then(|d| d.as_str()).map(String::from);
                    return Some(RegistryEntry { git, branch, tag, description });
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

fn install_lpp_opencode_global() {
    println!("[L++] Installing lpp-opencode globally...");
    #[cfg(windows)]
    {
        let script = "irm https://raw.githubusercontent.com/samarnever-droid/lpp-opencode/main/scripts/install.ps1 | iex";
        let status = std::process::Command::new("powershell")
            .args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", script])
            .status();
        match status {
            Ok(s) if s.success() => println!("[L++] lpp-opencode installed. Run: lpp-opencode /provider"),
            Ok(s) => eprintln!("[L++] lpp-opencode installer exited with status {}", s),
            Err(e) => eprintln!("[L++] Failed to run PowerShell installer: {}", e),
        }
        return;
    }
    #[cfg(not(windows))]
    {
        let script = "curl -fsSL https://raw.githubusercontent.com/samarnever-droid/lpp-opencode/main/scripts/install.sh | sh";
        let status = std::process::Command::new("sh").args(["-c", script]).status();
        match status {
            Ok(s) if s.success() => println!("[L++] lpp-opencode installed. Run: lpp-opencode /provider"),
            Ok(s) => eprintln!("[L++] lpp-opencode installer exited with status {}", s),
            Err(e) => eprintln!("[L++] Failed to run shell installer: {}", e),
        }
    }
}

fn cmd_install_command(args: &[String]) {
    if !args.is_empty() && !Path::new("lpp.toml").exists() {
        let package = &args[0];
        if is_lpp_opencode_alias(package) {
            install_lpp_opencode_global();
            return;
        }
        eprintln!("[L++] No lpp.toml found here, so this is not a project dependency install.");
        eprintln!("[L++] '{}' is not a known global app alias.", package);
        eprintln!("");
        eprintln!("What you probably want:");
        eprintln!("  Global app:        lpp install lpp-opencode");
        eprintln!("  Search registry:   lpp search {}", package);
        eprintln!("  New project:       lpp new my_app && cd my_app && lpp add {}", package);
        eprintln!("");
        eprintln!("Known global app aliases: lpp-opencode, opencode, openclaude, lpp-openclaude");
        return;
    }
    cmd_install(false);
}

fn cmd_install(force_update: bool) {
    println!("[L++] Resolving dependencies...");
    let package = match read_manifest() {
        Ok(pkg) => pkg,
        Err(e) => {
            eprintln!("[L++] Manifest error: {}", e);
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
    let mut worklist = package.dependencies;
    let mut processed = std::collections::HashSet::new();

    while let Some(dep) = worklist.pop() {
        if !processed.insert(dep.name.clone()) {
            continue;
        }

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

        let installed_successfully = if let Some(ref git_url) = dep_git {
            let mut git_checkout_needed = false;
            let mut clone_ok = true;
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
                            clone_ok = false;
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
                        clone_ok = false;
                    }
                }
            }

            if clone_ok && git_checkout_needed {
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

            if clone_ok {
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
                true
            } else {
                false
            }
        } else if let Some(ref path) = dep.path {
            println!("  Linked path: {}", path);
            let path_ref = std::path::Path::new(path);
            if !path_ref.exists() {
                eprintln!(
                    "  [L++] Error: path '{}' for dependency '{}' does not exist.",
                    path, dep.name
                );
                false
            } else {
                lock_content.push_str(&format!(
                    "[[package]]\nname = \"{}\"\nversion = \"{}\"\nsource = \"path+{}\"\nresolved = \"{}\"\n\n",
                    dep.name,
                    dep.version.clone().unwrap_or_else(|| "workspace".to_string()),
                    path,
                    path_ref.display()
                ));
                true
            }
        } else {
            false
        };

        if installed_successfully {
            let sub_pkg_res = if dest_path.join("lpp.json").exists() {
                fs::read_to_string(dest_path.join("lpp.json"))
                    .ok()
                    .and_then(|c| parse_json_manifest(&c).ok())
            } else if dest_path.join("lpp.toml").exists() {
                fs::read_to_string(dest_path.join("lpp.toml"))
                    .ok()
                    .and_then(|c| parse_toml(&c).ok())
            } else {
                None
            };
            if let Some(sub_pkg) = sub_pkg_res {
                for sub_dep in sub_pkg.dependencies {
                    if !processed.contains(&sub_dep.name) {
                        worklist.push(sub_dep);
                    }
                }
            }
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

fn is_app_package_name(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n == "lpp-opencode" || n == "opencode" || n == "openclaude" || n == "lpp-openclaude"
}

fn print_search_item(name: &str, entry: &RegistryEntry, app: bool) {
    println!("  {}", name);
    if let Some(ref desc) = entry.description {
        println!("      {}", desc);
    }
    println!("      repo: {}", entry.git);
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

fn cmd_search(args: &[String]) {
    let query = args.get(0).map(|s| s.to_lowercase()).unwrap_or_default();
    let mut results = registry_package_entries();
    results.sort_by(|a, b| a.0.cmp(&b.0));

    if !query.is_empty() {
        results.retain(|(name, entry)| {
            name.to_lowercase().contains(&query)
                || entry.git.to_lowercase().contains(&query)
                || entry
                    .description
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&query)
        });
    }

    if results.is_empty() {
        if query.is_empty() {
            println!("[L++] No packages available in registry.");
        } else {
            println!("[L++] No registry packages matched '{}'.", query);
            println!();
            println!("Try:");
            println!("  lpp search opencode     # app / command package");
            println!("  lpp search sqlite       # database library");
            println!("  lpp search math         # stdlib/helper package");
        }
        return;
    }

    let apps: Vec<_> = results
        .iter()
        .filter(|(name, _)| is_app_package_name(name))
        .collect();
    let libs: Vec<_> = results
        .iter()
        .filter(|(name, _)| !is_app_package_name(name))
        .collect();

    println!("[L++] Registry search");
    println!("  query: {}", if query.is_empty() { "*" } else { &query });
    println!("  results: {}", results.len());
    println!();

    if !apps.is_empty() {
        println!("Applications / global commands");
        println!("──────────────────────────────");
        for (name, entry) in apps {
            print_search_item(name, entry, true);
            println!();
        }
    }

    if !libs.is_empty() {
        println!("Libraries / project dependencies");
        println!("────────────────────────────────");
        for (name, entry) in libs {
            print_search_item(name, entry, false);
            println!();
        }
    }

    println!("Usage guide");
    println!("───────────");
    println!("  Global app:  lpp install lpp-opencode");
    println!("  Project dep: lpp add <name>  &&  lpp install");
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

    cmd_install(false);

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

fn cmd_dev() {
    let entry_point_str = resolve_entry_point();
    let entry_point = Path::new(&entry_point_str);
    if !entry_point.exists() {
        eprintln!("[Lreact Dev] Error: entry point '{}' not found.", entry_point_str);
        return;
    }

    let pkg_dir = Path::new(".lpp_packages");
    if !pkg_dir.exists() {
        cmd_install(false);
    }

    println!("==========================================================");
    println!("        Lreact Dev Server (L++ Native IPC Backend)       ");
    println!("        Dev URL: http://localhost:3000                   ");
    println!("==========================================================");

    if let Some(exe_path) = cmd_build_opts(false) {
        println!("[Lreact Dev] Running native dev server {}...", exe_path);
        let status = std::process::Command::new(&exe_path).status();
        if let Err(e) = status {
            eprintln!("[Lreact Dev] Execution failed: {}", e);
        }
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
