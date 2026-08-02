//! target.rs — target-triple selection for L++.
//!
//! L++ normally targets the host. `--target <triple>` overrides the output
//! architecture/OS/ABI so the compiler can emit for another platform — notably
//! Android (`*-linux-android*`) and Termux (aarch64/armv7 Linux on Android).
//!
//! A target triple is `arch-vendor-os[-abi]`, e.g.
//!   * `x86_64-unknown-linux-gnu`        (host Linux, glibc)
//!   * `aarch64-linux-android`            (Android arm64 / Termux 64-bit)
//!   * `armv7-linux-androideabi`          (Android arm32)
//!   * `i686-linux-android`               (Android x86)
//!   * `aarch64-unknown-linux-gnu`        (generic arm64 Linux)
//!
//! We accept any well-formed triple via `target_lexicon`; the compiler's ISA
//! lookup then selects the matching backend (e.g. Cranelift's aarch64 backend
//! when compiled with `all-arch`). This module additionally classifies the
//! triple so the rest of the pipeline can choose the right runtime and linker
//! behaviour.

use std::str::FromStr;

use target_lexicon::Triple;

/// A validated target selected by the user, or the host.
#[derive(Debug, Clone, Default)]
pub struct TargetSpec {
    /// The raw triple string the user passed (None = host).
    pub raw: Option<String>,
    /// The parsed triple (None = host).
    pub triple: Option<Triple>,
    /// True if the OS is Android (bionic libc).
    pub is_android: bool,
    /// True if a Termux-style target: Linux on a mobile/arm arch with a userspace.
    pub is_termux_like: bool,
    /// Human description for diagnostics.
    pub description: String,
}

/// Architectures treated as "mobile/ARM" where Termux commonly runs.
fn is_termux_arch(arch_str: &str) -> bool {
    arch_str.starts_with("aarch64")
        || arch_str.starts_with("arm")
        || arch_str.starts_with("riscv64")
        || arch_str.starts_with("x86_64")
        || arch_str.starts_with("i686")
}

impl TargetSpec {
    /// Parse a `--target` triple string. Returns an Err with a clear message if
    /// the string is not a well-formed target_lexicon triple.
    pub fn from_triple_str(raw: &str) -> Result<Self, String> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err("empty --target triple".to_string());
        }
        let triple = Triple::from_str(trimmed)
            .map_err(|e| format!("invalid --target triple '{}': {}", trimmed, e))?;
        let os_str = triple.operating_system.to_string();
        let arch_str = triple.architecture.to_string();
        let is_android = os_str.contains("android");
        let is_termux_like =
            is_android || (os_str.contains("linux") && is_termux_arch(&arch_str));
        let description = format!("{} ({})", trimmed, os_str);
        Ok(TargetSpec {
            raw: Some(trimmed.to_string()),
            triple: Some(triple),
            is_android,
            is_termux_like,
            description,
        })
    }

    /// The host target spec.
    pub fn host() -> Self {
        let arch_str = std::env::consts::ARCH;
        TargetSpec {
            raw: None,
            triple: None,
            is_android: std::env::consts::OS == "android",
            is_termux_like: std::env::consts::OS == "android"
                || (std::env::consts::OS == "linux" && is_termux_arch(arch_str)),
            description: format!("host ({} {})", std::env::consts::OS, arch_str),
        }
    }

    /// The effective triple: the user override, or the host triple.
    pub fn effective_triple(&self) -> Triple {
        self.triple.clone().unwrap_or_else(Triple::host)
    }

    /// Return the `cc`/`clang` `-target` string for this target when cross
    /// compiling (None if it is the host, where no flag is needed).
    pub fn cc_target_flag(&self) -> Option<String> {
        self.raw.clone()
    }
}

impl std::fmt::Display for TargetSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.description)
    }
}
