//! Exit gate for the package-manager storage core: content addressing, the
//! durable blob store, and the hot KV cache.

use lpp_pm::{
    BlobStore, ContentAddress, DiskBlobStore, InMemoryKv, KvBackendKind, KvCache, PmError,
    create_kv,
};

fn temp_dir(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!("lpp_pm_{name}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&p);
    std::fs::create_dir_all(&p).unwrap();
    p
}

#[test]
fn content_address_is_deterministic_sha256() {
    // The well-known SHA-256 of "abc".
    let a = ContentAddress::of_bytes(b"abc");
    assert_eq!(
        a.as_str(),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let b = ContentAddress::of_bytes(b"abc");
    assert_eq!(a, b, "same bytes -> same address");
    assert_ne!(
        a,
        ContentAddress::of_bytes(b"abd"),
        "different bytes -> different address"
    );
    assert!(ContentAddress::try_new(a.as_str()).is_ok());
    assert!(ContentAddress::try_new("nope").is_err());
    assert!(
        ContentAddress::try_new(&"A".repeat(64)).is_err(),
        "uppercase rejected"
    );
}

#[test]
fn disk_blob_store_round_trips_and_content_addresses() {
    let dir = temp_dir("blob");
    let store = DiskBlobStore::open(&dir).unwrap();

    let a = store.insert(b"hello lpp package").unwrap();
    let again = store.insert(b"hello lpp package").unwrap();
    assert_eq!(a, again, "insert is idempotent");
    assert!(store.contains(&a));
    assert_eq!(store.fetch(&a).unwrap(), b"hello lpp package");

    let missing = ContentAddress::of_bytes(b"never stored");
    assert!(!store.contains(&missing));
    assert!(matches!(
        store.fetch(&missing),
        Err(PmError::BlobNotFound(_))
    ));

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn in_memory_kv_is_deterministic_and_accounts_hits() {
    let mut kv: Box<dyn KvCache> = Box::new(InMemoryKv::new());
    kv.set("name:serde", b"1.0.200");
    kv.set("name:sha2", b"0.10.8");

    assert_eq!(kv.len(), 2);
    assert_eq!(kv.get("name:serde").unwrap(), b"1.0.200");
    assert!(kv.get("name:missing").is_none());
    // Deterministic (sorted) key ordering.
    assert_eq!(
        kv.keys(),
        vec!["name:serde".to_string(), "name:sha2".to_string()]
    );
    assert!(kv.del("name:sha2"));
    assert!(!kv.del("name:sha2"), "second delete reports absent");
    assert_eq!(kv.len(), 1);

    let stats = kv.stats();
    assert_eq!(stats.hits, 1);
    assert_eq!(stats.misses, 1);
    assert_eq!(stats.entries, 1);
    assert_eq!(stats.hit_ratio(), 0.5);
    assert_eq!(kv.name(), "memory");
}

#[test]
fn backend_factory_selects_memory() {
    assert_eq!(KvBackendKind::parse("memory"), Some(KvBackendKind::Memory));
    assert_eq!(KvBackendKind::parse("auto"), Some(KvBackendKind::Auto));
    assert_eq!(KvBackendKind::parse("bogus"), None);
    assert_eq!(KvBackendKind::default(), KvBackendKind::Auto);

    let mem = create_kv(KvBackendKind::Memory).unwrap();
    assert_eq!(mem.name(), "memory");
    let auto = create_kv(KvBackendKind::Auto).unwrap();
    assert_eq!(auto.name(), "memory");
}
