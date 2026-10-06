use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use lpp_driver::{CompilerSession, DriverRequest, RewriteEngine};

fn repository_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .unwrap()
        .to_owned()
}

fn request(directory: &Path, arguments: Vec<String>) -> DriverRequest {
    DriverRequest::from_args(
        std::iter::once("lpp".to_string()).chain(arguments),
        directory,
    )
    .unwrap()
}

#[test]
fn rewrite_wasm_executes_the_complete_positive_legacy_corpus() {
    let root = repository_root();
    let cases = root.join("tests/wasm/cases");
    let host = root.join("crates/lpp-codegen-wasm/tests/wasm_run.mjs");
    let output_root =
        std::env::temp_dir().join(format!("lpp-rewrite-wasm-corpus-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&output_root);
    std::fs::create_dir_all(&output_root).unwrap();

    let mut sources: Vec<PathBuf> = std::fs::read_dir(&cases)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("lpp"))
        .collect();
    sources.sort();
    assert_eq!(sources.len(), 24, "the positive corpus census changed");

    for source in sources {
        let name = source.file_stem().unwrap().to_string_lossy();
        let wasm = output_root.join(format!("{name}.wasm"));
        let compile = request(
            &root,
            vec![
                source.to_string_lossy().into_owned(),
                "--target".into(),
                "wasm32-wasip1".into(),
                "--emit-object".into(),
                "-o".into(),
                wasm.to_string_lossy().into_owned(),
            ],
        );
        let mut session = CompilerSession::new(RewriteEngine);
        assert_eq!(
            session.execute(&compile).exit_code(),
            0,
            "rewrite rejected positive WASM case {name}"
        );
        assert_eq!(&std::fs::read(&wasm).unwrap()[..8], b"\0asm\x01\0\0\0");

        let stdin_path = cases.join(format!("{name}.stdin"));
        let mut command = Command::new("node");
        command.arg(&host).arg(&wasm);
        if stdin_path.is_file() {
            command.stdin(Stdio::from(File::open(stdin_path).unwrap()));
        } else {
            command.stdin(Stdio::null());
        }
        let output = command.output().unwrap();
        assert!(
            output.status.success(),
            "{name} trapped:\n{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let expected = std::fs::read(cases.join(format!("{name}.expected"))).unwrap();
        assert_eq!(output.stdout, expected, "{name} output diverged");
    }

    let _ = std::fs::remove_dir_all(output_root);
}

#[test]
fn rewrite_wasm_preserves_host_capability_rejections() {
    let root = repository_root();
    let reject = root.join("tests/wasm/reject");
    // Restricted eager `spawn` is intentionally supported by rewrite WASM;
    // the remaining legacy negative cases still require rejection.
    for name in [
        "reject_command.lpp",
        "reject_env_set.lpp",
        "reject_ffi.lpp",
        "reject_file.lpp",
        "reject_net.lpp",
        "reject_simd.lpp",
    ] {
        let source = reject.join(name);
        let compile = request(
            &root,
            vec![
                source.to_string_lossy().into_owned(),
                "--target".into(),
                "wasm32-wasip1".into(),
                "--check".into(),
            ],
        );
        let mut session = CompilerSession::new(RewriteEngine);
        assert_ne!(
            session.execute(&compile).exit_code(),
            0,
            "unsupported WASM capability unexpectedly compiled: {name}"
        );
    }
}

#[test]
fn rewrite_wasm_input_preserves_following_lines() {
    let root = repository_root();
    let directory = std::env::temp_dir().join(format!(
        "lpp-rewrite-wasm-input-lines-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join("main.lpp");
    let wasm = directory.join("main.wasm");
    let stdin = directory.join("stdin.txt");
    std::fs::write(
        &source,
        concat!(
            "def main() -> Int:\n",
            "    first := input()\n",
            "    second := input()\n",
            "    print_str(first)\n",
            "    print_str(second)\n",
            "    return 0\n",
        ),
    )
    .unwrap();
    std::fs::write(&stdin, "first\nsecond\n").unwrap();

    let compile = request(
        &directory,
        vec![
            source.to_string_lossy().into_owned(),
            "--target".into(),
            "wasm32-wasip1".into(),
            "--emit-object".into(),
            "-o".into(),
            wasm.to_string_lossy().into_owned(),
        ],
    );
    let mut session = CompilerSession::new(RewriteEngine);
    assert_eq!(session.execute(&compile).exit_code(), 0);

    let host = root.join("crates/lpp-codegen-wasm/tests/wasm_run.mjs");
    let output = Command::new("node")
        .arg(host)
        .arg(wasm)
        .stdin(Stdio::from(File::open(stdin).unwrap()))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"first\nsecond\n");
    let _ = std::fs::remove_dir_all(directory);
}
