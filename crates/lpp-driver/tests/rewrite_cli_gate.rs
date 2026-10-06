use std::path::{Path, PathBuf};

use lpp_driver::{CompilerSession, DriverRequest, RewriteEngine};

fn workdir(name: &str) -> PathBuf {
    let directory =
        std::env::temp_dir().join(format!("lpp_rewrite_cli_{}_{}", std::process::id(), name));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    directory
}

fn source(directory: &Path) -> PathBuf {
    let entry = directory.join("main.lpp");
    std::fs::write(&entry, "def main() -> Int:\n    return 0\n").unwrap();
    entry
}

fn execute(directory: &Path, arguments: &[String]) -> i32 {
    let request = DriverRequest::from_args(
        std::iter::once("lpp".to_string()).chain(arguments.iter().cloned()),
        directory,
    )
    .unwrap();
    let mut session = CompilerSession::new(RewriteEngine);
    session.execute(&request).exit_code()
}

#[test]
fn keel_project_commands_use_the_request_working_directory() {
    let root = workdir("keel-routing");
    let directory = root.join("project");
    std::fs::create_dir_all(&directory).unwrap();
    assert_eq!(execute(&directory, &["init".into()]), 0);
    assert!(directory.join("Keel.toml").is_file());

    assert_eq!(
        execute(&directory, &["add".into(), "example@^1.2".into()]),
        0
    );
    let manifest = std::fs::read_to_string(directory.join("Keel.toml")).unwrap();
    assert!(manifest.contains("example = \"^1.2\""), "{manifest}");
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn common_project_commands_no_longer_defer_to_legacy() {
    let scratch = workdir("help-clean-routing");
    assert_eq!(execute(&scratch, &["help".into()]), 0);
    assert_eq!(execute(&scratch, &["doctor".into()]), 0);
    assert_eq!(execute(&scratch, &["doctor".into(), "extra".into()]), 2);
    assert_eq!(
        execute(&scratch, &["create".into(), "child-project".into()]),
        0
    );
    assert!(scratch.join("child-project/Keel.toml").is_file());
    let root = scratch.join("project");
    std::fs::create_dir_all(&root).unwrap();
    assert_eq!(execute(&root, &["init".into()]), 0);
    assert_eq!(execute(&root, &["install".into()]), 0);
    assert_eq!(execute(&root, &["metadata".into()]), 0);
    assert_eq!(execute(&root, &["list".into()]), 0);
    assert_eq!(execute(&root, &["workspace".into()]), 0);
    assert_eq!(execute(&root, &["workspace".into(), "graph".into()]), 0);
    assert_eq!(execute(&root, &["version".into()]), 0);

    let artifact = root.join("target/debug/program");
    std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
    std::fs::write(&artifact, b"artifact").unwrap();
    assert!(artifact.is_file());

    assert_eq!(execute(&root, &["clean".into()]), 0);
    assert!(!root.join("target").exists());
    let _ = std::fs::remove_dir_all(scratch);
}

#[test]
fn rejects_unknown_flags_and_missing_option_values() {
    let directory = workdir("malformed-options");
    let entry = source(&directory).to_string_lossy().into_owned();
    assert_eq!(execute(&directory, &[entry.clone(), "--mystery".into()]), 2);
    assert_eq!(execute(&directory, &[entry.clone(), "--backend".into()]), 2);
    assert_eq!(execute(&directory, &[entry.clone(), "--target".into()]), 2);
    assert_eq!(execute(&directory, &[entry, "-o".into()]), 2);
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn validates_wasm_target_spelling_strictly() {
    let directory = workdir("wasm-target");
    let entry = source(&directory).to_string_lossy().into_owned();
    assert_eq!(
        execute(
            &directory,
            &[
                entry,
                "--target".into(),
                "wasm64-whatever".into(),
                "--check".into()
            ]
        ),
        2
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn wasm_check_validates_without_writing_a_module() {
    let directory = workdir("wasm-check");
    let entry = source(&directory).to_string_lossy().into_owned();
    assert_eq!(
        execute(
            &directory,
            &[
                entry,
                "--target".into(),
                "wasm32-wasip1".into(),
                "--check".into()
            ]
        ),
        0
    );
    assert!(!directory.join("main.wasm").exists());
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn formatted_strings_reach_the_wasm_backend() {
    let directory = workdir("wasm-formatted-string");
    let entry = directory.join("main.lpp");
    std::fs::write(
        &entry,
        "def main() -> Int:\n    name := \"world\"\n    message := f\"hello {name}!\"\n    print_str(message)\n    return 0\n",
    )
    .unwrap();
    assert_eq!(
        execute(
            &directory,
            &[
                entry.to_string_lossy().into_owned(),
                "--target".into(),
                "wasm32-wasip1".into(),
                "--check".into(),
            ]
        ),
        0
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn tuples_and_projections_reach_the_wasm_backend() {
    let directory = workdir("wasm-tuples");
    let entry = directory.join("main.lpp");
    std::fs::write(
        &entry,
        "def main() -> Int:\n    pair := (20, 22)\n    text_pair := (\"left\", \"right\")\n    print_str(text_pair.1)\n    return pair.0 + pair[1]\n",
    )
    .unwrap();
    assert_eq!(
        execute(
            &directory,
            &[
                entry.to_string_lossy().into_owned(),
                "--target".into(),
                "wasm32-wasip1".into(),
                "--check".into(),
            ]
        ),
        0
    );
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn aarch64_is_selectable_for_object_emission() {
    let directory = workdir("aarch64-object");
    let entry = source(&directory).to_string_lossy().into_owned();
    let output = directory.join("main-aarch64.o");
    assert_eq!(
        execute(
            &directory,
            &[
                entry,
                "--target".into(),
                "aarch64".into(),
                "--emit-object".into(),
                "-o".into(),
                output.to_string_lossy().into_owned(),
            ]
        ),
        0
    );
    assert!(output.is_file());
    let _ = std::fs::remove_dir_all(directory);
}

#[test]
fn slices_and_maps_reach_the_wasm_backend() {
    let directory = workdir("wasm-slices-maps");
    let entry = directory.join("main.lpp");
    std::fs::write(
        &entry,
        concat!(
            "def map_value() -> Int:\n",
            "    mut values := map_new()\n",
            "    map_put(values, \"answer\", 42)\n",
            "    return map_get(values, \"answer\")\n",
            "\n",
            "def main() -> Int:\n",
            "    numbers := [10, 20, 30]\n",
            "    view := slice(numbers, 1, 2)\n",
            "    print_int(slice_get(view, 0))\n",
            "    return map_value()\n",
        ),
    )
    .unwrap();
    assert_eq!(
        execute(
            &directory,
            &[
                entry.to_string_lossy().into_owned(),
                "--target".into(),
                "wasm32-wasip1".into(),
                "--check".into(),
            ]
        ),
        0
    );
    let _ = std::fs::remove_dir_all(directory);
}
