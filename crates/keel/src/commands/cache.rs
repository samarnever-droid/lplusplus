//! `keel cache` — inspect or clean the global cache.

use tabled::builder::Builder;

use crate::cli::{CacheAction, CacheBackend};
use lpp_pm::{KvBackendKind, create_kv};

/// Total size in bytes of everything under `dir` (recursive).
fn dir_size_bytes(dir: &std::path::Path) -> (u64, u64) {
    fn walk(dir: &std::path::Path) -> (u64, u64) {
        let mut bytes = 0u64;
        let mut files = 0u64;
        let Ok(rd) = std::fs::read_dir(dir) else {
            return (0, 0);
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                let (b, f) = walk(&p);
                bytes += b;
                files += f;
            } else if let Ok(md) = e.metadata() {
                bytes += md.len();
                files += 1;
            }
        }
        (bytes, files)
    }
    walk(dir)
}

fn human(n: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{n} {}", UNITS[0])
    } else {
        format!("{v:.1} {}", UNITS[i])
    }
}

pub fn run(action: &CacheAction, backend: CacheBackend) -> Result<(), String> {
    match action {
        CacheAction::Path => {
            println!("{}", crate::cache_dir().display());
            Ok(())
        }
        CacheAction::Stats => {
            let kv = create_kv(to_kind(backend)).map_err(|e| e.to_string())?;
            let s = kv.stats();
            let mut b = Builder::default();
            b.push_record(["field".to_string(), "value".to_string()]);
            let cache_dir = crate::cache_dir();
            let (disk_bytes, disk_files) = dir_size_bytes(&cache_dir);
            let registry_count = std::fs::read_dir(cache_dir.join("registries"))
                .map(|entries| {
                    entries
                        .flatten()
                        .filter(|entry| entry.path().is_dir())
                        .count()
                })
                .unwrap_or(0);
            b.push_record(["hot_backend".to_string(), kv.name().to_string()]);
            b.push_record(["hot_scope".to_string(), "current process".to_string()]);
            b.push_record(["hot_entries".to_string(), s.entries.to_string()]);
            b.push_record(["hot_hits".to_string(), s.hits.to_string()]);
            b.push_record(["hot_misses".to_string(), s.misses.to_string()]);
            b.push_record(["disk_files".to_string(), disk_files.to_string()]);
            b.push_record([
                "disk_bytes".to_string(),
                format!("{disk_bytes} ({})", human(disk_bytes)),
            ]);
            b.push_record(["registry_clones".to_string(), registry_count.to_string()]);
            b.push_record(["cache_dir".to_string(), cache_dir.display().to_string()]);
            let table = b.build();
            println!("{table}");
            Ok(())
        }
        CacheAction::Clean => clean(),
    }
}

fn to_kind(b: CacheBackend) -> KvBackendKind {
    match b {
        CacheBackend::Auto => KvBackendKind::Auto,
        CacheBackend::Memory => KvBackendKind::Memory,
    }
}

/// `keel cache clean` — remove the global cache dir (registry clone, blobs,
/// hot cache). Per-project fingerprints live in the workspace `target/` and
/// are untouched (they are the cargo-clean analog, not the global cache).
fn clean() -> Result<(), String> {
    let dir = crate::cache_dir();
    if !dir.exists() {
        println!("cache is already empty ({} does not exist)", dir.display());
        return Ok(());
    }
    let (bytes, files) = dir_size_bytes(&dir);
    std::fs::remove_dir_all(&dir).map_err(|e| format!("failed to clean cache: {e}"))?;
    let mut b = Builder::default();
    b.push_record(["field".to_string(), "value".to_string()]);
    b.push_record(["cache_dir".to_string(), dir.display().to_string()]);
    b.push_record(["removed_files".to_string(), files.to_string()]);
    b.push_record([
        "removed_bytes".to_string(),
        format!("{bytes} ({})", human(bytes)),
    ]);
    println!("{}", b.build());
    println!("cache cleaned");
    Ok(())
}
