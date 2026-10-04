use std::fs;
use std::path::{Path, PathBuf};

use lpp_hir::{
    GraphBuilder, GraphError, GraphRequest, OsFileSystem, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_types::{ShadowInferenceOptions, TypeError, infer_hir_package};

#[test]
fn standalone_repository_type_shadow_is_bounded_and_deterministic() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("types crate must live under <root>/crates");
    let mut files = Vec::new();
    collect_lpp_files(&root.join("tests"), &mut files);
    collect_lpp_files(&root.join("test"), &mut files);
    files.sort();

    let mut analyzed = 0;
    let mut typed = 0;
    let mut trait_rules = 0;
    let mut generic_instances = 0;
    let mut trait_diagnostics = 0;
    let mut instance_diagnostics = 0;
    let mut limit_failures = Vec::new();
    for path in files {
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", path.display()));
        if source.lines().any(|line| {
            let line = line.trim_start();
            line.starts_with("import ") || line.starts_with("from ")
        }) {
            continue;
        }
        let request = GraphRequest::new(
            &path,
            PackageSpec::new(
                "standalone-type-shadow",
                path.parent().expect("collected source has a parent"),
            ),
        );
        let graph = match GraphBuilder::new(&OsFileSystem).build(request) {
            Ok(graph) => graph,
            Err(GraphError::Frontend { .. }) => continue,
            Err(error) => panic!("{}: graph: {error}", relative(root, &path)),
        };
        let hir = lower_package(&graph, ResolutionMode::Namespaced).unwrap_or_else(|diagnostics| {
            panic!(
                "{}: {} {}",
                relative(root, &path),
                diagnostics[0].code,
                diagnostics[0].message
            )
        });
        let first = infer_hir_package(&hir, ShadowInferenceOptions::default());
        let second = infer_hir_package(&hir, ShadowInferenceOptions::default());
        assert_eq!(
            first,
            second,
            "type shadow changed between identical runs for {}",
            relative(root, &path)
        );
        analyzed += 1;
        match first {
            Ok(output) => {
                typed += 1;
                trait_rules += output.trait_index.len();
                generic_instances += output.instances.records().len();
                trait_diagnostics += output.trait_diagnostics.len();
                instance_diagnostics += output.instance_diagnostics.len();
                assert!(
                    hir.expressions
                        .enumerate()
                        .all(|(id, _)| output.assignments.expression(id).is_some())
                );
                assert!(
                    hir.locals
                        .enumerate()
                        .all(|(id, _)| output.assignments.local(id).is_some())
                );
                assert!(
                    hir.type_refs
                        .enumerate()
                        .all(|(id, _)| output.assignments.type_ref(id).is_some())
                );
                assert!(
                    hir.items
                        .enumerate()
                        .all(|(id, _)| output.assignments.item(id).is_some())
                );
            }
            Err(error)
                if matches!(
                    error.error,
                    TypeError::WorkLimitExceeded { .. } | TypeError::DepthLimitExceeded { .. }
                ) =>
            {
                limit_failures.push(relative(root, &path));
            }
            Err(_) => {}
        }
    }

    assert!(
        limit_failures.is_empty(),
        "default type budgets rejected repository programs: {}",
        limit_failures.join(", ")
    );
    assert!(
        analyzed >= 180,
        "standalone type shadow unexpectedly shrank to {analyzed} files"
    );
    assert!(
        typed >= 148,
        "complete shadow typing unexpectedly shrank to {typed} repository programs"
    );
    assert!(
        trait_rules >= 11,
        "repository trait-index coverage unexpectedly shrank to {trait_rules} rules"
    );
    assert!(
        generic_instances >= 21,
        "repository generic-demand coverage unexpectedly shrank to {generic_instances} instances"
    );
    assert_eq!(trait_diagnostics, 0, "repository trait indexing regressed");
    assert_eq!(
        instance_diagnostics, 0,
        "repository generic-demand collection regressed"
    );
}

fn relative(root: &Path, path: &Path) -> String {
    path.strip_prefix(root)
        .unwrap_or(path)
        .to_string_lossy()
        .replace('\\', "/")
}

fn collect_lpp_files(directory: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries {
        let entry = entry.expect("directory entry must be readable");
        let path = entry.path();
        if path.is_dir() {
            collect_lpp_files(&path, output);
        } else if path.extension().is_some_and(|extension| extension == "lpp") {
            output.push(path);
        }
    }
}
