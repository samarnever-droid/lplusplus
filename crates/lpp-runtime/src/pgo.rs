//! Lightweight runtime counters used by Cranelift PGO instrumentation.
//!
//! The compiler supplies stable MIR function IDs and a fingerprint for the
//! unoptimized program. A lazily allocated dense array of relaxed atomics keeps
//! the hot path to an indexed atomic increment; generated `main` writes a
//! sorted snapshot only when it returns normally.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};

/// The source builder currently caps a program at 100,000 MIR functions. The
/// extra headroom leaves room for generated functions and future growth.
const PGO_COUNTER_CAPACITY: usize = 1 << 20;
static COUNTERS: OnceLock<Box<[AtomicU64]>> = OnceLock::new();

fn counters() -> &'static [AtomicU64] {
    COUNTERS
        .get_or_init(|| {
            (0..PGO_COUNTER_CAPACITY)
                .map(|_| AtomicU64::new(0))
                .collect()
        })
        .as_ref()
}

fn increment_counter(counters: &[AtomicU64], function_id: i64) -> bool {
    let Ok(index) = usize::try_from(function_id) else {
        return false;
    };
    let Some(counter) = counters.get(index) else {
        return false;
    };
    // A wrapping overflow would require more than 18 quintillion entries for
    // one function during a single process run, so a relaxed increment is safe
    // and avoids a contended map or mutex on hot call paths.
    counter.fetch_add(1, Ordering::Relaxed);
    true
}

/// Increment the function-entry counter for a compiler-generated MIR function
/// ID. IDs outside the dense runtime table are ignored defensively.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_pgo_hit(function_id: i64) {
    let _ = increment_counter(counters(), function_id);
}

fn collect_counts(counters: &[AtomicU64]) -> BTreeMap<u32, u64> {
    counters
        .iter()
        .enumerate()
        .filter_map(|(function_id, counter)| {
            let count = counter.load(Ordering::Relaxed);
            (count > 0).then_some((function_id as u32, count))
        })
        .collect()
}

/// Write the collected counters to `LPP_PGO_PROFILE` (default: `lpp.pgo`).
/// The fingerprint is passed as signed bits by the machine ABI and restored as
/// an unsigned value in the portable text format.
#[unsafe(no_mangle)]
pub extern "C" fn lpp_pgo_flush(fingerprint: i64) {
    let fingerprint = u64::from_ne_bytes(fingerprint.to_ne_bytes());
    let path = std::env::var_os("LPP_PGO_PROFILE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| "lpp.pgo".into());
    let counts = collect_counts(counters());
    if let Err(error) = write_profile(&path, fingerprint, &counts) {
        eprintln!(
            "lpp: failed to write PGO profile {}: {error}",
            path.display()
        );
    }
}

fn profile_text(fingerprint: u64, counts: &BTreeMap<u32, u64>) -> String {
    let mut output = format!("LPP-PGO-1 {fingerprint:016x}\n");
    for (function_id, count) in counts {
        if *count > 0 {
            let _ = writeln!(output, "{function_id} {count}");
        }
    }
    output
}

fn write_profile(
    path: &Path,
    fingerprint: u64,
    counts: &BTreeMap<u32, u64>,
) -> std::io::Result<()> {
    let temporary = path.with_extension(format!("pgo-tmp-{}", std::process::id()));
    std::fs::write(&temporary, profile_text(fingerprint, counts))?;
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(temporary, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profile_writer_is_sorted_and_preserves_the_fingerprint() {
        let path = std::env::temp_dir().join(format!("lpp_pgo_{}.profile", std::process::id()));
        let counts = BTreeMap::from([(7, 11), (2, 5), (9, 0)]);
        write_profile(&path, u64::MAX - 4, &counts).unwrap();
        let output = std::fs::read_to_string(&path).unwrap();
        assert_eq!(output, "LPP-PGO-1 fffffffffffffffb\n2 5\n7 11\n");
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn entry_hits_increment_atomically_and_ignore_invalid_ids() {
        let counters = [AtomicU64::new(0), AtomicU64::new(0)];
        assert!(increment_counter(&counters, 1));
        assert!(increment_counter(&counters, 1));
        assert!(!increment_counter(&counters, -1));
        assert!(!increment_counter(&counters, 2));
        assert_eq!(counters[0].load(Ordering::Relaxed), 0);
        assert_eq!(counters[1].load(Ordering::Relaxed), 2);
    }

    #[test]
    fn counter_snapshot_is_sorted_and_omits_zero_entries() {
        let counters = [AtomicU64::new(0), AtomicU64::new(4), AtomicU64::new(0)];
        assert_eq!(collect_counts(&counters), BTreeMap::from([(1, 4)]));
    }
}
