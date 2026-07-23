//! L++ user configuration — stored in ~/.lpp/config.json
//!
//! Created on first run with auto-detected defaults.
//! Users can edit the JSON or use `lpp config` commands.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LppConfig {
    /// Which linker to use: "direct" (lpp-link) or "host" (cc/cl.exe)
    pub linker: String,

    /// Detected system info (populated on first run)
    pub system: SystemInfo,

    /// Linker benchmark results (populated on first run)
    pub linker_benchmarks: Option<LinkerBenchmarks>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemInfo {
    pub os: String,
    pub arch: String,
    pub has_cc: bool,
    pub has_msvc: bool,
    pub has_lpp_link: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkerBenchmarks {
    pub direct_available: bool,
    pub host_available: bool,
    pub direct_label: String,
    pub host_label: String,
    pub recommendation: String,
}

impl Default for LppConfig {
    fn default() -> Self {
        Self {
            linker: "auto".to_string(),
            system: SystemInfo::detect(),
            linker_benchmarks: None,
        }
    }
}

impl SystemInfo {
    pub fn detect() -> Self {
        let os = if cfg!(target_os = "windows") {
            "windows"
        } else if cfg!(target_os = "macos") {
            "macos"
        } else {
            "linux"
        }
        .to_string();

        let arch = if cfg!(target_arch = "x86_64") {
            "x86_64"
        } else if cfg!(target_arch = "aarch64") {
            "aarch64"
        } else {
            "unknown"
        }
        .to_string();

        // Detect host C compiler
        let has_cc = if cfg!(target_os = "windows") {
            // Check for cl.exe via MSVC
            std::process::Command::new("cl.exe")
                .arg("/?")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok()
        } else {
            std::process::Command::new("cc")
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok()
        };

        let has_msvc = if cfg!(target_os = "windows") {
            has_cc // on Windows, has_cc means MSVC
        } else {
            false
        };

        // Check if lpp-link is available (should be alongside lpp binary)
        let has_lpp_link = if let Ok(exe) = std::env::current_exe() {
            let dir = exe.parent().unwrap_or(Path::new("."));
            let link_name = if cfg!(target_os = "windows") {
                "lpp-link.exe"
            } else {
                "lpp-link"
            };
            dir.join(link_name).exists()
        } else {
            false
        };

        Self {
            os,
            arch,
            has_cc,
            has_msvc,
            has_lpp_link,
        }
    }
}

impl LppConfig {
    /// Get config file path: ~/.lpp/config.json
    pub fn path() -> PathBuf {
        let home = std::env::var("HOME")
            .or_else(|_| std::env::var("USERPROFILE"))
            .unwrap_or_else(|_| ".".to_string());
        PathBuf::from(home).join(".lpp").join("config.json")
    }

    /// Load config from disk, or create default on first run
    pub fn load_or_create() -> Self {
        let path = Self::path();
        if path.exists() {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if let Ok(config) = serde_json::from_str::<LppConfig>(&content) {
                    return config;
                }
            }
        }
        // First run — create default config
        let config = Self::create_default();
        let _ = config.save();
        config
    }

    /// Create config with auto-detected defaults
    fn create_default() -> Self {
        let system = SystemInfo::detect();

        // Determine best linker default
        let (linker, benchmarks) = Self::recommend_linker(&system);

        Self {
            linker,
            system,
            linker_benchmarks: Some(benchmarks),
        }
    }

    /// Recommend linker based on system capabilities
    fn recommend_linker(system: &SystemInfo) -> (String, LinkerBenchmarks) {
        let direct_available = system.has_lpp_link;
        let host_available = system.has_cc;

        let (recommendation, linker) = if direct_available && !host_available {
            (
                "Using lpp-link (direct) — no host C compiler found".to_string(),
                "direct".to_string(),
            )
        } else if !direct_available && host_available {
            (
                "Using host linker — lpp-link not found".to_string(),
                "host".to_string(),
            )
        } else if direct_available && host_available {
            // Both available — recommend direct for zero-dependency builds
            (
                "Both available. Using lpp-link (direct) for zero-dependency native builds. \
                 Set \"linker\": \"host\" in ~/.lpp/config.json to use system cc/cl.exe instead."
                    .to_string(),
                "direct".to_string(),
            )
        } else {
            (
                "WARNING: No linker found! Install a C compiler (gcc/clang/MSVC) or ensure lpp-link is in PATH."
                    .to_string(),
                "host".to_string(),
            )
        };

        let direct_label = if system.os == "windows" {
            "lpp-link pe (freestanding, ~18KB exe, no MSVC needed)"
        } else if system.os == "macos" {
            "lpp-link macho (direct Mach-O)"
        } else {
            "lpp-link elf (freestanding, ~8KB exe, no libc)"
        }
        .to_string();

        let host_label = if system.os == "windows" {
            "cl.exe / link.exe (MSVC, full CRT, ~100KB+ exe)"
        } else if system.os == "macos" {
            "clang (Xcode, full libc)"
        } else {
            "cc / gcc / clang (system, full libc, ~20KB+ exe)"
        }
        .to_string();

        (
            linker,
            LinkerBenchmarks {
                direct_available,
                host_available,
                direct_label,
                host_label,
                recommendation,
            },
        )
    }

    /// Save config to disk
    pub fn save(&self) -> Result<(), String> {
        let path = Self::path();
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("create config dir: {e}"))?;
        }
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| format!("serialize config: {e}"))?;
        std::fs::write(&path, json)
            .map_err(|e| format!("write config: {e}"))?;
        Ok(())
    }

    /// Should we use the direct linker?
    pub fn use_direct_linker(&self) -> bool {
        match self.linker.as_str() {
            "direct" => true,
            "host" => false,
            "auto" | _ => {
                // Auto: prefer direct if available
                self.system.has_lpp_link
            }
        }
    }

    /// Print config summary
    pub fn print_summary(&self) {
        println!("L++ Configuration ({})", Self::path().display());
        println!("  OS:          {}", self.system.os);
        println!("  Arch:        {}", self.system.arch);
        println!("  Linker:      {}", self.linker);
        println!("  lpp-link:    {}", if self.system.has_lpp_link { "found" } else { "not found" });
        println!("  Host cc:     {}", if self.system.has_cc { "found" } else { "not found" });
        if let Some(ref bench) = self.linker_benchmarks {
            println!("  Direct:      {}", bench.direct_label);
            println!("  Host:        {}", bench.host_label);
            println!("  Recommend:   {}", bench.recommendation);
        }
    }
}
