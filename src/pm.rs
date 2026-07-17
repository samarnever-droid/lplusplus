use std::fs;

#[allow(dead_code)]
pub struct Dependency {
    pub name: String,
    pub git: Option<String>,
    pub tag: Option<String>,
    pub branch: Option<String>,
    pub path: Option<String>,
}

#[allow(dead_code)]
pub struct Package {
    pub name: String,
    pub version: String,
    pub author: Option<String>,
    pub dependencies: Vec<Dependency>,
}

pub struct RegistryEntry {
    pub git: String,
    pub branch: Option<String>,
    pub tag: Option<String>,
}

pub fn parse_toml(content: &str) -> Result<Package, String> {
    let mut name = String::new();
    let mut version = String::new();
    let mut author = None;
    let mut dependencies = Vec::new();
    
    let mut current_section = "";
    
    for (line_idx, raw_line) in content.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        
        if line.starts_with('[') && line.ends_with(']') {
            current_section = &line[1..line.len()-1];
            continue;
        }
        
        if let Some(eq_idx) = line.find('=') {
            let key = line[..eq_idx].trim();
            let val_str = line[eq_idx+1..].trim();
            
            match current_section {
                "package" => {
                    let cleaned_val = val_str.trim_matches('"').trim_matches('\'').to_string();
                    if key == "name" {
                        name = cleaned_val;
                    } else if key == "version" {
                        version = cleaned_val;
                    } else if key == "author" {
                        author = Some(cleaned_val);
                    }
                }
                "dependencies" => {
                    if val_str.starts_with('{') && val_str.ends_with('}') {
                        let inline = &val_str[1..val_str.len()-1];
                        let mut git = None;
                        let mut tag = None;
                        let mut branch = None;
                        let mut path = None;
                        
                        for part in inline.split(',') {
                            if let Some(p_eq) = part.find('=') {
                                let pk = part[..p_eq].trim();
                                let pv = part[p_eq+1..].trim().trim_matches('"').trim_matches('\'').trim().to_string();
                                if pk == "git" {
                                    git = Some(pv);
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
                            git,
                            tag,
                            branch,
                            path,
                        });
                    } else {
                        return Err(format!(
                            "Line {}: invalid dependency value '{}'. Must be an inline table {{ ... }}",
                            line_idx + 1, val_str
                        ));
                    }
                }
                _ => {}
            }
        } else {
            return Err(format!("Line {}: invalid TOML syntax '{}'", line_idx + 1, line));
        }
    }
    
    if name.is_empty() {
        return Err("Missing package name in [package] section".to_string());
    }
    
    Ok(Package {
        name,
        version,
        author,
        dependencies,
    })
}

pub fn run_command(args: &[String]) {
    if args.is_empty() {
        print_help();
        return;
    }
    
    match args[0].as_str() {
        "init" => cmd_init(&args[1..]),
        "install" => cmd_install(),
        "add" => cmd_add(&args[1..]),
        "remove" => cmd_remove(&args[1..]),
        "update" => cmd_update(),
        "check" => cmd_check(),
        "build" => {
            let _ = cmd_build();
        }
        "run" => cmd_run(),
        "test" => cmd_test(),
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
    println!("  init <project_name>   Initialize a new project structure and lpp.toml");
    println!("  install               Resolve, download and install all dependencies");
    println!("  add <name>            Add dependency from online registry");
    println!("  add @owner/repo       Add dependency directly from GitHub repository");
    println!("  add <name> --git <U>  Add dependency via explicit git URL");
    println!("  add <name> --path <P> Add dependency via local folder path");
    println!("  remove <name>         Remove a dependency from lpp.toml");
    println!("  update                Update all resolved dependencies");
    println!("  check                 Validate grammar, scope and types in project");
    println!("  build                 Build project into native target executable");
    println!("  run                   Compile and run the project native target");
    println!("  test                  Compile and execute all tests in tests/ folder");
    println!("  help                  Show this help menu");
}

fn cmd_init(args: &[String]) {
    let project_name = args.get(0).map(|s| s.as_str()).unwrap_or("my_project");
    println!("[L++] Initializing new project '{}'...", project_name);
    
    let toml_content = format!(
        "[package]\nname = \"{}\"\nversion = \"0.1.0\"\nauthor = \"Khati\"\n\n[dependencies]\n",
        project_name
    );
    if let Err(e) = fs::write("lpp.toml", toml_content) {
        eprintln!("Failed to write lpp.toml: {}", e);
        return;
    }
    
    if let Err(e) = fs::create_dir_all("src") {
        eprintln!("Failed to create src/ directory: {}", e);
        return;
    }
    
    let main_content = "def main():\n    print_str(\"Hello from L++ project!\")\n";
    if let Err(e) = fs::write("src/main.lpp", main_content) {
        eprintln!("Failed to write src/main.lpp: {}", e);
        return;
    }
    
    let gitignore_content = ".lpp_packages/\ntarget/\noutput.c\noutput.obj\n*.exe\n*.o\n";
    if let Err(e) = fs::write(".gitignore", gitignore_content) {
        eprintln!("Failed to write .gitignore: {}", e);
        return;
    }
    
    println!("[L++] Project '{}' initialized successfully!", project_name);
}

pub fn resolve_from_json(json_str: &str, target_name: &str) -> Option<RegistryEntry> {
    let quoted_target = format!("\"{}\"", target_name);
    if let Some(target_idx) = json_str.find(&quoted_target) {
        let rest = &json_str[target_idx + quoted_target.len()..];
        if let Some(colon_idx) = rest.find(':') {
            let block_rest = &rest[colon_idx+1..];
            if let Some(open_brace) = block_rest.find('{') {
                let entry_content = &block_rest[open_brace+1..];
                if let Some(close_brace) = entry_content.find('}') {
                    let entry_str = &entry_content[..close_brace];
                    
                    let mut git = String::new();
                    let mut branch = None;
                    let mut tag = None;
                    
                    for field_part in entry_str.split(',') {
                        if let Some(eq_idx) = field_part.find(':') {
                            let key = field_part[..eq_idx].trim().trim_matches('"').trim_matches('\'').trim();
                            let val = field_part[eq_idx+1..].trim().trim_matches('"').trim_matches('\'').trim().to_string();
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

pub fn resolve_registry_package(name: &str) -> Option<RegistryEntry> {
    println!("[L++] Querying package registry for '{}'...", name);
    let url = "https://raw.githubusercontent.com/samarnever-droid/Lpp-a-programing-langauge-/master/githubpage/registry.json";
    let cmd_arg = format!("Invoke-RestMethod -Uri '{}' | ConvertTo-Json -Depth 5", url);
    let output = std::process::Command::new("powershell")
        .args(&["-Command", &cmd_arg])
        .output();
        
    if let Ok(out) = output {
        if out.status.success() {
            let json_str = String::from_utf8_lossy(&out.stdout);
            if let Some(entry) = resolve_from_json(&json_str, name) {
                return Some(entry);
            }
        }
    }
    
    let local_registry = std::path::Path::new("githubpage").join("registry.json");
    if local_registry.exists() {
        if let Ok(json_str) = fs::read_to_string(local_registry) {
            if let Some(entry) = resolve_from_json(&json_str, name) {
                return Some(entry);
            }
        }
    }
    
    None
}

fn cmd_install() {
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
                println!("[L++] Resolved '{}' from registry -> {}", dep.name, entry.git);
                dep_git = Some(entry.git);
                dep_branch = entry.branch;
                dep_tag = entry.tag;
            } else {
                eprintln!("[L++] Error: dependency '{}' has no source (git/path) and is not in the registry.", dep.name);
                continue;
            }
        }
        
        if let Some(ref git_url) = dep_git {
            if dest_path.exists() {
                println!("  Updating '{}' from {}...", dep.name, git_url);
                let status = std::process::Command::new("git")
                    .env("GIT_TERMINAL_PROMPT", "0")
                    .args(&["-c", "credential.helper=", "-C", dest_path.to_str().unwrap(), "pull"])
                    .status();
                match status {
                    Ok(s) if s.success() => {},
                    _ => {
                        eprintln!("  Failed to pull updates for '{}'. skipping.", dep.name);
                        continue;
                    }
                }
            } else {
                println!("  Cloning '{}' from {}...", dep.name, git_url);
                let status = std::process::Command::new("git")
                    .env("GIT_TERMINAL_PROMPT", "0")
                    .args(&["-c", "credential.helper=", "clone", git_url, dest_path.to_str().unwrap()])
                    .status();
                match status {
                    Ok(s) if s.success() => {},
                    _ => {
                        eprintln!("  Failed to clone '{}'. skipping.", dep.name);
                        continue;
                    }
                }
            }
            
            if let Some(ref tag) = dep_tag {
                println!("  Checking out tag '{}'...", tag);
                let _ = std::process::Command::new("git")
                    .env("GIT_TERMINAL_PROMPT", "0")
                    .args(&["-c", "credential.helper=", "-C", dest_path.to_str().unwrap(), "checkout", tag])
                    .status();
            } else if let Some(ref branch) = dep_branch {
                println!("  Checking out branch '{}'...", branch);
                let _ = std::process::Command::new("git")
                    .env("GIT_TERMINAL_PROMPT", "0")
                    .args(&["-c", "credential.helper=", "-C", dest_path.to_str().unwrap(), "checkout", branch])
                    .status();
            }
            
            let commit_output = std::process::Command::new("git")
                .env("GIT_TERMINAL_PROMPT", "0")
                .args(&["-c", "credential.helper=", "-C", dest_path.to_str().unwrap(), "rev-parse", "HEAD"])
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
                "[[package]]\nname = \"{}\"\nsource = \"git+{}#{}\"\n\n",
                dep.name, git_url, commit_hash
            ));
        } else if let Some(ref path) = dep.path {
            println!("  Linked path: {}", path);
            let path_ref = std::path::Path::new(path);
            if !path_ref.exists() {
                eprintln!("  [L++] Error: path '{}' for dependency '{}' does not exist.", path, dep.name);
                continue;
            }
            
            lock_content.push_str(&format!(
                "[[package]]\nname = \"{}\"\nsource = \"path+{}\"\n\n",
                dep.name, path
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
        eprintln!("Usage: lpp add <package_name> [--git <url> [--tag <tag>] [--branch <branch>]] [--path <local_path>]");
        return;
    }
    
    let mut package_name = args[0].clone();
    let mut git_url = None;
    let mut tag = None;
    let mut branch = None;
    let mut path = None;
    
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--git" => {
                if i + 1 < args.len() {
                    git_url = Some(args[i+1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --git expects a URL argument");
                    return;
                }
            }
            "--tag" => {
                if i + 1 < args.len() {
                    tag = Some(args[i+1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --tag expects a tag name argument");
                    return;
                }
            }
            "--branch" => {
                if i + 1 < args.len() {
                    branch = Some(args[i+1].clone());
                    i += 2;
                } else {
                    eprintln!("Error: --branch expects a branch name argument");
                    return;
                }
            }
            "--path" => {
                if i + 1 < args.len() {
                    path = Some(args[i+1].clone());
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
                    package_name = package_name[slash_idx+1..].to_string();
                }
            }
        } else {
            eprintln!("Error: Package '{}' not found in registry. You must specify --git or --path to add an unregistered package.", package_name);
            return;
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
        if let Some(ref t) = tag {
            dep_line.push_str(&format!(", tag = \"{}\"", t));
        } else if let Some(ref b) = branch {
            dep_line.push_str(&format!(", branch = \"{}\"", b));
        }
    } else if let Some(ref p) = path {
        dep_line.push_str(&format!("path = \"{}\"", p));
    }
    dep_line.push_str(" }\n");
    
    content.push_str(&dep_line);
    
    if let Err(e) = fs::write("lpp.toml", content) {
        eprintln!("Failed to update lpp.toml: {}", e);
        return;
    }
    
    println!("[L++] Added dependency '{}' to lpp.toml.", package_name);
    cmd_install();
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
        if trimmed.starts_with(&format!("{} =", package_name)) || trimmed.starts_with(&format!("{}=", package_name)) {
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
    
    cmd_install();
}

fn cmd_update() {
    println!("[L++] Updating lockfile and pulling latest dependency updates...");
    cmd_install();
}

fn cmd_check() {
    println!("[L++] Checking project...");
    let entry_point = if std::path::Path::new("src/main.lpp").exists() {
        "src/main.lpp"
    } else if std::path::Path::new("main.lpp").exists() {
        "main.lpp"
    } else {
        eprintln!("[L++] Error: entry point src/main.lpp or main.lpp not found.");
        return;
    };
    
    let home_dir = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\khati".to_string());
    let compiler_path = std::path::Path::new(&home_dir)
        .join(".lpp")
        .join("bin")
        .join("lpp-compiler.exe");
        
    let mut cmd = std::process::Command::new(&compiler_path);
    cmd.arg(entry_point).arg("--check");
    
    let status = cmd.status();
    match status {
        Ok(s) if s.success() => {
            println!("[L++] Project is semantically valid.");
        }
        _ => {
            eprintln!("[L++] Error: Project check failed.");
        }
    }
}

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
            Err(std::io::Error::new(std::io::ErrorKind::Other, "Failed to write temp batch file"))
        };
            
        match output {
            Ok(out) if out.status.success() => {
                let env_dump = String::from_utf8_lossy(&out.stdout);
                let mut loaded_count = 0;
                for line in env_dump.lines() {
                    if let Some(eq_idx) = line.find('=') {
                        let name = &line[..eq_idx];
                        let val = &line[eq_idx+1..];
                        unsafe { std::env::set_var(name, val); }
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

fn cmd_build() -> Option<String> {
    load_msvc_env();
    println!("[L++] Building project...");
    let entry_point = if std::path::Path::new("src/main.lpp").exists() {
        "src/main.lpp"
    } else if std::path::Path::new("main.lpp").exists() {
        "main.lpp"
    } else {
        eprintln!("[L++] Error: entry point src/main.lpp or main.lpp not found.");
        return None;
    };
    
    cmd_install();
    
    let target_dir = std::path::Path::new("target").join("release");
    let _ = fs::create_dir_all(&target_dir);
    
    let home_dir = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\khati".to_string());
    let compiler_path = std::path::Path::new(&home_dir)
        .join(".lpp")
        .join("bin")
        .join("lpp-compiler.exe");
        
    let mut cmd = std::process::Command::new(&compiler_path);
    cmd.env("LPP_AOT", "1")
       .env("BENCHMARK", "1")
       .arg(entry_point);
       
    println!("  Compiling {}...", entry_point);
    let status = cmd.status();
    match status {
        Ok(s) if s.success() => {},
        _ => {
            eprintln!("[L++] Error: Compilation failed.");
            return None;
        }
    }
    
    let obj_file = entry_point.replace(".lpp", ".o");
    if !std::path::Path::new(&obj_file).exists() {
        eprintln!("[L++] Error: Compiled object file {} not found.", obj_file);
        return None;
    }
    
    let mut bin_name = "output".to_string();
    if std::path::Path::new("lpp.toml").exists() {
        if let Ok(content) = fs::read_to_string("lpp.toml") {
            if let Ok(pkg) = parse_toml(&content) {
                bin_name = pkg.name;
            }
        }
    }
    
    let exe_path = target_dir.join(format!("{}.exe", bin_name));
    
    let runtime_obj = std::path::Path::new(&home_dir)
        .join(".lpp")
        .join("lib")
        .join("lpp_runtime.obj");
        
    let runtime_src = std::path::Path::new(&home_dir)
        .join(".lpp")
        .join("lib")
        .join("lpp_runtime.c");
        
    let mut link_cmd = std::process::Command::new("link.exe");
    link_cmd.arg("/nologo")
            .arg(&obj_file)
            .arg(runtime_obj.to_str().unwrap())
            .arg(format!("/out:{}", exe_path.to_str().unwrap()))
            .arg("/SUBSYSTEM:CONSOLE");
            
    println!("  Linking {}...", exe_path.display());
    let link_status = link_cmd.status();
    
    let success = match link_status {
        Ok(s) if s.success() => true,
        _ => {
            let mut cl_cmd = std::process::Command::new("cl.exe");
            cl_cmd.arg("/nologo")
                  .arg("/O2")
                  .arg(&obj_file)
                  .arg(runtime_src.to_str().unwrap())
                  .arg(format!("/Fe:{}", exe_path.to_str().unwrap()));
            matches!(cl_cmd.status(), Ok(s) if s.success())
        }
    };
    
    let _ = fs::remove_file(obj_file);
    
    if success {
        println!("[L++] Build successful: {}", exe_path.display());
        Some(exe_path.to_str().unwrap().to_string())
    } else {
        eprintln!("[L++] Error: Linking failed. Make sure MSVC build tools are in your PATH.");
        None
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

fn cmd_test() {
    load_msvc_env();
    println!("[L++] Running tests...");
    let test_dir = if std::path::Path::new("tests").exists() {
        "tests"
    } else if std::path::Path::new("test").exists() {
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
    
    let home_dir = std::env::var("USERPROFILE").unwrap_or_else(|_| "C:\\Users\\khati".to_string());
    let compiler_path = std::path::Path::new(&home_dir)
        .join(".lpp")
        .join("bin")
        .join("lpp-compiler.exe");
    let runtime_obj = std::path::Path::new(&home_dir)
        .join(".lpp")
        .join("lib")
        .join("lpp_runtime.obj");
        
    let target_test_dir = std::path::Path::new("target").join("test");
    let _ = fs::create_dir_all(&target_test_dir);
    
    for test_path in test_files {
        let test_name = test_path.file_name().unwrap().to_str().unwrap();
        print!("  test {} ... ", test_name);
        
        let temp_exe = target_test_dir.join(format!("test_{}.exe", test_name.replace(".lpp", "")));
        let temp_obj = test_path.with_extension("o");
        
        let comp_status = std::process::Command::new(&compiler_path)
            .env("LPP_AOT", "1")
            .env("BENCHMARK", "1")
            .arg(&test_path)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
            
        if matches!(comp_status, Ok(s) if s.success()) && temp_obj.exists() {
            let mut link_cmd = std::process::Command::new("link.exe");
            link_cmd.arg("/nologo")
                    .arg(&temp_obj)
                    .arg(runtime_obj.to_str().unwrap())
                    .arg(format!("/out:{}", temp_exe.to_str().unwrap()))
                    .arg("/SUBSYSTEM:CONSOLE");
            
            let link_success = match link_cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status() {
                Ok(s) if s.success() => true,
                _ => {
                    let runtime_src = std::path::Path::new(&home_dir)
                        .join(".lpp")
                        .join("lib")
                        .join("lpp_runtime.c");
                    let mut cl_cmd = std::process::Command::new("cl.exe");
                    cl_cmd.arg("/nologo")
                          .arg("/O2")
                          .arg(&temp_obj)
                          .arg(runtime_src.to_str().unwrap())
                          .arg(format!("/Fe:{}", temp_exe.to_str().unwrap()));
                    matches!(cl_cmd.stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).status(), Ok(s) if s.success())
                }
            };
            
            let _ = fs::remove_file(&temp_obj);
            
            if link_success && temp_exe.exists() {
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
        } else {
            println!("FAILED (compilation failed)");
            failed += 1;
        }
    }
    
    println!("\ntest result: {}. {} passed; {} failed", if failed == 0 { "ok" } else { "FAILED" }, passed, failed);
}
