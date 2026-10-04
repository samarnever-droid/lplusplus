use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_types::{
    SemanticMetrics, ShadowInferenceOptions, infer_hir_package, semantic_metrics, semantic_snapshot,
};

#[derive(Debug, Default)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn with_source(source: String) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/scale/main.lpp"), source)]),
        }
    }
}

impl FileSystem for MemoryFileSystem {
    fn is_file(&self, path: &Path) -> Result<bool, FileSystemError> {
        Ok(self.files.contains_key(path))
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError> {
        if self.files.contains_key(path) || self.files.keys().any(|file| file.starts_with(path)) {
            Ok(path.to_owned())
        } else {
            Err(FileSystemError::new("canonicalize", path, "path not found"))
        }
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileSystemError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| FileSystemError::new("read", path, "file not found"))
    }
}

#[derive(Debug)]
struct Measurement {
    metrics: SemanticMetrics,
    snapshot_bytes: usize,
    elapsed: Duration,
}

#[test]
fn independent_shadow_compiles_are_parallel_and_deterministic() {
    let sources = (0..12)
        .map(|_| generated_generic_calls(500))
        .collect::<Vec<_>>();
    let _warmup = snapshot_source(&sources[0]);

    let sequential_started = Instant::now();
    let sequential = sources
        .iter()
        .map(|source| snapshot_source(source))
        .collect::<Vec<_>>();
    let sequential_elapsed = sequential_started.elapsed();

    let workers = std::thread::available_parallelism()
        .map_or(2, usize::from)
        .clamp(2, 4)
        .min(sources.len());
    let parallel_started = Instant::now();
    let mut parallel = Vec::with_capacity(sources.len());
    std::thread::scope(|scope| {
        let chunk_size = sources.len().div_ceil(workers);
        let handles = sources
            .chunks(chunk_size)
            .map(|chunk| {
                scope.spawn(move || {
                    chunk
                        .iter()
                        .map(|source| snapshot_source(source))
                        .collect::<Vec<_>>()
                })
            })
            .collect::<Vec<_>>();
        for handle in handles {
            parallel.extend(handle.join().expect("shadow worker must not panic"));
        }
    });
    let parallel_elapsed = parallel_started.elapsed();

    assert_eq!(parallel, sequential);
    assert!(
        parallel_elapsed <= sequential_elapsed.saturating_mul(4) + Duration::from_millis(500),
        "parallel scheduling regressed catastrophically: sequential={sequential_elapsed:?}, parallel={parallel_elapsed:?}, workers={workers}",
    );
    eprintln!(
        "phase3-concurrency workers={workers} packages={} sequential_ms={} parallel_ms={} ratio={:.3}",
        sources.len(),
        sequential_elapsed.as_millis(),
        parallel_elapsed.as_millis(),
        parallel_elapsed.as_secs_f64() / sequential_elapsed.as_secs_f64(),
    );
}

#[test]
fn phase3_time_memory_and_specialization_growth_are_bounded() {
    let small = measure(200);
    let large = measure(2_000);

    assert!(
        large.metrics.hir_nodes >= small.metrics.hir_nodes * 8,
        "the scaling input did not grow enough: small={small:?}, large={large:?}",
    );
    assert!(
        large.metrics.hir_nodes <= small.metrics.hir_nodes * 12,
        "HIR node growth is superlinear: small={small:?}, large={large:?}",
    );
    assert!(
        large.metrics.minimum_payload_bytes <= small.metrics.minimum_payload_bytes * 12,
        "minimum semantic payload grew superlinearly: small={small:?}, large={large:?}",
    );
    assert!(
        large.snapshot_bytes <= small.snapshot_bytes * 12,
        "deterministic snapshot size grew superlinearly: small={small:?}, large={large:?}",
    );
    assert_eq!(small.metrics.generic_instances, 1);
    assert_eq!(large.metrics.generic_instances, 1);
    assert!(
        large.elapsed <= small.elapsed.saturating_mul(20) + Duration::from_millis(500),
        "shadow compilation scaling exceeded its deliberately loose CI guard: small={small:?}, large={large:?}",
    );
    assert!(
        large.elapsed < Duration::from_secs(10),
        "2,000-call shadow compile exceeded the absolute exit guard: {large:?}",
    );
    eprintln!(
        "phase3-scaling calls=200/2000 hir_nodes={}/{} payload_bytes={}/{} snapshot_bytes={}/{} instances={}/{} elapsed_ms={}/{}",
        small.metrics.hir_nodes,
        large.metrics.hir_nodes,
        small.metrics.minimum_payload_bytes,
        large.metrics.minimum_payload_bytes,
        small.snapshot_bytes,
        large.snapshot_bytes,
        small.metrics.generic_instances,
        large.metrics.generic_instances,
        small.elapsed.as_millis(),
        large.elapsed.as_millis(),
    );
}

fn snapshot_source(source: &str) -> String {
    let filesystem = MemoryFileSystem::with_source(source.to_owned());
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/scale/main.lpp",
            PackageSpec::new("phase3-scale", "/scale"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let output = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    semantic_snapshot(&package, &output)
}

fn measure(call_count: usize) -> Measurement {
    let source = generated_generic_calls(call_count);
    let filesystem = MemoryFileSystem::with_source(source);
    let started = Instant::now();
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/scale/main.lpp",
            PackageSpec::new("phase3-scale", "/scale"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    let output = infer_hir_package(&package, ShadowInferenceOptions::default()).unwrap();
    let elapsed = started.elapsed();
    assert!(output.trait_diagnostics.is_empty());
    assert!(output.instance_diagnostics.is_empty());
    let metrics = semantic_metrics(&package, &output);
    let snapshot_bytes = semantic_snapshot(&package, &output).len();
    Measurement {
        metrics,
        snapshot_bytes,
        elapsed,
    }
}

fn generated_generic_calls(call_count: usize) -> String {
    let mut source =
        String::from("def identity[T](value: T) -> T:\n    return value\ndef main():\n");
    for index in 0..call_count {
        source.push_str(&format!("    value_{index:04} := identity({index})\n"));
    }
    source
}
