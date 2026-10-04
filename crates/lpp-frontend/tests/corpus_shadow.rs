use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use lpp_common::FileId;
use lpp_frontend::parse_source;

const EXPECTED_FRONTEND_REJECTIONS: &[&str] = &[
    "packages/lpp-semver/src/semver.lpp",
    "packages/lpp-toml/src/toml.lpp",
    "pm/src/publisher.lpp",
    "tests/tuple_bad_arity.lpp",
    "tests/variadic_bad_position.lpp",
];

#[test]
fn repository_corpus_matches_the_legacy_frontend_acceptance_boundary() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("frontend crate must live under <root>/crates");
    let mut files = Vec::new();
    collect_lpp_files(root, root, &mut files);
    files.sort();

    let expected: BTreeSet<_> = EXPECTED_FRONTEND_REJECTIONS.iter().copied().collect();
    let mut rejected = BTreeSet::new();
    let mut unexpected = Vec::new();
    for (raw_id, relative) in files.iter().enumerate() {
        let source = fs::read_to_string(root.join(relative))
            .unwrap_or_else(|error| panic!("failed to read {}: {error}", relative.display()));
        let result = parse_source(
            FileId::from_raw(u32::try_from(raw_id).expect("test corpus fits in u32")),
            &source,
        );
        let relative = relative.to_string_lossy().replace('\\', "/");
        match result {
            Ok(_) if expected.contains(relative.as_str()) => {
                unexpected.push(format!("unexpectedly accepted {relative}"));
            }
            Err(diagnostics) if !expected.contains(relative.as_str()) => {
                let diagnostic = &diagnostics[0];
                let offset = diagnostic
                    .primary_span
                    .map_or(0, |span| span.start as usize);
                let line = source[..offset.min(source.len())]
                    .bytes()
                    .filter(|byte| *byte == b'\n')
                    .count()
                    + 1;
                let excerpt = source.lines().nth(line - 1).unwrap_or("").trim();
                unexpected.push(format!(
                    "unexpectedly rejected {relative}:{line}: {} {} [{excerpt}]",
                    diagnostic.code, diagnostic.message
                ));
            }
            Err(_) => {
                rejected.insert(relative);
            }
            Ok(_) => {}
        }
    }

    assert!(
        unexpected.is_empty(),
        "frontend shadow mismatches:\n{}",
        unexpected.join("\n")
    );
    assert_eq!(rejected, expected.iter().map(ToString::to_string).collect());
}

fn collect_lpp_files(root: &Path, directory: &Path, output: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("failed to read {}: {error}", directory.display()))
    {
        let entry = entry.expect("directory entry must be readable");
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            let name = name.to_string_lossy();
            if name.starts_with('.') || matches!(name.as_ref(), "target" | "node_modules") {
                continue;
            }
            collect_lpp_files(root, &path, output);
        } else if path.extension().is_some_and(|extension| extension == "lpp") {
            output.push(
                path.strip_prefix(root)
                    .expect("collected paths are under the repository root")
                    .to_owned(),
            );
        }
    }
}
