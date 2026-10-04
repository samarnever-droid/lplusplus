//! Typed target selection shared by the driver and compiler stages.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use target_lexicon::Triple;

/// Broad target family used for early capability selection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TargetFamily {
    Native,
    WasmWasi,
    WasmUnknown,
}

/// WebAssembly triples recognized by the compatibility compiler.
#[must_use]
pub fn is_wasm_triple_str(raw: &str) -> bool {
    matches!(
        raw.trim(),
        "wasm32-wasi" | "wasm32-wasip1" | "wasm32-unknown-unknown"
    )
}

/// A validated target selected by the user, or the host.
#[derive(Debug, Clone)]
pub struct TargetSpec {
    /// Original user spelling. `None` denotes the current host.
    pub raw: Option<String>,
    /// Parsed native triple. The WebAssembly compatibility aliases are modeled
    /// by `family` because target-lexicon 0.12 predates `wasip1`.
    pub _triple: Option<Triple>,
    pub _is_android: bool,
    pub _is_termux_like: bool,
    pub description: String,
    family: TargetFamily,
}

fn is_termux_arch(architecture: &str) -> bool {
    architecture.starts_with("aarch64")
        || architecture.starts_with("arm")
        || architecture.starts_with("riscv64")
        || architecture.starts_with("x86_64")
        || architecture.starts_with("i686")
}

impl TargetSpec {
    pub fn from_triple_str(raw: &str) -> Result<Self, TargetError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(TargetError::Empty);
        }
        if is_wasm_triple_str(trimmed) {
            let family = if trimmed == "wasm32-unknown-unknown" {
                TargetFamily::WasmUnknown
            } else {
                TargetFamily::WasmWasi
            };
            return Ok(Self {
                raw: Some(trimmed.to_string()),
                _triple: None,
                _is_android: false,
                _is_termux_like: false,
                description: format!("{trimmed} (WebAssembly)"),
                family,
            });
        }

        let triple = Triple::from_str(trimmed).map_err(|error| TargetError::Invalid {
            triple: trimmed.to_string(),
            reason: error.to_string(),
        })?;
        let operating_system = triple.operating_system.to_string();
        let environment = triple.environment.to_string();
        let architecture = triple.architecture.to_string();
        let is_android = operating_system.contains("android")
            || environment.contains("android")
            || trimmed.ends_with("-android")
            || trimmed.ends_with("-androideabi");
        let is_termux_like =
            is_android || (operating_system.contains("linux") && is_termux_arch(&architecture));
        Ok(Self {
            raw: Some(trimmed.to_string()),
            _triple: Some(triple),
            _is_android: is_android,
            _is_termux_like: is_termux_like,
            description: format!("{trimmed} ({operating_system})"),
            family: TargetFamily::Native,
        })
    }

    #[must_use]
    pub fn host() -> Self {
        let architecture = std::env::consts::ARCH;
        Self {
            raw: None,
            _triple: None,
            _is_android: std::env::consts::OS == "android",
            _is_termux_like: std::env::consts::OS == "android"
                || (std::env::consts::OS == "linux" && is_termux_arch(architecture)),
            description: format!("host ({} {architecture})", std::env::consts::OS),
            family: TargetFamily::Native,
        }
    }

    #[must_use]
    pub const fn family(&self) -> TargetFamily {
        self.family
    }

    #[must_use]
    pub fn is_host(&self) -> bool {
        self.raw.is_none()
    }

    #[must_use]
    pub fn _effective_triple(&self) -> Triple {
        self._triple.clone().unwrap_or_else(Triple::host)
    }

    #[must_use]
    pub fn _cc_target_flag(&self) -> Option<String> {
        self.raw.clone()
    }
}

impl Default for TargetSpec {
    fn default() -> Self {
        Self::host()
    }
}

impl fmt::Display for TargetSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.description)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TargetError {
    Empty,
    Invalid { triple: String, reason: String },
}

impl fmt::Display for TargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => formatter.write_str("empty --target triple"),
            Self::Invalid { triple, reason } => {
                write!(formatter, "invalid --target triple '{triple}': {reason}")
            }
        }
    }
}

impl std::error::Error for TargetError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_legacy_wasm_aliases() {
        let wasi = TargetSpec::from_triple_str(" wasm32-wasip1 ").unwrap();
        assert_eq!(wasi.family(), TargetFamily::WasmWasi);
        assert_eq!(wasi.raw.as_deref(), Some("wasm32-wasip1"));

        let unknown = TargetSpec::from_triple_str("wasm32-unknown-unknown").unwrap();
        assert_eq!(unknown.family(), TargetFamily::WasmUnknown);
    }

    #[test]
    fn classifies_android_targets() {
        let target = TargetSpec::from_triple_str("aarch64-linux-android").unwrap();
        assert_eq!(target.family(), TargetFamily::Native);
        assert!(target._is_android);
        assert!(target._is_termux_like);
    }

    #[test]
    fn rejects_empty_and_malformed_targets() {
        assert!(matches!(
            TargetSpec::from_triple_str(" "),
            Err(TargetError::Empty)
        ));
        assert!(TargetSpec::from_triple_str("not a triple").is_err());
    }
}
