use flate2::Compression;
use flate2::write::GzEncoder;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use tar::{Builder, EntryType, Header};

static NEXT: AtomicU64 = AtomicU64::new(0);

fn temp(label: &str) -> PathBuf {
    std::env::temp_dir().join(format!(
        "keel-archive-{label}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

#[test]
fn package_archives_are_deterministic_and_round_trip() {
    let root = temp("deterministic");
    let package = root.join("package");
    std::fs::create_dir_all(package.join("src/nested")).unwrap();
    std::fs::write(
        package.join("Keel.toml"),
        "[package]\nname = \"demo\"\nversion = \"1.0.0\"\n",
    )
    .unwrap();
    std::fs::write(
        package.join("src/lib.lpp"),
        "def answer():\n    return 42\n",
    )
    .unwrap();
    std::fs::write(package.join("src/nested/data.txt"), b"data").unwrap();

    let first = keel::commands::archive::pack(&package).unwrap();
    let second = keel::commands::archive::pack(&package).unwrap();
    assert_eq!(first, second);

    let destination = root.join("unpacked");
    keel::commands::archive::unpack(&first, &destination).unwrap();
    assert_eq!(
        std::fs::read_to_string(destination.join("src/lib.lpp")).unwrap(),
        "def answer():\n    return 42\n"
    );
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extraction_rejects_parent_traversal() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut archive = Builder::new(&mut encoder);
        let payload = b"escaped";
        let mut header = Header::new_gnu();
        header.set_size(payload.len() as u64);
        header.set_mode(0o644);
        header.set_entry_type(EntryType::Regular);
        // Construct an intentionally hostile header without asking tar's safe
        // path setter to normalize or reject it first.
        let raw = header.as_mut_bytes();
        raw[..100].fill(0);
        raw[..13].copy_from_slice(b"../escape.txt");
        header.set_cksum();
        archive.append(&header, &payload[..]).unwrap();
        archive.finish().unwrap();
    }
    let bytes = encoder.finish().unwrap();

    let root = temp("traversal");
    let destination = root.join("destination");
    let error = keel::commands::archive::unpack(&bytes, &destination).unwrap_err();
    assert!(error.contains("unsafe archive path"), "{error}");
    assert!(!root.join("escape.txt").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn extraction_rejects_links_and_special_entries() {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut archive = Builder::new(&mut encoder);
        let mut header = Header::new_gnu();
        header.set_size(0);
        header.set_mode(0o777);
        header.set_entry_type(EntryType::Symlink);
        header.set_link_name("../../outside").unwrap();
        header.set_cksum();
        archive.append_data(&mut header, "link", &[][..]).unwrap();
        archive.finish().unwrap();
    }
    let bytes = encoder.finish().unwrap();

    let root = temp("link");
    let error = keel::commands::archive::unpack(&bytes, &root.join("out")).unwrap_err();
    assert!(error.contains("unsupported archive entry type"), "{error}");
    let _ = std::fs::remove_dir_all(root);
}
