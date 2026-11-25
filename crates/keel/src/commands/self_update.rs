//! `keel self-update` (alias `keel upgrade`) — check for a newer L++ & Keel
//! release and optionally install it via the SHA-256 verified release installer.

use std::process::Command;

const LATEST_RELEASE_API: &str =
    "https://api.github.com/repos/samarnever-droid/lplusplus/releases/latest";

fn fetch_latest_tag() -> Result<String, String> {
    #[cfg(windows)]
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "(Invoke-RestMethod -Uri '{LATEST_RELEASE_API}' -Headers @{{'User-Agent'='keel-updater'}}).tag_name"
            ),
        ])
        .output()
        .map_err(|e| format!("failed to run PowerShell: {e}"))?;

    #[cfg(not(windows))]
    let output = Command::new("curl")
        .args([
            "-fsSL",
            "-H",
            "User-Agent: keel-updater",
            "-H",
            "Accept: application/vnd.github+json",
            LATEST_RELEASE_API,
        ])
        .output()
        .map_err(|e| format!("failed to run curl: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("failed to query latest release: {}", stderr.trim()));
    }

    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    #[cfg(windows)]
    {
        if stdout.is_empty() {
            return Err("empty tag_name returned from GitHub Releases".to_string());
        }
        return Ok(stdout);
    }

    #[cfg(not(windows))]
    {
        extract_json_string(&stdout, "tag_name")
            .ok_or_else(|| "missing tag_name in GitHub Releases response".to_string())
    }
}

#[cfg(not(windows))]
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let start = json.find(&needle)? + needle.len();
    let rest = json[start..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

pub fn self_update(check_only: bool) -> Result<(), String> {
    let current_str = env!("CARGO_PKG_VERSION");
    println!("Keel & L++ Updater");
    println!("Installed version: v{current_str}");

    let latest_tag = fetch_latest_tag()?;
    let latest_str = latest_tag.trim_start_matches('v');
    let current = lpp_pm::Version::parse(current_str)
        .ok_or_else(|| format!("invalid installed version `{current_str}`"))?;
    let latest = lpp_pm::Version::parse(latest_str)
        .ok_or_else(|| format!("invalid release tag `{latest_tag}`"))?;

    println!("Latest release:    v{latest_str}");

    if latest <= current {
        println!("You are already on the latest version (v{current_str}).");
        return Ok(());
    }

    println!("New version found: v{current_str} -> v{latest_str}!");

    if check_only {
        #[cfg(windows)]
        println!("Run `keel self-update` or: irm https://lplusplus.bond/install.ps1 | iex");
        #[cfg(not(windows))]
        println!("Run `keel self-update` or: curl -fsSL https://lplusplus.bond/install.sh | sh");
        return Ok(());
    }

    println!("Installing verified release v{latest_str} (lpp, lpp-link, keel)...");
    #[cfg(windows)]
    let status = Command::new("powershell")
        .args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &format!(
                "$env:LPP_VERSION='{latest_tag}'; irm https://lplusplus.bond/install.ps1 | iex"
            ),
        ])
        .status()
        .map_err(|e| format!("failed to launch PowerShell installer: {e}"))?;

    #[cfg(not(windows))]
    let status = Command::new("sh")
        .args([
            "-c",
            &format!(
                "LPP_VERSION='{latest_tag}' curl -fsSL https://lplusplus.bond/install.sh | LPP_VERSION='{latest_tag}' sh"
            ),
        ])
        .status()
        .map_err(|e| format!("failed to launch shell installer: {e}"))?;

    if !status.success() {
        return Err(format!("installer exited with status {status}"));
    }
    println!("Updated L++ and Keel to v{latest_str}.");
    Ok(())
}
