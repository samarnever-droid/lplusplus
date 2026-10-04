//! Regression coverage for the trait-typed-parameter desugaring.
//!
//! `def f(x: SomeTrait)` must be lowered into an implicit trait-bounded generic
//! `def f[$impl0: SomeTrait](x: $impl0)` so that the existing monomorphization
//! machinery produces one concrete copy per implementing type (dynamic-dispatch
//! by static specialization). See `crates/lpp-hir/src/lower/declaration.rs`.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use lpp_hir::{
    GraphBuilder, GraphRequest, HirItemKind, OsFileSystem, PackageSpec, ResolutionMode,
    lower_package,
};

static NEXT_TEMP_DIR: AtomicUsize = AtomicUsize::new(0);

const SOURCE: &str = r#"trait Speak:
    def speak(self) -> Int

struct Dog:
    name: Str

impl Speak for Dog:
    def speak(self) -> Int:
        return 1

def make_speak(animal: Speak) -> Int:
    return animal.speak()

def plain(value: Int) -> Int:
    return value
"#;

fn lower_source(source: &str) -> lpp_hir::HirPackage {
    let unique = NEXT_TEMP_DIR.fetch_add(1, Ordering::Relaxed);
    let dir =
        std::env::temp_dir().join(format!("lpp-trait-desugar-{}-{unique}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("create temp dir");
    let path: PathBuf = dir.join("main.lpp");
    fs::write(&path, source).expect("write source");

    let request = GraphRequest::new(&path, PackageSpec::new("trait-desugar", &dir));
    let graph = GraphBuilder::new(&OsFileSystem)
        .build(request)
        .expect("graph builds");
    lower_package(&graph, ResolutionMode::Namespaced).expect("package lowers")
}

#[test]
fn trait_typed_parameter_becomes_bounded_generic() {
    let package = lower_source(SOURCE);

    let mut make_speak = None;
    let mut plain = None;
    for item in package.items.iter() {
        let HirItemKind::Function(function) = item.kind else {
            continue;
        };
        let Some(name) = item.name else { continue };
        match package.names.symbols.resolve(name) {
            Some("make_speak") => make_speak = Some(function),
            Some("plain") => plain = Some(function),
            _ => {}
        }
    }

    let make_speak = make_speak.expect("make_speak function is lowered");
    let plain = plain.expect("plain function is lowered");

    // A trait-typed parameter gains exactly one synthetic, trait-bounded generic.
    let type_params = package.type_parameters(make_speak.type_parameters);
    assert_eq!(
        type_params.len(),
        1,
        "make_speak should gain one synthetic type parameter"
    );
    let synthetic = &package.type_parameters[type_params[0]];
    let bound = synthetic
        .bound
        .expect("synthetic type parameter must carry a trait bound");
    assert!(
        matches!(
            package.type_refs[bound].kind,
            lpp_hir::TypeRefKind::Named(_)
        ),
        "the bound should name the trait"
    );

    // A scalar-typed parameter must NOT introduce a synthetic generic.
    assert!(
        package.type_parameters(plain.type_parameters).is_empty(),
        "plain(value: Int) must stay non-generic"
    );
    assert_eq!(
        plain.type_parameters.start(),
        0,
        "plain(value: Int) should keep the canonical empty type-parameter range"
    );
}

#[test]
fn explicit_generics_are_not_duplicated_when_no_trait_parameter_is_desugared() {
    let package = lower_source(
        "def identity[T](value: T) -> T:\n    return value\n\ndef plain(value: Int) -> Int:\n    return value\n",
    );

    let mut identity = None;
    let mut plain = None;
    for item in package.items.iter() {
        let HirItemKind::Function(function) = item.kind else {
            continue;
        };
        let Some(name) = item.name else { continue };
        match package.names.symbols.resolve(name) {
            Some("identity") => identity = Some(function),
            Some("plain") => plain = Some(function),
            _ => {}
        }
    }

    let identity = identity.expect("identity function is lowered");
    let plain = plain.expect("plain function is lowered");

    assert_eq!(
        identity.type_parameters.start(),
        0,
        "identity[T] should reuse the explicit type-parameter list instead of appending a duplicate"
    );
    assert_eq!(package.type_parameters(identity.type_parameters).len(), 1);
    assert_eq!(
        plain.type_parameters.start(),
        0,
        "a non-generic function after identity[T] should still use the canonical empty range"
    );
    assert!(package.type_parameters(plain.type_parameters).is_empty());
}
