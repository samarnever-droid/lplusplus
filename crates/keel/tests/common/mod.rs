#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

#[derive(Clone, Copy)]
pub enum FakeMode {
    ArgsLines,
    PwdArgs,
    Incremental,
    PwdPipeArgs,
    TestRunner,
    SlowMarks,
}

impl FakeMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::ArgsLines => "args_lines",
            Self::PwdArgs => "pwd_args",
            Self::Incremental => "incremental",
            Self::PwdPipeArgs => "pwd_pipe_args",
            Self::TestRunner => "test_runner",
            Self::SlowMarks => "slow_marks",
        }
    }
}

const FAKE_LPP_SRC: &str = r#"
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

fn append_all(path: &Path, content: &str) {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .expect("open fake lpp log file");
    file.write_all(content.as_bytes())
        .expect("write fake lpp log file");
}

fn main() {
    let exe_from_arg0 = std::env::args_os()
        .next()
        .map(PathBuf::from)
        .filter(|p| p.parent().map(|d| d.join("lpp-fake.cfg").exists()).unwrap_or(false));
    let exe = exe_from_arg0.unwrap_or_else(|| std::env::current_exe().expect("current_exe"));
    let cfg_path = exe.parent().expect("exe parent").join("lpp-fake.cfg");
    let cfg = fs::read_to_string(&cfg_path).expect("read lpp-fake.cfg");
    let mut lines = cfg.lines();
    let mode = lines.next().expect("missing mode in lpp-fake.cfg");
    let log_path = PathBuf::from(lines.next().expect("missing log path in lpp-fake.cfg"));
    let args: Vec<String> = std::env::args().skip(1).collect();

    match mode {
        "args_lines" => {
            if !args.is_empty() {
                let mut buf = args.join("\n");
                buf.push('\n');
                append_all(&log_path, &buf);
            }
        }
        "pwd_args" => {
            let cwd = std::env::current_dir().expect("current_dir");
            let buf = format!("PWD {}\nARGS {}\n", cwd.display(), args.join(" "));
            append_all(&log_path, &buf);
        }
        "incremental" => {
            append_all(&log_path, &format!("INVOKE {}\n", args.join(" ")));
            let mut out: Option<&str> = None;
            let mut prev = "";
            for arg in &args {
                if prev == "-o" {
                    out = Some(arg.as_str());
                }
                prev = arg.as_str();
            }
            if let Some(out_path) = out {
                let p = Path::new(out_path);
                if let Some(parent) = p.parent() {
                    if !parent.as_os_str().is_empty() {
                        fs::create_dir_all(parent).expect("create output parent dir");
                    }
                }
                fs::write(p, b"built\n").expect("write fake build artifact");
            }
        }
        "pwd_pipe_args" => {
            let cwd = std::env::current_dir().expect("current_dir");
            let buf = format!("{}|{}\n", cwd.display(), args.join(" "));
            append_all(&log_path, &buf);
        }
        "test_runner" => {
            if !args.is_empty() {
                let mut buf = args.join("\n");
                buf.push('\n');
                append_all(&log_path, &buf);
                let first_name = Path::new(&args[0])
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("");
                if first_name.contains("fail") {
                    eprintln!("assertion failed: 1 != 2");
                    std::process::exit(3);
                }
            }
        }
        "slow_marks" => {
            let first = args.first().map(String::as_str).unwrap_or("");
            append_all(&log_path, &format!("START {first}\n"));
            std::thread::sleep(Duration::from_millis(500));
            append_all(&log_path, &format!("END {first}\n"));
        }
        other => panic!("unknown lpp-fake mode: {other}"),
    }
}
"#;

#[cfg(unix)]
fn make_executable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
}

#[cfg(not(unix))]
fn make_executable(_p: &Path) {}

fn fake_lpp_master() -> &'static PathBuf {
    static MASTER: OnceLock<PathBuf> = OnceLock::new();
    MASTER.get_or_init(|| {
        let build_dir = std::env::temp_dir().join(format!("lpp-fake-bin-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&build_dir);
        std::fs::create_dir_all(&build_dir).unwrap();
        let src = build_dir.join("lpp_fake.rs");
        let exe = build_dir.join(format!("lpp-fake{}", std::env::consts::EXE_SUFFIX));
        std::fs::write(&src, FAKE_LPP_SRC).unwrap();

        let rustc = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let out = Command::new(rustc)
            .arg("--edition")
            .arg("2021")
            .arg("-o")
            .arg(&exe)
            .arg(&src)
            .output()
            .expect("rustc must be available to build fake lpp helper");
        assert!(
            out.status.success(),
            "failed to compile lpp-fake helper: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_file(&src);
        make_executable(&exe);
        exe
    })
}

/// Install a cross-platform `lpp-fake` binary into `root` configured to run in
/// `mode` and log into `log_file`.
///
/// Uses a hard link from the `rustc`-compiled master binary so this process never
/// opens the installed executable for writing (avoiding multi-threaded fork `ETXTBSY`).
pub fn install_fake_lpp(root: &Path, mode: FakeMode, log_file: &Path) -> PathBuf {
    let exe = root.join(format!("lpp-fake{}", std::env::consts::EXE_SUFFIX));
    let cfg = root.join("lpp-fake.cfg");
    std::fs::write(&cfg, format!("{}\n{}\n", mode.as_str(), log_file.display())).unwrap();
    let master = fake_lpp_master();
    let _ = std::fs::remove_file(&exe);
    if std::fs::hard_link(master, &exe).is_err() {
        let bytes = std::fs::read(master).expect("read compiled lpp-fake binary");
        std::fs::write(&exe, bytes).unwrap();
        make_executable(&exe);
    }
    exe
}

pub fn normalize_slashes(s: &str) -> String {
    s.replace('\\', "/")
}
