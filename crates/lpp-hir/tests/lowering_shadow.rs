use std::fs;
use std::path::{Path, PathBuf};

use lpp_hir::{
    GraphBuilder, GraphError, GraphRequest, OsFileSystem, PackageSpec, ResolutionMode,
    lower_package,
};

#[test]
fn standalone_repository_programs_lower_without_recursive_hir_nodes() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("HIR crate must live under <root>/crates");
    let mut files = Vec::new();
    collect_lpp_files(&root.join("tests"), &mut files);
    collect_lpp_files(&root.join("test"), &mut files);
    files.sort();

    let mut lowered = 0;
    let mut failures = Vec::new();
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
                "standalone-shadow",
                path.parent().expect("collected source has a parent"),
            ),
        );
        let graph = match GraphBuilder::new(&OsFileSystem).build(request) {
            Ok(graph) => graph,
            Err(GraphError::Frontend { .. }) => continue,
            Err(error) => {
                failures.push(format!("{}: graph: {error}", relative(root, &path)));
                continue;
            }
        };
        match lower_package(&graph, ResolutionMode::Namespaced) {
            Ok(_) => lowered += 1,
            Err(diagnostics) => failures.push(format!(
                "{}: {} {}",
                relative(root, &path),
                diagnostics[0].code,
                diagnostics[0].message
            )),
        }
    }

    assert!(
        failures.is_empty(),
        "HIR lowering shadow failures:\n{}",
        failures.join("\n")
    );
    assert!(
        lowered >= 180,
        "standalone lowering corpus unexpectedly shrank to {lowered} files"
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
