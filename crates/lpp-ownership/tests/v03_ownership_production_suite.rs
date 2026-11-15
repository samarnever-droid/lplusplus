//! Production ownership & cycle-capability test suite for L++ v0.3.0.
//!
//! Exercises `compute_ownership_plan`, `verify_ownership_plan`,
//! `verify_ownership_balance`, `OwnershipPlanPass`, `OwnershipBalancePass`,
//! `ValuePlacement::{Frame, Owned, Shared}`, `TypeStrategy::{Owned, Shared}`,
//! `ArenaPlan`, `concurrent_cells`/`concurrent_types`, optimization pass
//! preservation of ownership balance, and raw pointer (`Ptr[T]`/`RawPtr`)
//! exemption from managed ARC placement.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_common::OptimizationLevel;
use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, HirPackage, PackageSpec,
    ResolutionMode, lower_package,
};
use lpp_mir::{
    ExecutionValue, InterpreterLimits, MirBuildOptions, MirFunctionId, MirInvariant, MirProgram,
    build_mir, execute_mir_arc, verify_mir,
};
use lpp_ownership::{
    OwnershipBalancePass, OwnershipPlan, OwnershipPlanPass, TypeStrategy, ValuePlacement,
    compute_ownership_plan, ownership_plan_snapshot, verify_ownership_balance,
    verify_ownership_plan,
};
use lpp_passes::{PassManager, run_optimization};
use lpp_types::{ShadowInferenceOptions, TypeInterner, infer_hir_package};

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/own_v03/main.lpp"), source.to_owned())]),
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
            .ok_or_else(|| FileSystemError::new("read", path, "path not found"))
    }
}

fn compile_and_plan(source: &str) -> (MirProgram, TypeInterner, OwnershipPlan, HirPackage) {
    let fs = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&fs)
        .build(GraphRequest::new(
            "/own_v03/main.lpp",
            PackageSpec::new("own_v03", "/own_v03"),
        ))
        .unwrap_or_else(|e| panic!("graph: {e:?}"));
    let package = lower_package(&graph, ResolutionMode::Namespaced)
        .unwrap_or_else(|e| panic!("lower: {e:?}"));
    let mut types = infer_hir_package(&package, ShadowInferenceOptions::default())
        .unwrap_or_else(|e| panic!("types: {e:?}"));
    let program = build_mir(
        &package,
        &graph.sources,
        &mut types,
        MirBuildOptions::default(),
    )
    .unwrap_or_else(|e| panic!("mir: {e:?}"));
    assert!(verify_mir(&program, &types.interner).is_empty());
    let plan = compute_ownership_plan(&program, &types.interner)
        .unwrap_or_else(|e| panic!("ownership plan: {e:?}"));
    let plan_errors = verify_ownership_plan(&program, &types.interner, &plan);
    assert!(
        plan_errors.is_empty(),
        "ownership plan verification failed: {plan_errors:?}"
    );
    (program, types.interner, plan, package)
}

fn main_fn(program: &MirProgram, package: &HirPackage) -> MirFunctionId {
    program
        .functions()
        .find(|(_, f)| f.name.and_then(|sym| package.names.symbols.resolve(sym)) == Some("main"))
        .map(|(id, _)| id)
        .expect("entry function")
}

#[test]
fn non_escaping_local_structs_and_tuples_are_placed_in_frame_arena() {
    let src = r#"
struct Point:
    x: Int
    y: Int

def local_geometry(a: Int, b: Int) -> Int:
    p := Point(a, b)
    q := Point(a * 2, b * 3)
    tup := (p.x + q.x, p.y + q.y, 10)
    return tup.0 + tup.1 + tup.2

def main() -> Int:
    return local_geometry(4, 5)
"#;
    let (program, types, plan, _) = compile_and_plan(src);
    assert!(
        plan.stats.frame_cell_count > 0,
        "non-escaping Point and tuple locals must be placed in Frame arena: {:?}",
        plan.stats
    );
    assert!(
        !plan.arenas.is_empty(),
        "expected non-empty per-function ArenaPlan"
    );
    assert!(
        plan.pinned_types().is_empty(),
        "acyclic Point/tuple program must have zero Shared pinned types"
    );
    let balance_errors = verify_ownership_balance(&program, &types, &plan);
    assert!(
        balance_errors.is_empty(),
        "balance errors: {balance_errors:?}"
    );
}

#[test]
fn returned_and_escaping_acyclic_aggregates_are_owned_not_shared() {
    let src = r#"
struct Config:
    port: Int
    host: Str
    flags: (Int, Bool)

def make_config(p: Int) -> Config:
    return Config(p, "localhost", (1, true))

def main() -> Int:
    c := make_config(8080)
    f := c.flags
    if f.1 && str_len(c.host) == 9:
        return c.port + f.0
    return 0
"#;
    let (program, types, plan, _) = compile_and_plan(src);
    assert!(
        plan.stats.owned_cell_count > 0,
        "returned Config aggregate must produce Owned cells: {:?}",
        plan.stats
    );
    assert_eq!(
        plan.stats.shared_node_count, 0,
        "acyclic Config must remain TypeStrategy::Owned"
    );
    assert!(
        plan.nodes.iter().all(|n| n.strategy == TypeStrategy::Owned),
        "all containment nodes must be Owned"
    );
    assert!(verify_ownership_balance(&program, &types, &plan).is_empty());
}

#[test]
fn self_referential_and_mutually_recursive_types_are_classified_as_shared() {
    let src = r#"
struct Child:
    tag: Int
    parent: Parent

struct Parent:
    tag: Int
    kid: Child

def main() -> Int:
    mut c := Child(2, 0)
    mut p := Parent(1, 0)
    p.kid = c
    c.parent = p
    return c.parent.kid.tag
"#;
    let (program, types, plan, _) = compile_and_plan(src);
    assert!(
        plan.stats.cycle_count >= 1,
        "expected Child <-> Parent cycle: {:?}",
        plan.stats
    );
    assert!(
        plan.stats.shared_node_count >= 2,
        "Child and Parent must be marked TypeStrategy::Shared: {:?}",
        plan.stats
    );
    assert!(!plan.pinned_types().is_empty());
    assert!(verify_ownership_balance(&program, &types, &plan).is_empty());
}

#[test]
fn spawn_boundaries_mark_concurrent_cells_and_types_distinct_from_cycles() {
    let src = r#"
struct Payload:
    id: Int
    name: Str

def main() -> Int:
    m := mutex_new()
    log := list_new()
    item := Payload(40, "ok")
    worker := fn():
        mutex_lock(m)
        list_push(log, item.id + str_len(item.name))
        mutex_unlock(m)
    spawn worker
    mutex_lock(m)
    val := list_get(log, 0)
    mutex_unlock(m)
    return val
"#;
    let (program, types, plan, _) = compile_and_plan(src);
    assert!(
        plan.stats.concurrent_cell_count > 0,
        "Payload crossing spawn boundary must mark concurrent cells: {:?}",
        plan.stats
    );
    assert!(
        !plan.concurrent_types().is_empty(),
        "Payload/List type must be marked concurrent"
    );
    assert!(
        plan.pinned_types().is_empty(),
        "acyclic concurrent Payload must NOT be conflated with cyclic Shared types"
    );
    assert!(verify_ownership_balance(&program, &types, &plan).is_empty());
}

#[test]
fn raw_pointers_and_unsafe_casts_are_excluded_from_managed_containment() {
    let src = r#"
struct DmaDescriptor:
    phys_addr: Ptr[Int]
    raw_buf: RawPtr
    len: Int

unsafe def init_desc(addr: Int, n: Int) -> DmaDescriptor:
    p := addr as Ptr[Int]
    r := addr as RawPtr
    return DmaDescriptor(p, r, n)

def main() -> Int:
    unsafe:
        d := init_desc(0x1000, 64)
        return (d.phys_addr as Int) + d.len
"#;
    let (program, types, plan, pkg) = compile_and_plan(src);
    assert!(
        plan.pinned_types().is_empty(),
        "Ptr[T] and RawPtr fields must not introduce ownership cycles"
    );
    assert!(verify_ownership_balance(&program, &types, &plan).is_empty());
    let outcome = execute_mir_arc(
        &program,
        &types,
        main_fn(&program, &pkg),
        &[],
        InterpreterLimits::default(),
        &plan.pinned_types(),
    )
    .expect("execute_mir_arc");
    assert_eq!(outcome.value, ExecutionValue::Int(0x1000 + 64));
}

#[test]
fn optimization_passes_preserve_ownership_balance_and_arc_stats() {
    let programs = [
        (
            r#"
struct Pair:
    a: Int
    b: Int

def calc(x: Int) -> Int:
    p := Pair(x + 1, x * 2)
    if true:
        return p.a + p.b
    else:
        return 0

def main() -> Int:
    return calc(10) + calc(20)
"#,
            ExecutionValue::Int(92),
        ),
        (
            r#"
enum ResultBox:
    Ok(v: Int, label: Str)
    Err(msg: Str)

def unwrap_or(r: ResultBox, fallback: Int) -> Int:
    match r:
        ResultBox.Ok(v, label) if str_len(label) > 0:
            return v + str_len(label)
        ResultBox.Ok(v, _):
            return v
        ResultBox.Err(_):
            return fallback

def main() -> Int:
    a := unwrap_or(ResultBox.Ok(50, "abc"), 0)
    b := unwrap_or(ResultBox.Err("bad"), 9)
    return a + b
"#,
            ExecutionValue::Int(62),
        ),
    ];

    for (src, expected) in programs {
        let (mut program, types, plan_before, pkg) = compile_and_plan(src);
        let entry = main_fn(&program, &pkg);
        let run_before = execute_mir_arc(
            &program,
            &types,
            entry,
            &[],
            InterpreterLimits::default(),
            &plan_before.pinned_types(),
        )
        .expect("unoptimized arc run");
        assert_eq!(run_before.value, expected);

        run_optimization(&mut program, &types, OptimizationLevel::O3).expect("O3 optimization");
        assert!(verify_mir(&program, &types).is_empty());

        let plan_after = compute_ownership_plan(&program, &types).expect("plan after O3");
        assert!(verify_ownership_plan(&program, &types, &plan_after).is_empty());
        assert!(verify_ownership_balance(&program, &types, &plan_after).is_empty());

        let run_after = execute_mir_arc(
            &program,
            &types,
            entry,
            &[],
            InterpreterLimits::default(),
            &plan_after.pinned_types(),
        )
        .expect("optimized arc run");
        assert_eq!(run_after.value, expected);
        assert_eq!(run_after.arc.unwrap().pinned_live, 0);
    }
}

#[test]
fn pass_manager_establishes_ownership_and_balance_invariants_and_snapshot_is_stable() {
    let src = r#"
struct Item:
    code: Int
    tag: Str

def main() -> Int:
    it := Item(42, "tag")
    return it.code + str_len(it.tag)
"#;
    let (mut program, types, plan, _) = compile_and_plan(src);
    assert!(
        plan.cells
            .iter()
            .any(|c| c.placement == ValuePlacement::Frame),
        "expected ValuePlacement::Frame cells"
    );
    let snap1 = ownership_plan_snapshot(&plan);
    let snap2 = ownership_plan_snapshot(&plan);
    assert_eq!(snap1, snap2);

    let mut pm = PassManager::new();
    pm.push(OwnershipPlanPass);
    pm.push(OwnershipBalancePass);
    let outcome = pm.run(&mut program, &types).expect("ownership passes");
    assert!(outcome.established.contains(&MirInvariant::Ownership));
    assert!(outcome.established.contains(&MirInvariant::NoOwningCycles));
    assert!(
        outcome
            .established
            .contains(&MirInvariant::OwnershipBalance)
    );
}
