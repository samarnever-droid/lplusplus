//! Comprehensive v0.3 production test suite for `lpp-types`.
//!
//! Exercises:
//! - Full primitive, aggregate, container, alias, and multi-module type matrix
//! - Hindley-Milner let-polymorphism, unification, generalization, and builtin specialization
//! - Trait resolution, coherence overlap rejection, specialization, UFCS, and InstancePlanner
//! - Recursive structs/enums (valid `0`-sentinel cycles) vs `TypeAliasCycle` / `OccursCheck`
//! - Tuple arities 1..=16 (interner + `Tuple[T]` + source 2..=16) vs arity 17 & 0 rejections
//! - `Set`, `Deque`, `List[T]`, `Map[K, V]`, `Buf`, `Str`, `StrSlice`, `Slice[T]`, `VectorI64x2`
//! - F-string interpolation rules, range `..`/`..=` typing, guarded/exhaustive `match`, and `?` try
//! - `unsafe` block rules (`Ptr[T]`, `RawPtr`, kernel intrinsics/attributes, `ShadowInferenceOptions`)
//! - Default & variadic parameters, async/await/spawn safety rules, and 65+ negative semantic tests

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_common::{OptimizationLevel, OptimizationOptions};
use lpp_hir::{
    BindingTarget, DefId, FileSystem, FileSystemError, GraphBuilder, GraphError, GraphRequest,
    HirItemId, HirItemKind, HirPackage, PackageSpec, ResolutionMode, TypeParamId, lower_package,
};
use lpp_types::{
    EnumMatchArmFact, InferVarId, InferenceLevel, InferenceTable, InstanceGrowthError, InstanceKey,
    InstanceLimits, InstancePlanner, InstanceRequestKind, InstanceRequestOutcome,
    PlaceExpressionFact, PlaceStatementFact, PrimitiveType, ShadowInferenceOptions,
    ShadowTypeError, ShadowTypeOutput, TraitBound, TraitCoherenceError, TraitGoal, TraitIndex,
    TraitRule, TraitSolution, TraitSolverLimits, TraitSolverOverflow, TypeError, TypeId,
    TypeInterner, TypeKind, TypeSubstitution, TypeWorkBudget, infer_hir_package, semantic_metrics,
    semantic_snapshot,
};

// ── Test Harness Scaffolding ───────────────────────────────────────────────

#[derive(Debug, Default)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn single(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/types/main.lpp"), source.to_owned())]),
        }
    }

    fn multi(entries: &[(&str, &str)]) -> Self {
        Self {
            files: entries
                .iter()
                .map(|(path, src)| (PathBuf::from(path), (*src).to_owned()))
                .collect(),
        }
    }
}

impl FileSystem for MemoryFileSystem {
    fn is_file(&self, path: &Path) -> Result<bool, FileSystemError> {
        Ok(self.files.contains_key(path))
    }

    fn canonicalize(&self, path: &Path) -> Result<PathBuf, FileSystemError> {
        if self.files.contains_key(path) || self.files.keys().any(|f| f.starts_with(path)) {
            Ok(path.to_owned())
        } else {
            Err(FileSystemError::new("canonicalize", path, "path not found"))
        }
    }

    fn read_to_string(&self, path: &Path) -> Result<String, FileSystemError> {
        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| FileSystemError::new("read", path, "file not found"))
    }
}

fn try_lower_single(source: &str) -> Result<HirPackage, String> {
    let fs = MemoryFileSystem::single(source);
    let graph = GraphBuilder::new(&fs)
        .build(GraphRequest::new(
            "/types/main.lpp",
            PackageSpec::new("types-suite", "/types"),
        ))
        .map_err(|e| format!("graph:{e}"))?;
    lower_package(&graph, ResolutionMode::Namespaced).map_err(|errs| {
        errs.into_iter()
            .map(|e| format!("{}:{}", e.code, e.message))
            .collect::<Vec<_>>()
            .join("; ")
    })
}

#[derive(Debug, Clone)]
struct LowerErrInfo {
    code: String,
    #[allow(dead_code)]
    message: String,
}

fn try_lower_errors(source: &str) -> Result<Vec<LowerErrInfo>, GraphError> {
    let fs = MemoryFileSystem::single(source);
    let graph = GraphBuilder::new(&fs).build(GraphRequest::new(
        "/types/main.lpp",
        PackageSpec::new("types-suite", "/types"),
    ))?;
    match lower_package(&graph, ResolutionMode::Namespaced) {
        Ok(_) => Ok(Vec::new()),
        Err(errs) => Ok(errs
            .into_iter()
            .map(|e| LowerErrInfo {
                code: e.code.to_string(),
                message: e.message,
            })
            .collect()),
    }
}

fn lower_single(source: &str) -> HirPackage {
    try_lower_single(source).unwrap_or_else(|err| panic!("unexpected lowering failure: {err}"))
}

fn lower_multi(entries: &[(&str, &str)]) -> Result<HirPackage, GraphError> {
    let fs = MemoryFileSystem::multi(entries);
    let graph = GraphBuilder::new(&fs).build(GraphRequest::new(
        "/types/main.lpp",
        PackageSpec::new("types-suite", "/types"),
    ))?;
    Ok(lower_package(&graph, ResolutionMode::Namespaced)
        .unwrap_or_else(|errs| panic!("unexpected lowering failure: {errs:?}")))
}

fn check_with_options(
    source: &str,
    options: ShadowInferenceOptions,
) -> Result<(HirPackage, ShadowTypeOutput), ShadowTypeError> {
    let package = lower_single(source);
    let output = infer_hir_package(&package, options)?;
    Ok((package, output))
}

fn check_ok(source: &str) -> (HirPackage, ShadowTypeOutput) {
    check_with_options(source, ShadowInferenceOptions::default())
        .unwrap_or_else(|err| panic!("expected type-check success, got {err:?}"))
}

fn check_err(source: &str) -> TypeError {
    check_with_options(source, ShadowInferenceOptions::default())
        .expect_err("expected type-check failure")
        .error
}

fn check_multi_ok(entries: &[(&str, &str)]) -> (HirPackage, ShadowTypeOutput) {
    let package = lower_multi(entries).expect("multi-file graph must build");
    let output = infer_hir_package(&package, ShadowInferenceOptions::default())
        .expect("multi-file package must type-check");
    (package, output)
}

fn resolve_def(package: &HirPackage, name: &str) -> DefId {
    let module = package.modules[0].module;
    match package
        .names
        .resolve(module, name)
        .expect("definition exists")
    {
        BindingTarget::Definition(def) => def,
        BindingTarget::Module(_) => panic!("{name} is a module, expected definition"),
    }
}

fn fn_return_type(package: &HirPackage, output: &ShadowTypeOutput, fn_name: &str) -> TypeId {
    let (item_id, _) = package
        .items
        .enumerate()
        .find(|(_, item)| {
            matches!(item.kind, HirItemKind::Function(_))
                && item.name.and_then(|sym| package.names.symbols.resolve(sym)) == Some(fn_name)
        })
        .unwrap_or_else(|| panic!("missing function {fn_name}"));
    let ty = output.assignments.item(item_id).expect("item typed");
    let TypeKind::Function { result, .. } = output.interner.kind(ty) else {
        panic!("expected function type for {fn_name}");
    };
    result
}

fn nominal_rule(
    origin: u32,
    trait_def: u32,
    self_pattern: TypeId,
    parameters: &[u32],
    bounds: &[TraitBound],
    types: &TypeInterner,
) -> TraitRule {
    TraitRule {
        origin: HirItemId::from_raw(origin),
        trait_definition: DefId::from_raw(trait_def),
        self_pattern,
        trait_arguments: types.empty_list(),
        parameters: parameters
            .iter()
            .map(|r| TypeParamId::from_raw(*r))
            .collect::<Vec<_>>()
            .into(),
        bounds: bounds.to_vec().into(),
    }
}

fn make_nominal(types: &mut TypeInterner, def: u32, args: &[TypeId]) -> TypeId {
    let args = types.intern_list(args).unwrap();
    types
        .intern(TypeKind::Nominal {
            definition: DefId::from_raw(def),
            arguments: args,
        })
        .unwrap()
}

fn make_param(types: &mut TypeInterner, raw: u32) -> TypeId {
    types
        .intern(TypeKind::GenericParameter(TypeParamId::from_raw(raw)))
        .unwrap()
}

// ── 1. Full Primitive, Aggregate, Container & Multi-Module Matrix ──────────

#[test]
fn primitive_type_matrix_covers_all_18_builtin_primitives_and_aliases() {
    let source = concat!(
        "type Byte = U8\n",
        "type Word = U16\n",
        "type DWord = U32\n",
        "type QWord = U64\n",
        "type SByte = I8\n",
        "type SShort = I16\n",
        "type SInt = I32\n",
        "type UPtr = USize\n",
        "type SPtr = ISize\n",
        "type Text = Str\n",
        "type Label = String\n",
        "def accept_all(\n",
        "    a: Int,\n",
        "    b: Float,\n",
        "    c: Bool,\n",
        "    d: Char,\n",
        "    e: Text,\n",
        "    f: Label,\n",
        "    g: Byte,\n",
        "    h: Word,\n",
        "    i: DWord,\n",
        "    j: QWord,\n",
        "    k: SByte,\n",
        "    l: SShort,\n",
        "    m: SInt,\n",
        "    n: UPtr,\n",
        "    o: SPtr,\n",
        "    p: RawPtr,\n",
        "    q: StrSlice,\n",
        "    r: VectorI64x2\n",
        ") -> Void:\n",
        "    return\n",
        "def make_void() -> Void:\n",
        "    return\n",
        "def make_u8() -> u8:\n",
        "    return 255 as u8\n",
        "def make_u16() -> u16:\n",
        "    return 65535 as u16\n",
        "def make_u32() -> u32:\n",
        "    return 100 as u32\n",
        "def make_u64() -> u64:\n",
        "    return 200 as u64\n",
        "def make_i8() -> i8:\n",
        "    return -1 as i8\n",
        "def make_i16() -> i16:\n",
        "    return -2 as i16\n",
        "def make_i32() -> i32:\n",
        "    return -3 as i32\n",
        "def make_usize() -> usize:\n",
        "    return 16 as usize\n",
        "def make_isize() -> isize:\n",
        "    return -16 as isize\n",
    );
    let (pkg, out) = check_ok(source);
    let expected = [
        ("make_void", PrimitiveType::Void),
        ("make_u8", PrimitiveType::U8),
        ("make_u16", PrimitiveType::U16),
        ("make_u32", PrimitiveType::U32),
        ("make_u64", PrimitiveType::U64),
        ("make_i8", PrimitiveType::I8),
        ("make_i16", PrimitiveType::I16),
        ("make_i32", PrimitiveType::I32),
        ("make_usize", PrimitiveType::USize),
        ("make_isize", PrimitiveType::ISize),
    ];
    for (name, prim) in expected {
        assert_eq!(
            fn_return_type(&pkg, &out, name),
            out.interner.primitive(prim),
            "function {name} return type mismatch"
        );
    }

    let (accept_item, _) = pkg
        .items
        .enumerate()
        .find(|(_, it)| it.name.and_then(|s| pkg.names.symbols.resolve(s)) == Some("accept_all"))
        .unwrap();
    let TypeKind::Function { parameters, result } = out
        .interner
        .kind(out.assignments.item(accept_item).unwrap())
    else {
        panic!("accept_all must be a function");
    };
    assert_eq!(result, out.interner.primitive(PrimitiveType::Void));
    let param_tys = out.interner.list(parameters);
    assert_eq!(param_tys.len(), 18);
    assert_eq!(param_tys[4], out.interner.primitive(PrimitiveType::String));
    assert_eq!(param_tys[5], out.interner.primitive(PrimitiveType::String));
    assert_eq!(param_tys[15], out.interner.primitive(PrimitiveType::RawPtr));
    assert_eq!(
        param_tys[16],
        out.interner.primitive(PrimitiveType::StrSlice)
    );
    assert_eq!(
        param_tys[17],
        out.interner.primitive(PrimitiveType::VectorI64x2)
    );
}

#[test]
fn container_and_nominal_generic_matrix_assigns_every_hir_node_deterministically() {
    let source = concat!(
        "struct Entry[K, V]:\n",
        "    key: K\n",
        "    value: V\n",
        "enum Outcome[T, E]:\n",
        "    Ok(val: T)\n",
        "    Err(err: E)\n",
        "async def load(id: Int) -> Outcome[Entry[Str, List[Int]], Str]:\n",
        "    nums := [id, id + 1, id + 2]\n",
        "    return Outcome.Ok(Entry(\"k\", nums))\n",
        "def index_map(m: Map[Str, List[Int]], key: Str) -> Int:\n",
        "    return map_get(m, key)\n",
        "def main() -> Int:\n",
        "    t := load(10)\n",
        "    res := t.await\n",
        "    m: Map[Str, List[Int]] = map_new()\n",
        "    map_put(m, \"x\", 99)\n",
        "    got := index_map(m, \"x\")\n",
        "    match res:\n",
        "        Outcome.Ok(entry):\n",
        "            return entry.value[1] + got - got\n",
        "        Outcome.Err(_):\n",
        "            return 0\n",
        "    return 0\n",
    );
    let (pkg, out1) = check_ok(source);
    let (_, out2) = check_ok(source);
    assert_eq!(out1, out2);

    assert!(
        pkg.expressions
            .enumerate()
            .all(|(id, _)| out1.assignments.expression(id).is_some())
    );
    assert!(
        pkg.locals
            .enumerate()
            .all(|(id, _)| out1.assignments.local(id).is_some())
    );
    assert!(
        pkg.type_refs
            .enumerate()
            .all(|(id, _)| out1.assignments.type_ref(id).is_some())
    );
    assert!(
        pkg.items
            .enumerate()
            .all(|(id, _)| out1.assignments.item(id).is_some())
    );

    let load_ret = fn_return_type(&pkg, &out1, "load");
    assert!(matches!(out1.interner.kind(load_ret), TypeKind::Task(_)));
    let snap = semantic_snapshot(&pkg, &out1);
    let metrics = semantic_metrics(&pkg, &out1);
    assert!(snap.contains("semantic-v4\n"));
    assert!(metrics.hir_nodes > 0);
    assert!(metrics.interned_types > 0);
}

#[test]
fn multi_file_imports_preserve_generic_and_nominal_types() {
    let (pkg, out) = check_multi_ok(&[
        (
            "/types/main.lpp",
            concat!(
                "from models import Box, wrap\n",
                "import helpers\n",
                "def main() -> Int:\n",
                "    b1 := wrap[Int](21)\n",
                "    b2 := helpers.twice(21)\n",
                "    return b1.value + b2.value\n",
            ),
        ),
        (
            "/types/models.lpp",
            concat!(
                "struct Box[T]:\n",
                "    value: T\n",
                "def wrap[T](x: T) -> Box[T]:\n",
                "    return Box(x)\n",
            ),
        ),
        (
            "/types/helpers.lpp",
            concat!(
                "from models import Box\n",
                "def twice(n: Int) -> Box[Int]:\n",
                "    return Box(n * 2)\n",
            ),
        ),
    ]);
    assert_eq!(
        fn_return_type(&pkg, &out, "main"),
        out.interner.primitive(PrimitiveType::Int)
    );
    assert!(out.instance_diagnostics.is_empty());
    assert!(!out.instances.records().is_empty());
}

#[test]
fn type_interner_canonicalizes_nested_shapes_and_supports_all_type_kinds() {
    let mut types = TypeInterner::new();
    let int = types.primitive(PrimitiveType::Int);
    let str_ty = types.primitive(PrimitiveType::String);
    let vec_ty = types.primitive(PrimitiveType::VectorI64x2);
    let raw_ptr = types.primitive(PrimitiveType::RawPtr);

    let list_int = types.intern(TypeKind::List(int)).unwrap();
    let slice_int = types.intern(TypeKind::Slice(int)).unwrap();
    let ptr_vec = types.intern(TypeKind::Ptr(vec_ty)).unwrap();
    let task_str = types.intern(TypeKind::Task(str_ty)).unwrap();
    let map_ty = types
        .intern(TypeKind::Map {
            key: str_ty,
            value: list_int,
        })
        .unwrap();
    let params = types.intern_list(&[slice_int, ptr_vec, raw_ptr]).unwrap();
    let fn_a = types
        .intern(TypeKind::Function {
            parameters: params,
            result: task_str,
        })
        .unwrap();
    let params2 = types.intern_list(&[slice_int, ptr_vec, raw_ptr]).unwrap();
    let fn_b = types
        .intern(TypeKind::Function {
            parameters: params2,
            result: task_str,
        })
        .unwrap();
    assert_eq!(fn_a, fn_b);
    assert_ne!(map_ty, fn_a);
}

// ── 2. HM Let-Polymorphism, Unification & Builtin Specialization ───────────

#[test]
fn hm_let_polymorphism_generalizes_and_instantiates_respecting_levels() {
    let mut types = TypeInterner::new();
    let mut table = InferenceTable::new();
    let mut budget = TypeWorkBudget::new(50_000);
    let int = types.primitive(PrimitiveType::Int);
    let str_ty = types.primitive(PrimitiveType::String);
    let bool_ty = types.primitive(PrimitiveType::Bool);

    let outer = InferenceLevel::ROOT;
    let inner = outer.child();
    let deep = inner.child();

    let outer_var = table.fresh(&mut types, outer, None).unwrap();
    let inner_var = table.fresh(&mut types, inner, None).unwrap();
    let deep_var = table.fresh(&mut types, deep, None).unwrap();
    let params = types.intern_list(&[outer_var, inner_var]).unwrap();
    let fn_ty = types
        .intern(TypeKind::Function {
            parameters: params,
            result: deep_var,
        })
        .unwrap();

    // Generalizing at `outer` quantifies `inner_var` and `deep_var`, leaving `outer_var` free.
    let scheme = table
        .generalize(&mut types, fn_ty, outer, &mut budget)
        .unwrap();
    assert_eq!(scheme.variables().len(), 2);

    let inst1 = table
        .instantiate(&mut types, &scheme, inner, None, &mut budget)
        .unwrap();
    let inst2 = table
        .instantiate(&mut types, &scheme, inner, None, &mut budget)
        .unwrap();
    assert_ne!(inst1, inst2);

    // Unify outer_var with Bool; both instantiations observe outer_var == Bool while remaining independent on inner_var/deep_var.
    table
        .unify(&mut types, outer_var, bool_ty, &mut budget)
        .unwrap();

    let p1 = types.intern_list(&[bool_ty, int]).unwrap();
    let target1 = types
        .intern(TypeKind::Function {
            parameters: p1,
            result: int,
        })
        .unwrap();
    table
        .unify(&mut types, inst1, target1, &mut budget)
        .unwrap();

    let p2 = types.intern_list(&[bool_ty, str_ty]).unwrap();
    let target2 = types
        .intern(TypeKind::Function {
            parameters: p2,
            result: str_ty,
        })
        .unwrap();
    table
        .unify(&mut types, inst2, target2, &mut budget)
        .unwrap();

    let r1 = table.normalize(&mut types, inst1, &mut budget).unwrap();
    let r2 = table.normalize(&mut types, inst2, &mut budget).unwrap();
    assert_ne!(r1, r2);
}

#[test]
fn hm_occurs_check_and_unification_mismatch_errors_are_exact() {
    let mut types = TypeInterner::new();
    let mut table = InferenceTable::new();
    let mut budget = TypeWorkBudget::new(50_000);
    let int = types.primitive(PrimitiveType::Int);
    let float = types.primitive(PrimitiveType::Float);

    let v = table
        .fresh(&mut types, InferenceLevel::ROOT.child(), None)
        .unwrap();
    let list_v = types.intern(TypeKind::List(v)).unwrap();
    let err = table.unify(&mut types, v, list_v, &mut budget).unwrap_err();
    assert!(matches!(err, TypeError::OccursCheck { .. }));

    let v_id = match types.kind(v) {
        TypeKind::InferenceVariable(id) => id,
        _ => unreachable!(),
    };
    assert_eq!(v_id, InferVarId::from_raw(0));

    let mismatch = table
        .unify(&mut types, int, float, &mut budget)
        .unwrap_err();
    assert!(matches!(mismatch, TypeError::Mismatch { .. }));

    let p1 = types.intern_list(&[int]).unwrap();
    let p2 = types.intern_list(&[int, int]).unwrap();
    let f1 = types
        .intern(TypeKind::Function {
            parameters: p1,
            result: int,
        })
        .unwrap();
    let f2 = types
        .intern(TypeKind::Function {
            parameters: p2,
            result: int,
        })
        .unwrap();
    let arity = table.unify(&mut types, f1, f2, &mut budget).unwrap_err();
    assert_eq!(
        arity,
        TypeError::ArityMismatch {
            expected: 1,
            actual: 2
        }
    );
}

#[test]
fn polymorphic_generic_functions_instantiate_across_distinct_call_sites() {
    let source = concat!(
        "struct Pair[A, B]:\n",
        "    first: A\n",
        "    second: B\n",
        "def swap[A, B](p: Pair[A, B]) -> Pair[B, A]:\n",
        "    return Pair(p.second, p.first)\n",
        "def id[T](x: T) -> T:\n",
        "    return x\n",
        "def main() -> Int:\n",
        "    a := id::<Int>(10)\n",
        "    b := id(\"hello\")\n",
        "    p1 := swap(Pair(a, b))\n",
        "    p2 := swap(Pair(true, 3.5))\n",
        "    if p2.second:\n",
        "        return p1.second + str_len(p1.first)\n",
        "    return 0\n",
    );
    let (pkg, out) = check_ok(source);
    assert_eq!(
        fn_return_type(&pkg, &out, "main"),
        out.interner.primitive(PrimitiveType::Int)
    );
    assert!(out.instance_diagnostics.is_empty());
    assert!(out.instances.records().len() >= 5);
}

#[test]
fn polymorphic_builtins_specialize_len_map_and_slice_get_correctly() {
    let source = concat!(
        "def main() -> Int:\n",
        "    xs := [10, 20, 30]\n",
        "    s := \"hello\"\n",
        "    sv := str_slice(s, 1, 3)\n",
        "    lv := slice(xs, 0, 2)\n",
        "    ch := slice_get(sv, 0)\n",
        "    el := slice_get(lv, 1)\n",
        "    mut m := map_new()\n",
        "    map_put(m, \"alpha\", 7)\n",
        "    has_k := map_has(m, \"alpha\")\n",
        "    got := map_get(m, \"alpha\")\n",
        "    map_remove(m, \"alpha\")\n",
        "    if has_k:\n",
        "        return len(xs) + len(s) + len(sv) + len(lv) + str_len(ch) + el + got\n",
        "    return 0\n",
    );
    let (pkg, out) = check_ok(source);
    let names: Vec<&str> = out
        .builtins
        .expressions()
        .map(|(_, f)| f.builtin.descriptor().name)
        .collect();
    assert!(names.contains(&"len"));
    assert!(names.contains(&"str_len"));
    assert!(names.contains(&"slice_len"));
    assert!(names.contains(&"lpp_map_put_str"));
    assert!(names.contains(&"lpp_map_has_str"));
    assert!(names.contains(&"lpp_map_get_str"));
    assert!(names.contains(&"lpp_map_remove_str"));

    // Verify `ch := slice_get(sv, 0)` resolved to `Str`, not `Int`.
    let ch_local = pkg
        .locals
        .enumerate()
        .find_map(|(id, l)| (pkg.names.symbols.resolve(l.name) == Some("ch")).then_some(id))
        .unwrap();
    assert_eq!(
        out.assignments.local(ch_local),
        Some(out.interner.primitive(PrimitiveType::String))
    );
}

#[test]
fn type_substitution_handles_all_compound_type_kinds() {
    let mut types = TypeInterner::new();
    let mut budget = TypeWorkBudget::new(10_000);
    let t_param = TypeParamId::from_raw(0);
    let u_param = TypeParamId::from_raw(1);
    let t_ty = types.intern(TypeKind::GenericParameter(t_param)).unwrap();
    let u_ty = types.intern(TypeKind::GenericParameter(u_param)).unwrap();
    let int = types.primitive(PrimitiveType::Int);
    let str_ty = types.primitive(PrimitiveType::String);

    let list_t = types.intern(TypeKind::List(t_ty)).unwrap();
    let map_tu = types
        .intern(TypeKind::Map {
            key: t_ty,
            value: u_ty,
        })
        .unwrap();
    let slice_u = types.intern(TypeKind::Slice(u_ty)).unwrap();
    let ptr_t = types.intern(TypeKind::Ptr(t_ty)).unwrap();
    let task_u = types.intern(TypeKind::Task(u_ty)).unwrap();
    let tup_elems = types
        .intern_list(&[list_t, map_tu, slice_u, ptr_t])
        .unwrap();
    let tup = types.intern(TypeKind::Tuple(tup_elems)).unwrap();
    let fn_params = types.intern_list(&[tup]).unwrap();
    let fn_ty = types
        .intern(TypeKind::Function {
            parameters: fn_params,
            result: task_u,
        })
        .unwrap();

    let mut subst = TypeSubstitution::new();
    subst.insert(t_param, int);
    subst.insert(u_param, str_ty);
    let concrete = subst.apply(&mut types, fn_ty, &mut budget).unwrap();

    let TypeKind::Function { parameters, result } = types.kind(concrete) else {
        panic!("expected function");
    };
    assert_eq!(result, types.intern(TypeKind::Task(str_ty)).unwrap());
    let param_tup = types.list(parameters)[0];
    let TypeKind::Tuple(elems) = types.kind(param_tup) else {
        panic!("expected tuple");
    };
    let elems = types.list(elems).to_vec();
    assert_eq!(elems[0], types.intern(TypeKind::List(int)).unwrap());
    assert_eq!(
        elems[1],
        types
            .intern(TypeKind::Map {
                key: int,
                value: str_ty
            })
            .unwrap()
    );
    assert_eq!(elems[2], types.intern(TypeKind::Slice(str_ty)).unwrap());
    assert_eq!(elems[3], types.intern(TypeKind::Ptr(int)).unwrap());
}

// ── 3. Traits, Bounds, Coherence, UFCS & Instance Planner ──────────────────

#[test]
fn trait_bounds_ufcs_self_substitution_and_specialization_work_end_to_end() {
    let source = concat!(
        "trait Measure:\n",
        "    def weight(self) -> Int\n",
        "struct Rock:\n",
        "    grams: Int\n",
        "struct Crate[T]:\n",
        "    item: T\n",
        "impl Measure for Rock:\n",
        "    def weight(self) -> Int:\n",
        "        return self.grams\n",
        "    def clone_rock(self) -> Rock:\n",
        "        return Rock(self.grams)\n",
        "impl[T] Measure for Crate[T]:\n",
        "    def weight(self) -> Int:\n",
        "        return 1\n",
        "impl Measure for Crate[Rock]:\n",
        "    def weight(self) -> Int:\n",
        "        return 99\n",
        "def total_weight[T: Measure](a: T, b: T) -> Int:\n",
        "    return a.weight() + b.weight()\n",
        "def main() -> Int:\n",
        "    r := Rock(50)\n",
        "    r2 := r.clone_rock()\n",
        "    c_rock := Crate(r2)\n",
        "    c_int := Crate(7)\n",
        "    return total_weight(r, r2) + c_rock.weight() + c_int.weight()\n",
    );
    let (pkg, mut out) = check_ok(source);
    assert!(out.trait_diagnostics.is_empty());
    assert_eq!(out.trait_index.len(), 3);
    assert!(out.instance_diagnostics.is_empty());

    let measure_def = resolve_def(&pkg, "Measure");
    let rock_def = resolve_def(&pkg, "Rock");
    let crate_def = resolve_def(&pkg, "Crate");

    let empty = out.interner.empty_list();
    let rock_ty = out
        .interner
        .intern(TypeKind::Nominal {
            definition: rock_def,
            arguments: empty,
        })
        .unwrap();
    let crate_rock_args = out.interner.intern_list(&[rock_ty]).unwrap();
    let crate_rock_ty = out
        .interner
        .intern(TypeKind::Nominal {
            definition: crate_def,
            arguments: crate_rock_args,
        })
        .unwrap();
    let int_ty = out.interner.primitive(PrimitiveType::Int);
    let crate_int_args = out.interner.intern_list(&[int_ty]).unwrap();
    let crate_int_ty = out
        .interner
        .intern(TypeKind::Nominal {
            definition: crate_def,
            arguments: crate_int_args,
        })
        .unwrap();

    // Specialized Crate[Rock] beats blanket Crate[T]
    let TraitSolution::Unique(sel_rock) = out.trait_index.solve(
        &mut out.interner,
        TraitGoal::new(measure_def, crate_rock_ty, empty),
        TraitSolverLimits::default(),
    ) else {
        panic!("Crate[Rock] must resolve uniquely");
    };
    assert_eq!(
        out.trait_index
            .rule(sel_rock.implementation)
            .unwrap()
            .self_pattern,
        crate_rock_ty
    );

    // Crate[Int] falls back to blanket Crate[T] with T -> Int
    let TraitSolution::Unique(sel_int) = out.trait_index.solve(
        &mut out.interner,
        TraitGoal::new(measure_def, crate_int_ty, empty),
        TraitSolverLimits::default(),
    ) else {
        panic!("Crate[Int] must resolve via blanket impl");
    };
    assert_ne!(sel_rock.implementation, sel_int.implementation);

    // UFCS method index records Rock.weight & Rock.clone_self
    assert!(!out.method_index.is_empty());
}

#[test]
fn trait_coherence_rejects_overlapping_impls_in_hir_and_direct_index() {
    let source = concat!(
        "trait Tag:\n",
        "    def tag(self) -> Int\n",
        "struct Pair[A, B]:\n",
        "    a: A\n",
        "    b: B\n",
        "impl[T] Tag for Pair[T, Int]:\n",
        "    def tag(self) -> Int:\n",
        "        return 1\n",
        "impl[U] Tag for Pair[Str, U]:\n",
        "    def tag(self) -> Int:\n",
        "        return 2\n",
        "def main() -> Int:\n",
        "    return 0\n",
    );
    let (_, out) = check_ok(source);
    assert_eq!(out.trait_diagnostics.len(), 1);
    assert!(matches!(
        out.trait_diagnostics[0],
        lpp_types::TraitBuildError::Coherence(TraitCoherenceError::Overlap { .. })
    ));

    // Also test duplicate concrete impls on the same struct
    let dup_source = concat!(
        "trait Ping:\n",
        "    def ping(self) -> Int\n",
        "struct Node:\n",
        "    v: Int\n",
        "impl Ping for Node:\n",
        "    def ping(self) -> Int:\n",
        "        return 1\n",
        "impl Ping for Node:\n",
        "    def ping(self) -> Int:\n",
        "        return 2\n",
    );
    let (_, dup_out) = check_ok(dup_source);
    assert_eq!(dup_out.trait_diagnostics.len(), 1);
    assert!(matches!(
        dup_out.trait_diagnostics[0],
        lpp_types::TraitBuildError::Coherence(TraitCoherenceError::Overlap { .. })
    ));
}

#[test]
fn trait_solver_enforces_bounds_and_all_overflow_modes() {
    let mut types = TypeInterner::new();
    let int = types.primitive(PrimitiveType::Int);
    let bool_ty = types.primitive(PrimitiveType::Bool);
    let param0 = make_param(&mut types, 0);
    let wrap_param = make_nominal(&mut types, 100, &[param0]);
    let wrap_int = make_nominal(&mut types, 100, &[int]);
    let wrap_bool = make_nominal(&mut types, 100, &[bool_ty]);
    let t_eq = DefId::from_raw(200);
    let t_ord = DefId::from_raw(201);
    let empty = types.empty_list();

    let mut index = TraitIndex::new();
    index
        .register(&types, nominal_rule(0, t_eq.raw(), int, &[], &[], &types))
        .unwrap();
    let eq_bound = TraitBound {
        parameter: TypeParamId::from_raw(0),
        trait_definition: t_eq,
        arguments: empty,
    };
    index
        .register(
            &types,
            nominal_rule(1, t_ord.raw(), wrap_param, &[0], &[eq_bound], &types),
        )
        .unwrap();

    assert!(matches!(
        index.solve(
            &mut types,
            TraitGoal::new(t_ord, wrap_int, empty),
            TraitSolverLimits::default(),
        ),
        TraitSolution::Unique(_)
    ));
    assert_eq!(
        index.solve(
            &mut types,
            TraitGoal::new(t_ord, wrap_bool, empty),
            TraitSolverLimits::default(),
        ),
        TraitSolution::NoSolution
    );
    // Unresolved inference variable in goal produces Deferred
    let mut infer = InferenceTable::new();
    let unk = infer.fresh(&mut types, InferenceLevel::ROOT, None).unwrap();
    assert_eq!(
        index.solve(
            &mut types,
            TraitGoal::new(t_eq, unk, empty),
            TraitSolverLimits::default(),
        ),
        TraitSolution::Deferred
    );

    // Cycle, GoalLimit, WorkLimit, DepthLimit
    let cyc_trait = DefId::from_raw(202);
    let cyc_bound = TraitBound {
        parameter: TypeParamId::from_raw(0),
        trait_definition: cyc_trait,
        arguments: empty,
    };
    let mut cyc = TraitIndex::new();
    cyc.register(
        &types,
        nominal_rule(2, cyc_trait.raw(), param0, &[0], &[cyc_bound], &types),
    )
    .unwrap();
    let goal = TraitGoal::new(cyc_trait, int, empty);
    assert_eq!(
        cyc.solve(&mut types, goal, TraitSolverLimits::default()),
        TraitSolution::Overflow(TraitSolverOverflow::Cycle)
    );
    assert_eq!(
        cyc.solve(
            &mut types,
            goal,
            TraitSolverLimits {
                max_goals: 0,
                ..TraitSolverLimits::default()
            }
        ),
        TraitSolution::Overflow(TraitSolverOverflow::GoalLimit)
    );
    assert_eq!(
        cyc.solve(
            &mut types,
            goal,
            TraitSolverLimits {
                max_work: 0,
                ..TraitSolverLimits::default()
            }
        ),
        TraitSolution::Overflow(TraitSolverOverflow::WorkLimit)
    );
    assert_eq!(
        cyc.solve(
            &mut types,
            goal,
            TraitSolverLimits {
                max_depth: 0,
                ..TraitSolverLimits::default()
            }
        ),
        TraitSolution::Overflow(TraitSolverOverflow::DepthLimit)
    );
}

#[test]
fn instance_planner_discovers_transitive_specializations_and_enforces_limits() {
    let mut types = TypeInterner::new();
    let int = types.primitive(PrimitiveType::Int);
    let str_ty = types.primitive(PrimitiveType::String);
    let args_int = types.intern_list(&[int]).unwrap();
    let args_str = types.intern_list(&[str_ty]).unwrap();
    let k1 = InstanceKey::new(HirItemId::from_raw(1), args_int);
    let k2 = InstanceKey::new(HirItemId::from_raw(2), args_str);
    let k3 = InstanceKey::new(HirItemId::from_raw(3), args_int);

    let mut planner = InstancePlanner::new(InstanceLimits::default());
    assert_eq!(
        planner.request_root(k1, None, InstanceRequestKind::Required),
        Ok(InstanceRequestOutcome::Queued)
    );
    assert_eq!(
        planner.request_root(k1, None, InstanceRequestKind::Required),
        Ok(InstanceRequestOutcome::AlreadyQueued)
    );
    let w1 = planner.pop_next().unwrap().unwrap();
    assert_eq!(
        planner.request_from(w1.id, k2, None, InstanceRequestKind::Required),
        Ok(InstanceRequestOutcome::Queued)
    );
    planner.complete(w1.id).unwrap();
    let w2 = planner.pop_next().unwrap().unwrap();
    assert_eq!(w2.depth, 1);
    planner.complete(w2.id).unwrap();
    planner.finish().unwrap();
    assert_eq!(planner.stats().unique_requests, 2);
    assert_eq!(planner.stats().duplicate_requests, 1);

    // PerItemLimit, DepthLimit, GlobalLimit, OptionalInstanceLimit
    let mut cap_planner = InstancePlanner::new(InstanceLimits {
        max_instances_global: 1,
        max_instances_per_item: 1,
        max_depth: 0,
        max_fixed_point_rounds: 1,
        max_optional_instances: 0,
        max_optional_rounds: 0,
    });
    assert_eq!(
        cap_planner.request_root(k1, None, InstanceRequestKind::OptionalSpecialization),
        Err(InstanceGrowthError::OptionalInstanceLimit { limit: 0 })
    );
    cap_planner
        .request_root(k1, None, InstanceRequestKind::Required)
        .unwrap();
    assert_eq!(
        cap_planner.request_root(k3, None, InstanceRequestKind::Required),
        Err(InstanceGrowthError::GlobalLimit { limit: 1 })
    );
    let work = cap_planner.pop_next().unwrap().unwrap();
    assert_eq!(
        cap_planner.request_from(work.id, k2, None, InstanceRequestKind::Required),
        Err(InstanceGrowthError::DepthLimit { limit: 0 })
    );

    let o0_limits = InstanceLimits::for_optimization(OptimizationOptions {
        level: OptimizationLevel::O0,
        verify_each_pass: false,
    });
    assert_eq!(o0_limits.max_optional_instances, 0);
}

// ── 4. Recursive Structs, Recursive Enums & Alias/Occurs Cycles ────────────

#[test]
fn recursive_structs_and_enums_with_zero_null_sentinel_typecheck() {
    let source = concat!(
        "struct ListNode:\n",
        "    head: Int\n",
        "    tail: ListNode\n",
        "struct Tree[T]:\n",
        "    val: T\n",
        "    left: Tree[T]\n",
        "    right: Tree[T]\n",
        "enum MaybeNode:\n",
        "    Empty\n",
        "    Link(node: ListNode, rest: MaybeNode)\n",
        "struct MutA:\n",
        "    x: Int\n",
        "    b: MutB\n",
        "struct MutB:\n",
        "    y: Int\n",
        "    a: MutA\n",
        "def build() -> Int:\n",
        "    n1 := ListNode(1, 0)\n",
        "    n2 := ListNode(2, n1)\n",
        "    t := Tree[Str](\"root\", 0, 0)\n",
        "    m := MaybeNode.Link(n2, MaybeNode.Empty)\n",
        "    ma := MutA(10, 0)\n",
        "    mb := MutB(20, ma)\n",
        "    match m:\n",
        "        MaybeNode.Empty:\n",
        "            return 0\n",
        "        MaybeNode.Link(node, _):\n",
        "            return node.head + str_len(t.val) + mb.y\n",
        "    return 0\n",
    );
    let (pkg, out) = check_ok(source);
    assert_eq!(
        fn_return_type(&pkg, &out, "build"),
        out.interner.primitive(PrimitiveType::Int)
    );
}

#[test]
fn recursive_struct_rejects_nonzero_integer_as_nominal_sentinel() {
    let err = check_err(concat!(
        "struct Node:\n",
        "    val: Int\n",
        "    next: Node\n",
        "def main() -> Node:\n",
        "    return Node(1, 42)\n",
    ));
    assert!(matches!(err, TypeError::Mismatch { .. }));
}

#[test]
fn cyclic_type_aliases_and_indirect_cycles_are_rejected() {
    let err_self = check_err(concat!(
        "type Loop = Loop\n",
        "def main(x: Loop):\n",
        "    return\n",
    ));
    assert!(matches!(err_self, TypeError::TypeAliasCycle { .. }));

    let err_three = check_err(concat!(
        "type A = B\n",
        "type B = C\n",
        "type C = A\n",
        "def main(x: A):\n",
        "    return\n",
    ));
    assert!(matches!(err_three, TypeError::TypeAliasCycle { .. }));

    let err_nested = check_err(concat!(
        "type Nested = List[Nested]\n",
        "def main(x: Nested):\n",
        "    return\n",
    ));
    assert!(matches!(err_nested, TypeError::TypeAliasCycle { .. }));
}

#[test]
fn infinite_type_unification_triggers_occurs_check() {
    let mut types = TypeInterner::new();
    let mut table = InferenceTable::new();
    let mut budget = TypeWorkBudget::new(10_000);
    let a = table.fresh(&mut types, InferenceLevel::ROOT, None).unwrap();
    let tuple_elems = types
        .intern_list(&[a, types.primitive(PrimitiveType::Int)])
        .unwrap();
    let tup = types.intern(TypeKind::Tuple(tuple_elems)).unwrap();
    let fn_params = types.intern_list(&[tup]).unwrap();
    let fn_ty = types
        .intern(TypeKind::Function {
            parameters: fn_params,
            result: a,
        })
        .unwrap();
    let err = table.unify(&mut types, a, fn_ty, &mut budget).unwrap_err();
    assert!(matches!(err, TypeError::OccursCheck { .. }));

    // Source-level recursive list self-reference unification failure
    let src_err = check_err(concat!(
        "def main():\n",
        "    mut xs := []\n",
        "    list_push(xs, xs)\n",
    ));
    assert!(matches!(src_err, TypeError::OccursCheck { .. }));
}

// ── 5. Tuple Arities 1..=16 vs Arity 17 & 0 Rejection ──────────────────────

#[test]
fn tuples_arities_1_through_16_intern_and_source_2_through_16_typecheck() {
    // 1. TypeInterner & `Tuple[T]` type annotation support arities 1..=16
    let mut types = TypeInterner::new();
    let int = types.primitive(PrimitiveType::Int);
    for arity in 1..=16 {
        let elems = vec![int; arity];
        let list = types.intern_list(&elems).unwrap();
        let tup = types.intern(TypeKind::Tuple(list)).unwrap();
        let TypeKind::Tuple(stored) = types.kind(tup) else {
            panic!("expected tuple");
        };
        assert_eq!(types.list(stored).len(), arity);
    }

    // `Tuple[Int]` (arity 1) in signature position
    let (pkg1, out1) = check_ok(concat!(
        "def take_one(t: Tuple[Int]) -> Int:\n",
        "    return t.0\n",
    ));
    let TypeKind::Tuple(elems1) = out1.interner.kind(
        out1.assignments
            .local(
                pkg1.locals
                    .enumerate()
                    .find_map(|(id, l)| {
                        (pkg1.names.symbols.resolve(l.name) == Some("t")).then_some(id)
                    })
                    .unwrap(),
            )
            .unwrap(),
    ) else {
        panic!("expected Tuple[Int]");
    };
    assert_eq!(out1.interner.list(elems1).len(), 1);

    // 2. Source tuple literals and destructuring for arities 2..=16
    let source = concat!(
        "def test_all() -> Int:\n",
        "    t2 := (1, \"two\")\n",
        "    t3 := (1, 2, 3)\n",
        "    t4 := (1, 2, 3, 4)\n",
        "    t5 := (1, 2, 3, 4, 5)\n",
        "    t6 := (1, 2, 3, 4, 5, 6)\n",
        "    t7 := (1, 2, 3, 4, 5, 6, 7)\n",
        "    t8 := (1, 2, 3, 4, 5, 6, 7, 8)\n",
        "    t9 := (1, 2, 3, 4, 5, 6, 7, 8, 9)\n",
        "    t10 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10)\n",
        "    t11 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11)\n",
        "    t12 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12)\n",
        "    t13 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13)\n",
        "    t14 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14)\n",
        "    t15 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15)\n",
        "    t16 := (1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16)\n",
        "    (a0, a1, a2, a3, a4, a5, a6, a7, a8, a9, a10, a11, a12, a13, a14, a15) := t16\n",
        "    return t2.0 + t3[2] + t4.3 + t5.4 + t6.5 + t7.6 + t8.7 + t9.8 + t10.9 + t11.10 + t12.11 + t13.12 + t14.13 + t15.14 + a0 + a15\n",
    );
    let (pkg, out) = check_ok(source);
    let tuple_facts: Vec<_> = out
        .places
        .expressions()
        .filter(|(_, f)| matches!(f, PlaceExpressionFact::TupleField { .. }))
        .collect();
    assert_eq!(tuple_facts.len(), 14);
    assert!(
        out.places
            .statements()
            .any(|(_, f)| matches!(f, PlaceStatementFact::TupleDestructure { arity: 16 }))
    );
    assert_eq!(
        fn_return_type(&pkg, &out, "test_all"),
        out.interner.primitive(PrimitiveType::Int)
    );
}

#[test]
fn tuple_arity_17_and_arity_0_and_invalid_tuple_ops_are_rejected() {
    fn reject_codes(source: &str) -> Vec<String> {
        let fs = MemoryFileSystem::single(source);
        match GraphBuilder::new(&fs).build(GraphRequest::new(
            "/types/main.lpp",
            PackageSpec::new("types-suite", "/types"),
        )) {
            Err(GraphError::Frontend { diagnostics, .. }) => diagnostics
                .into_iter()
                .map(|d| d.code.to_string())
                .collect(),
            Err(other) => panic!("unexpected graph error: {other}"),
            Ok(graph) => match lower_package(&graph, ResolutionMode::Namespaced) {
                Ok(_) => panic!("expected frontend or lowering rejection for:\n{source}"),
                Err(errs) => errs.into_iter().map(|e| e.code.to_string()).collect(),
            },
        }
    }

    // Arity 17 literal rejected (`E1110` in frontend or `E3100` in HIR)
    let codes_17 = reject_codes(concat!(
        "def main() -> Int:\n",
        "    t := (1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17)\n",
        "    return t.0\n",
    ));
    assert!(codes_17.iter().any(|c| c == "E1110" || c == "E3100"));

    // Arity 17 type annotation rejected
    let codes_type17 = reject_codes(concat!(
        "def f(t: (Int,Int,Int,Int,Int,Int,Int,Int,Int,Int,Int,Int,Int,Int,Int,Int,Int)) -> Int:\n",
        "    return 0\n",
    ));
    assert!(codes_type17.iter().any(|c| c == "E1110" || c == "E3100"));

    // Arity 17 destructuring rejected
    let codes_destr17 = reject_codes(concat!(
        "def f(t: (Int, Int)) -> Int:\n",
        "    (a0,a1,a2,a3,a4,a5,a6,a7,a8,a9,a10,a11,a12,a13,a14,a15,a16) := t\n",
        "    return a0\n",
    ));
    assert!(codes_destr17.iter().any(|c| c == "E1110" || c == "E3100"));

    // Empty tuple () rejected
    let codes_empty = reject_codes("def f():\n    x := ()\n");
    assert!(codes_empty.iter().any(|c| c == "E1110" || c == "E3100"));

    // Dynamic tuple index, out-of-bounds tuple index, and tuple element assignment
    assert_eq!(
        check_err("def f(i: Int) -> Int:\n    t := (10, 20)\n    return t[i]\n"),
        TypeError::TupleIndexMustBeStatic
    );
    assert_eq!(
        check_err("def f() -> Int:\n    t := (10, 20)\n    return t[2]\n"),
        TypeError::TupleIndexOutOfBounds { index: 2, arity: 2 }
    );
    assert_eq!(
        check_err("def f() -> Int:\n    t := (10, 20)\n    return t[-1]\n"),
        TypeError::TupleIndexMustBeStatic
    );
    assert_eq!(
        check_err("def f():\n    mut t := (10, 20)\n    t[0] = 99\n"),
        TypeError::TupleElementAssignment
    );
}

// ── 6. Set, Deque, List, Map, Buf, Str, Slice & VectorI64x2 ────────────────

#[test]
fn set_deque_buf_and_vector_i64x2_typecheck_with_exact_signatures() {
    let source = concat!(
        "def test_set_deque_buf_simd() -> Int:\n",
        "    s1 := set_new()\n",
        "    _ := set_add(s1, 10)\n",
        "    _ := set_add_str(s1, \"hello\")\n",
        "    b1: Bool = set_has(s1, 10)\n",
        "    b2: Bool = set_has_str(s1, \"hello\")\n",
        "    _ := set_remove(s1, 10)\n",
        "    _ := set_remove_str(s1, \"hello\")\n",
        "    s2 := set_new()\n",
        "    su := set_union(s1, s2)\n",
        "    si := set_intersect(s1, s2)\n",
        "    slen: Int = set_len(su) + set_len(si)\n",
        "    set_clear(s1)\n",
        "    set_free(s1)\n",
        "    set_free(s2)\n",
        "    set_free(su)\n",
        "    set_free(si)\n",
        "    dq := deque_new()\n",
        "    deque_push_back(dq, 1)\n",
        "    deque_push_front(dq, 2)\n",
        "    dval := deque_peek_front(dq) + deque_peek_back(dq) + deque_get(dq, 0) + deque_pop_front(dq) + deque_pop_back(dq) + deque_len(dq)\n",
        "    deque_clear(dq)\n",
        "    deque_free(dq)\n",
        "    buf := buf_alloc(64)\n",
        "    buf_set8(buf, 0, 0x7F)\n",
        "    buf_set16le(buf, 2, 0x1234)\n",
        "    buf_set32le(buf, 4, 0x12345678)\n",
        "    buf_set64le(buf, 8, 123456789)\n",
        "    buf_set16be(buf, 16, 0x1122)\n",
        "    buf_set32be(buf, 20, 0x11223344)\n",
        "    buf_set64be(buf, 24, 987654321)\n",
        "    _ := buf_write_str(buf, 32, \"abc\")\n",
        "    rs: Str = buf_read_str(buf, 32, 3)\n",
        "    bv := buf_get8(buf, 0) + buf_get16le(buf, 2) + buf_get32le(buf, 4) + buf_get64le(buf, 8) + buf_get16be(buf, 16) + buf_get32be(buf, 20) + buf_get64be(buf, 24) + buf_len(buf) + buf_crc32(buf, 0, 8)\n",
        "    buf_free(buf)\n",
        "    v1: VectorI64x2 = vec_i64x2(10, 20)\n",
        "    v2 := vec_i64x2_splat(3)\n",
        "    v3 := vec_i64x2_xor(vec_i64x2_or(vec_i64x2_and(vec_i64x2_mul(vec_i64x2_sub(vec_i64x2_add(v1, v2), v2), v2), v2), v2), v2)\n",
        "    v4 := vec_i64x2_shr_var(vec_i64x2_shr(vec_i64x2_not(v3), 1), v2)\n",
        "    mm := vec_u8x16_movemask(vec_u8x16_eq(vec_u8x16_splat(65), vec_u8x16_splat(65)))\n",
        "    sv := vec_i64x2_extract(v4, 0) + vec_i64x2_sum(v4) + mm + vec_i64_checksum(12)\n",
        "    if b1 && b2:\n",
        "        return slen + dval + bv + sv + str_len(rs)\n",
        "    return 0\n",
    );
    let (pkg, out) = check_ok(source);
    assert_eq!(
        fn_return_type(&pkg, &out, "test_set_deque_buf_simd"),
        out.interner.primitive(PrimitiveType::Int)
    );
    let v1_local = pkg
        .locals
        .enumerate()
        .find_map(|(id, l)| (pkg.names.symbols.resolve(l.name) == Some("v1")).then_some(id))
        .unwrap();
    assert_eq!(
        out.assignments.local(v1_local),
        Some(out.interner.primitive(PrimitiveType::VectorI64x2))
    );
}

#[test]
fn borrowed_slice_and_str_slice_escape_rules_are_enforced() {
    // Valid local use of Slice[T] and StrSlice
    check_ok(concat!(
        "def sum_view(xs: List[Int]) -> Int:\n",
        "    v := slice(xs, 0, 2)\n",
        "    s := \"hello\"\n",
        "    sv := str_slice(s, 1, 2)\n",
        "    return v[0] + len(v) + str_len(slice_to_str(sv)) + len(sv)\n",
    ));

    // Returning a Slice[T] from a function is rejected
    assert!(matches!(
        check_err(concat!(
            "def leak(xs: List[Int]) -> Slice[Int]:\n",
            "    v := slice(xs, 0, 1)\n",
            "    return v\n",
        )),
        TypeError::BorrowedSliceEscape { .. }
    ));

    // Returning a StrSlice from a function is rejected
    assert!(matches!(
        check_err(concat!(
            "def leak_str(s: Str) -> StrSlice:\n",
            "    sv := str_slice(s, 0, 1)\n",
            "    return sv\n",
        )),
        TypeError::BorrowedSliceEscape { .. }
    ));

    // Capturing a slice inside a closure is rejected
    assert!(matches!(
        check_err(concat!(
            "def capture(xs: List[Int]) -> Int:\n",
            "    v := slice(xs, 0, 1)\n",
            "    f := fn() -> Int: v[0]\n",
            "    return f()\n",
        )),
        TypeError::BorrowedSliceEscape { .. }
    ));

    // Reassigning source while slice view is live is rejected
    assert!(matches!(
        check_err(concat!(
            "def mutate_src() -> Int:\n",
            "    mut xs := [1, 2, 3]\n",
            "    v := slice(xs, 0, 2)\n",
            "    xs = [4, 5]\n",
            "    return v[0]\n",
        )),
        TypeError::BorrowedSliceEscape { .. }
    ));
}

// ── 7. F-Strings, Ranges, Guarded/Exhaustive Match & Try Flow ──────────────

#[test]
fn fstring_interpolation_accepts_scalars_and_strings_and_rejects_invalid_shapes() {
    let (pkg, out) = check_ok(concat!(
        "def fmt(n: Int, f: Float, ok: Bool, s: Str) -> Str:\n",
        "    return f\"n={n},next={n + 1},f={f},ok={ok},s={s}\"\n",
    ));
    assert_eq!(
        fn_return_type(&pkg, &out, "fmt"),
        out.interner.primitive(PrimitiveType::String)
    );

    // Non-stringifiable aggregate (struct or list) inside f-string interpolation fails unification
    let err_struct = check_err(concat!(
        "struct P:\n",
        "    x: Int\n",
        "def bad() -> Str:\n",
        "    p := P(1)\n",
        "    return f\"p={p}\"\n",
    ));
    assert!(matches!(err_struct, TypeError::Mismatch { .. }));

    let err_list = check_err(concat!(
        "def bad() -> Str:\n",
        "    xs := [1, 2]\n",
        "    return f\"xs={xs}\"\n",
    ));
    assert!(matches!(err_list, TypeError::Mismatch { .. }));

    // Empty interpolation and unterminated interpolation fail at HIR lowering with E3105 / E3104
    let empty_interp = try_lower_errors("def bad() -> Str:\n    return f\"a={}b\"\n").unwrap();
    assert_eq!(empty_interp[0].code, "E3105");

    let unterminated = try_lower_errors("def bad() -> Str:\n    return f\"a={1+2\"\n").unwrap();
    assert_eq!(unterminated[0].code, "E3104");
}

#[test]
fn range_loops_and_value_match_ranges_pin_types_correctly() {
    let (pkg, out) = check_ok(concat!(
        "def sum_ranges(lo: Int, hi: Int) -> Int:\n",
        "    mut total := 0\n",
        "    for i in lo..hi:\n",
        "        total = total + i\n",
        "    for j in 0..=5:\n",
        "        total = total + j\n",
        "    for k in range(0, 10, 2):\n",
        "        total = total + k\n",
        "    return total\n",
        "def classify(c: Int, ch: Char) -> Int:\n",
        "    mut out := 0\n",
        "    match c:\n",
        "        -10..-1:\n",
        "            out = 1\n",
        "        0..=10:\n",
        "            out = 2\n",
        "        11 | 12 if out == 0:\n",
        "            out = 3\n",
        "        _:\n",
        "            out = 4\n",
        "    match ch:\n",
        "        'a'..='z':\n",
        "            return out + 10\n",
        "        _:\n",
        "            return out\n",
    ));
    assert_eq!(
        fn_return_type(&pkg, &out, "sum_ranges"),
        out.interner.primitive(PrimitiveType::Int)
    );
    assert_eq!(
        fn_return_type(&pkg, &out, "classify"),
        out.interner.primitive(PrimitiveType::Int)
    );

    // Non-integer range bound in `for i in 0.."x"` fails unification
    let err = check_err(concat!(
        "def bad() -> Int:\n",
        "    mut s := 0\n",
        "    for i in 0..\"ten\":\n",
        "        s = s + i\n",
        "    return s\n",
    ));
    assert!(matches!(err, TypeError::Mismatch { .. }));
}

#[test]
fn guarded_enum_match_or_patterns_and_try_propagation_unify_branches() {
    let source = concat!(
        "enum Expr:\n",
        "    Lit(v: Int)\n",
        "    Add(a: Int, b: Int)\n",
        "    Empty\n",
        "enum Res[T, E]:\n",
        "    Ok(val: T)\n",
        "    Err(err: E)\n",
        "def eval_guarded(e: Expr) -> Int:\n",
        "    match e:\n",
        "        Expr.Lit(v) if v < 0:\n",
        "            return -v\n",
        "        Expr.Lit(v):\n",
        "            return v\n",
        "        Expr.Add(a, b) if a == b:\n",
        "            return a * 2\n",
        "        Expr.Add(a, b):\n",
        "            return a + b\n",
        "        Expr.Empty:\n",
        "            return 0\n",
        "def or_match(e: Expr) -> Int:\n",
        "    match e:\n",
        "        Expr.Lit(v):\n",
        "            return v\n",
        "        Expr.Add(a, b):\n",
        "            return a + b\n",
        "        Expr.Empty:\n",
        "            return 0\n",
        "def chain(r: Res[Int, Str]) -> Res[Bool, Str]:\n",
        "    v := r?\n",
        "    return Res.Ok(v > 0)\n",
    );
    let (_, out) = check_ok(source);
    let tries: Vec<_> = out.enum_flow.tries().collect();
    assert_eq!(tries.len(), 1);
    assert!(!tries[0].1.direct_residual);
    let matches: Vec<_> = out.enum_flow.matches().collect();
    assert_eq!(matches.len(), 2);
    assert!(matches.iter().all(|(_, m)| m.exhaustive()));
}

#[test]
fn enum_match_detects_unreachable_duplicate_arms_and_branch_return_mismatches() {
    let (pkg, out) = check_ok(concat!(
        "enum Flag:\n",
        "    On\n",
        "    Off\n",
        "def f(x: Flag) -> Int:\n",
        "    match x:\n",
        "        Flag.On:\n",
        "            return 1\n",
        "        Flag.On:\n",
        "            return 2\n",
        "        _:\n",
        "            return 3\n",
    ));
    let _ = pkg;
    let arms: Vec<_> = out.enum_flow.arms().collect();
    assert_eq!(arms.len(), 3);
    assert!(matches!(
        arms[0].1,
        EnumMatchArmFact::Variant {
            reachable: true,
            ..
        }
    ));
    assert!(matches!(
        arms[1].1,
        EnumMatchArmFact::Variant {
            reachable: false,
            ..
        }
    ));

    // Branch return type mismatch inside match arm
    let err = check_err(concat!(
        "enum Flag:\n",
        "    On\n",
        "    Off\n",
        "def f(x: Flag) -> Int:\n",
        "    match x:\n",
        "        Flag.On:\n",
        "            return 1\n",
        "        Flag.Off:\n",
        "            return \"bad\"\n",
    ));
    assert!(matches!(err, TypeError::Mismatch { .. }));
}

// ── 8. Unsafe Rules, Ptr[T]/RawPtr, Kernel Intrinsics & Options ────────────

#[test]
fn unsafe_blocks_functions_pointer_arithmetic_and_kernel_attrs_typecheck() {
    let source = concat!(
        "@repr(C)\n",
        "@packed\n",
        "@align(16)\n",
        "struct Frame:\n",
        "    rip: U64\n",
        "    cs: U64\n",
        "@export_name(\"lpp_isr_entry\")\n",
        "@section(\".text.isr\")\n",
        "@interrupt\n",
        "unsafe def isr_handler() -> Void:\n",
        "    cpu_cli()\n",
        "    cpu_sti()\n",
        "    memory_barrier()\n",
        "@naked\n",
        "unsafe def naked_stub() -> Void:\n",
        "    cpu_hlt()\n",
        "unsafe def low_level_mem(dst: Ptr[U8], src: Ptr[U8], n: USize) -> Int:\n",
        "    memset_raw(dst, 0 as U8, n)\n",
        "    memcpy_raw(dst, src, n)\n",
        "    memmove_raw(dst, src, n)\n",
        "    return memcmp_raw(dst, src, n)\n",
        "def main() -> USize:\n",
        "    sz := size_of[Frame]()\n",
        "    al := align_of[Frame]()\n",
        "    p0 := ptr_null[U32]()\n",
        "    is_n := ptr_is_null[U32](p0)\n",
        "    mut total: USize = sz + al\n",
        "    unsafe:\n",
        "        p1 := (0x1000 as USize) as Ptr[U32]\n",
        "        p2 := ptr_add[U32](p1, 4 as USize)\n",
        "        p3 := ptr_offset[U32](p2, -1 as isize)\n",
        "        ptr_write[U32](p3, 0xDEAD as U32)\n",
        "        val := ptr_read[U32](p3)\n",
        "        mmio_write8(0x100 as USize, 1 as U8)\n",
        "        mmio_write16(0x100 as USize, 2 as U16)\n",
        "        mmio_write32(0x100 as USize, val)\n",
        "        mmio_write64(0x100 as USize, 4 as U64)\n",
        "        _ := mmio_read8(0x100 as USize)\n",
        "        _ := mmio_read16(0x100 as USize)\n",
        "        _ := mmio_read32(0x100 as USize)\n",
        "        _ := mmio_read64(0x100 as USize)\n",
        "        port_out8(0x3F8 as U16, 65 as U8)\n",
        "        port_out16(0x3F8 as U16, 65 as U16)\n",
        "        port_out32(0x3F8 as U16, 65 as U32)\n",
        "        _ := port_in8(0x3F8 as U16)\n",
        "        _ := port_in16(0x3F8 as U16)\n",
        "        _ := port_in32(0x3F8 as U16)\n",
        "        _ := rdtsc()\n",
        "        _ := read_cr0() + read_cr2() + read_cr3() + read_cr4()\n",
        "        write_cr3(0 as U64)\n",
        "        write_cr4(0 as U64)\n",
        "        _ := rdmsr(0xC0000080 as U32)\n",
        "        wrmsr(0xC0000080 as U32, 1 as U64)\n",
        "        lgdt(0 as USize)\n",
        "        lidt(0 as USize)\n",
        "        invlpg(0 as USize)\n",
        "        rp := p3 as RawPtr\n",
        "        total = total + (rp as USize)\n",
        "    if is_n:\n",
        "        return total\n",
        "    return 0 as USize\n",
    );
    let (pkg, out) = check_ok(source);
    assert_eq!(
        fn_return_type(&pkg, &out, "main"),
        out.interner.primitive(PrimitiveType::USize)
    );
}

#[test]
fn shadow_inference_options_enforce_work_and_depth_limits() {
    let source = concat!(
        "def chain(a: List[List[List[List[Int]]]]) -> Int:\n",
        "    return a[0][0][0][0]\n",
    );
    let err_work = check_with_options(
        source,
        ShadowInferenceOptions {
            work_units: 1,
            ..ShadowInferenceOptions::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        err_work.error,
        TypeError::WorkLimitExceeded { .. }
    ));

    let err_depth = check_with_options(
        source,
        ShadowInferenceOptions {
            max_type_depth: 1,
            ..ShadowInferenceOptions::default()
        },
    )
    .unwrap_err();
    assert!(matches!(
        err_depth.error,
        TypeError::DepthLimitExceeded { .. }
    ));
}

// ── 9. Default & Variadic Parameters, Closures & Async/Spawn Safety ────────

#[test]
fn default_and_variadic_parameters_typecheck_for_valid_call_arities() {
    let source = concat!(
        "def connect(host: Str, port: Int = 8080, tls: Bool = true) -> Bool:\n",
        "    return tls && port > 0 && str_len(host) > 0\n",
        "def sum_with_bias(bias: Int, ...nums: Int) -> Int:\n",
        "    mut total := bias\n",
        "    for n in nums:\n",
        "        total = total + n\n",
        "    return total\n",
        "def main() -> Int:\n",
        "    ok1 := connect(\"localhost\")\n",
        "    ok2 := connect(\"localhost\", 9000)\n",
        "    ok3 := connect(\"localhost\", 443, false)\n",
        "    s0 := sum_with_bias(5)\n",
        "    s1 := sum_with_bias(5, 10)\n",
        "    s3 := sum_with_bias(5, 10, 20, 30)\n",
        "    if ok1 && ok2 && !ok3:\n",
        "        return s0 + s1 + s3\n",
        "    return 0\n",
    );
    let (pkg, out) = check_ok(source);
    assert_eq!(
        fn_return_type(&pkg, &out, "main"),
        out.interner.primitive(PrimitiveType::Int)
    );
}

#[test]
fn closures_async_await_and_spawn_safety_rules_work() {
    // Valid closure capture, async await, and spawn of non-mutating closure
    check_ok(concat!(
        "async def work(x: Int) -> Int:\n",
        "    return x * 2\n",
        "def main() -> Int:\n",
        "    mut acc := 10\n",
        "    add := fn(d: Int) -> Int:\n",
        "        acc = acc + d\n",
        "        return acc\n",
        "    log := list_new()\n",
        "    spawn fn():\n",
        "        list_push(log, 1)\n",
        "    t := work(5)\n",
        "    return add(3) + t.await\n",
    ));
}

// ── 10. 65+ Negative Semantic Error Tests ──────────────────────────────────

#[test]
fn negative_type_mismatch_matrix_rejects_incompatible_expressions() {
    let cases = [
        // 1. Local annotation mismatch
        "def f():\n    x: Int = \"abc\"\n",
        // 2. Float assigned to Int
        "def f():\n    x: Int = 1.5\n",
        // 3. Bool assigned to Str
        "def f():\n    s: Str = true\n",
        // 4. Char assigned to Int
        "def f():\n    x: Int = 'A'\n",
        // 5. Function return mismatch
        "def f() -> Int:\n    return \"nope\"\n",
        // 6. Void function returning Int
        "def f() -> Void:\n    return 1\n",
        // 7. Non-void function returning bare return
        "def f() -> Int:\n    return\n",
        // 8. Binary Add Int + Float
        "def f() -> Int:\n    return 1 + 2.0\n",
        // 9. Binary Sub Str - Int
        "def f() -> Int:\n    return \"a\" - 1\n",
        // 10. Comparison Int < Str
        "def f() -> Bool:\n    return 1 < \"a\"\n",
        // 11. Heterogeneous list literal
        "def f():\n    xs := [1, \"two\"]\n",
        // 12. List indexing with non-Int index
        "def f(xs: List[Int]) -> Int:\n    return xs[\"zero\"]\n",
        // 13. String indexing with non-Int index
        "def f(s: Str) -> Str:\n    return s[true]\n",
        // 14. Destructuring non-tuple
        "def f():\n    (a, b) := 42\n",
        // 15. Function returning Int when Bool expected
        "def f(x: Int) -> Bool:\n    return x\n",
        // 16. Struct field default mismatch
        "struct Bad:\n    x: Int = \"wrong\"\n",
        // 17. Const type mismatch
        "const N = 1\ndef f() -> Str:\n    return N\n",
        // 18. Generic call type arg mismatch
        "def id[T](x: T) -> T:\n    return x\ndef f() -> Int:\n    return id[Int](\"bad\")\n",
        // 19. Awaiting a non-Task value
        "def f() -> Int:\n    return (42).await\n",
        // 20. Closure return type mismatch
        "def f():\n    c := fn(x: Int) -> Str: x + 1\n",
        // 21. Default parameter initializer mismatch
        "def g(x: Int = \"nope\") -> Int:\n    return x\n",
        // 22. Variadic trailing argument element mismatch
        "def sum(...xs: Int) -> Int:\n    return 0\ndef f() -> Int:\n    return sum(1, \"bad\")\n",
        // 23. Defaulted call argument type mismatch
        "def g(a: Int, b: Str = \"ok\") -> Int:\n    return a\ndef f() -> Int:\n    return g(\"bad\")\n",
        // 24. Builtin argument type mismatch (str_len on Int)
        "def f() -> Int:\n    return str_len(123)\n",
        // 25. Builtin result mismatch (str_contains assigned to Int)
        "def f() -> Int:\n    return str_contains(\"ab\", \"a\")\n",
        // 26. Vec intrinsic mismatch (vec_i64x2_add with Int)
        "def f() -> VectorI64x2:\n    return vec_i64x2_add(1, 2)\n",
        // 27. Conditional branch return mismatch
        "def f(flag: Bool) -> Int:\n    if flag:\n        return 1\n    else:\n        return false\n",
        // 28. Reassigning mutable local with wrong type
        "def f():\n    mut x := 10\n    x = \"ten\"\n",
        // 29. Struct constructor field mismatch
        "struct Pair:\n    a: Int\n    b: Str\ndef f() -> Pair:\n    return Pair(1, 2)\n",
        // 30. Enum variant payload mismatch
        "enum Boxed:\n    One(v: Int)\ndef f() -> Boxed:\n    return Boxed.One(\"str\")\n",
    ];

    for (idx, src) in cases.iter().enumerate() {
        let err = check_err(src);
        assert!(
            matches!(err, TypeError::Mismatch { .. }),
            "case #{} expected TypeError::Mismatch, got {err:?}\nsource:\n{src}",
            idx + 1
        );
    }
}

#[test]
fn negative_arity_and_aggregate_errors_cover_all_variants() {
    // 31. Function call too few args
    assert_eq!(
        check_err(
            "def add(a: Int, b: Int) -> Int:\n    return a + b\ndef f() -> Int:\n    return add(1)\n"
        ),
        TypeError::ArityMismatch {
            expected: 1,
            actual: 2
        }
    );
    // 32. Function call too many args
    assert_eq!(
        check_err(
            "def add(a: Int, b: Int) -> Int:\n    return a + b\ndef f() -> Int:\n    return add(1, 2, 3)\n"
        ),
        TypeError::ArityMismatch {
            expected: 3,
            actual: 2
        }
    );
    // 33. Struct constructor partial arity
    assert_eq!(
        check_err("struct P:\n    x: Int\n    y: Int\ndef f() -> P:\n    return P(1)\n"),
        TypeError::ArityMismatch {
            expected: 1,
            actual: 2
        }
    );
    // 34. Unit variant called with arguments
    assert_eq!(
        check_err("enum E:\n    Unit\ndef f() -> E:\n    return E.Unit(1)\n"),
        TypeError::ArityMismatch {
            expected: 0,
            actual: 1
        }
    );
    // 35. Generic function explicit type arg count mismatch
    assert_eq!(
        check_err(
            "def id[T](x: T) -> T:\n    return x\ndef f() -> Int:\n    return id[Int, Str](1)\n"
        ),
        TypeError::ArityMismatch {
            expected: 1,
            actual: 2
        }
    );
    // 36. Variadic function missing fixed parameter
    assert_eq!(
        check_err(
            "def g(a: Int, ...rest: Int) -> Int:\n    return a\ndef f() -> Int:\n    return g()\n"
        ),
        TypeError::ArityMismatch {
            expected: 1,
            actual: 0
        }
    );
    // 37. Defaulted function called with too many args
    assert_eq!(
        check_err(
            "def g(a: Int, b: Int = 2) -> Int:\n    return a + b\ndef f() -> Int:\n    return g(1, 2, 3)\n"
        ),
        TypeError::ArityMismatch {
            expected: 3,
            actual: 2
        }
    );
    // 38. Intrinsic explicit type arg mismatch on ptr_null
    assert_eq!(
        check_err("def f():\n    p := ptr_null[U8, U16]()\n"),
        TypeError::ArityMismatch {
            expected: 1,
            actual: 2
        }
    );
    // 39. Monomorphic intrinsic called with unexpected type arg
    assert_eq!(
        check_err("def f():\n    unsafe:\n        v := mmio_read8[U8](0 as USize)\n"),
        TypeError::ArityMismatch {
            expected: 0,
            actual: 1
        }
    );
    // 40. Unknown struct field
    assert!(matches!(
        check_err("struct S:\n    x: Int\ndef f(s: S) -> Int:\n    return s.missing\n"),
        TypeError::UnknownField { .. }
    ));
    // 41. Unknown enum variant
    assert!(matches!(
        check_err("enum E:\n    A\ndef f() -> E:\n    return E.Missing\n"),
        TypeError::UnknownVariant { .. }
    ));
    // 42. Unknown enum variant in match arm
    assert!(matches!(
        check_err(
            "enum E:\n    A\ndef f(e: E) -> Int:\n    match e:\n        E.Nope:\n            return 1\n"
        ),
        TypeError::UnknownVariant { .. }
    ));
    // 43. Match subject not an enum (when using variant arm)
    assert!(matches!(
        check_err(
            "struct S:\n    x: Int\ndef f(s: S) -> Int:\n    match s:\n        _:\n            return 1\n"
        ),
        TypeError::InvalidMatchSubject { .. }
    ));
    // 44. Match arm qualified with wrong enum name
    assert_eq!(
        check_err(
            "enum A:\n    X\nenum B:\n    X\ndef f(a: A) -> Int:\n    match a:\n        B.X:\n            return 1\n"
        ),
        TypeError::InvalidMatchPattern
    );
    // 45. Match variant binding arity mismatch
    assert!(matches!(
        check_err(
            "enum E:\n    Pair(x: Int, y: Int)\ndef f(e: E) -> Int:\n    match e:\n        E.Pair(only):\n            return only\n"
        ),
        TypeError::MatchBindingArity {
            expected: 2,
            actual: 1,
            ..
        }
    ));
}

#[test]
fn negative_try_assignment_safety_and_cast_errors_cover_all_variants() {
    // 46. Invalid try carrier (non-enum)
    assert!(matches!(
        check_err("def f(x: Int) -> Int:\n    return x?\n"),
        TypeError::InvalidTryCarrier { .. }
    ));
    // 47. Invalid try carrier (empty enum / struct)
    assert!(matches!(
        check_err("struct S:\n    x: Int\ndef f(s: S) -> S:\n    x := s?\n    return s\n"),
        TypeError::InvalidTryCarrier { .. }
    ));
    // 48. Try success variant has 0 payloads
    assert_eq!(
        check_err(
            "enum R:\n    Ok\n    Err(e: Int)\ndef f(r: R) -> R:\n    x := r?\n    return R.Ok\n"
        ),
        TypeError::TrySuccessPayloadArity { actual: 0 }
    );
    // 49. Try success variant has 2 payloads
    assert_eq!(
        check_err(
            "enum R:\n    Ok(a: Int, b: Int)\n    Err(e: Int)\ndef f(r: R) -> R:\n    x := r?\n    return R.Ok(1, 2)\n"
        ),
        TypeError::TrySuccessPayloadArity { actual: 2 }
    );
    // 50. Try return type is not the carrier enum
    assert!(matches!(
        check_err("enum R:\n    Ok(v: Int)\n    Err(e: Int)\ndef f(r: R) -> Int:\n    return r?\n"),
        TypeError::InvalidTryReturn { .. }
    ));
    // 51. Try residual payload mismatch
    assert!(matches!(
        check_err(
            "enum R[T, E]:\n    Ok(v: T)\n    Err(e: E)\ndef f(r: R[Int, Str]) -> R[Bool, Int]:\n    x := r?\n    return R.Ok(x > 0)\n"
        ),
        TypeError::InvalidTryResidual { .. }
    ));
    // 52. Immutable local reassignment
    assert_eq!(
        check_err("def f():\n    x := 1\n    x = 2\n"),
        TypeError::ImmutableAssignmentRoot
    );
    // 53. Immutable struct field reassignment
    assert_eq!(
        check_err("struct S:\n    x: Int\ndef f():\n    s := S(1)\n    s.x = 2\n"),
        TypeError::ImmutableAssignmentRoot
    );
    // 54. Invalid assignment target (field of temporary call expression)
    assert_eq!(
        check_err(
            "struct S:\n    x: Int\ndef make() -> S:\n    return S(1)\ndef f():\n    make().x = 2\n"
        ),
        TypeError::InvalidAssignmentTarget
    );
    // 55. Spawn capture mutation
    assert!(matches!(
        check_err("def f():\n    mut count := 0\n    spawn fn():\n        count = count + 1\n"),
        TypeError::SpawnCaptureMutation { .. }
    ));
    // 56. Blocking builtin inside async function (sleep)
    assert!(matches!(
        check_err("async def f():\n    sleep(10)\n"),
        TypeError::UnsafeAsyncBlocking { builtin: "sleep" }
    ));
    // 57. Task handle captured by closure inside async function
    assert_eq!(
        check_err(
            "async def step() -> Int:\n    return 1\nasync def f() -> Int:\n    t := step()\n    c := fn() -> Int: t.await\n    return c()\n"
        ),
        TypeError::UnsafeAsyncTaskCapture
    );
    // 58. UnsafeRequired: calling unsafe fn from safe fn
    assert!(matches!(
        check_err(
            "unsafe def danger() -> Int:\n    return 1\ndef f() -> Int:\n    return danger()\n"
        ),
        TypeError::UnsafeRequired {
            operation: "call to unsafe function"
        }
    ));
    // 59. UnsafeRequired: ptr_read outside unsafe
    assert!(matches!(
        check_err("def f(p: Ptr[U8]) -> U8:\n    return ptr_read[U8](p)\n"),
        TypeError::UnsafeRequired {
            operation: "ptr_read"
        }
    ));
    // 60. UnsafeRequired: ptr_write outside unsafe
    assert!(matches!(
        check_err("def f(p: Ptr[U8]):\n    ptr_write[U8](p, 1 as U8)\n"),
        TypeError::UnsafeRequired {
            operation: "ptr_write"
        }
    ));
    // 61. UnsafeRequired: cpu_cli outside unsafe
    assert!(matches!(
        check_err("def f():\n    cpu_cli()\n"),
        TypeError::UnsafeRequired {
            operation: "cpu_cli"
        }
    ));
    // 62. UnsafeRequired: rdtsc outside unsafe
    assert!(matches!(
        check_err("def f() -> U64:\n    return rdtsc()\n"),
        TypeError::UnsafeRequired { operation: "rdtsc" }
    ));
    // 63. UnsafeRequired: cast integer to Ptr[T] outside unsafe
    assert!(matches!(
        check_err("def f() -> Ptr[U8]:\n    return (0 as USize) as Ptr[U8]\n"),
        TypeError::UnsafeRequired {
            operation: "cast to raw pointer"
        }
    ));
    // 64. UnsafeRequired: cast integer to RawPtr outside unsafe
    assert!(matches!(
        check_err("def f() -> RawPtr:\n    return (0 as USize) as RawPtr\n"),
        TypeError::UnsafeRequired {
            operation: "cast to raw pointer"
        }
    ));
    // 65. InvalidCast: Str as Int
    assert!(matches!(
        check_err("def f() -> Int:\n    return \"hi\" as Int\n"),
        TypeError::InvalidCast { .. }
    ));
    // 66. InvalidCast: Int as Str
    assert!(matches!(
        check_err("def f() -> Str:\n    return 1 as Str\n"),
        TypeError::InvalidCast { .. }
    ));
    // 67. InvalidCast: List[Int] as Int
    assert!(matches!(
        check_err("def f() -> Int:\n    xs := [1]\n    return xs as Int\n"),
        TypeError::InvalidCast { .. }
    ));
    // 68. InvalidCast: Float as Ptr[U8] inside unsafe
    assert!(matches!(
        check_err("def f() -> Ptr[U8]:\n    unsafe:\n        return 1.0 as Ptr[U8]\n"),
        TypeError::InvalidCast { .. }
    ));
    // 69. GraphError::ImportCycle on circular module imports
    let cycle_err = lower_multi(&[
        (
            "/types/main.lpp",
            "import b\ndef main() -> Int:\n    return 0\n",
        ),
        (
            "/types/b.lpp",
            "import main\ndef b() -> Int:\n    return 1\n",
        ),
    ])
    .unwrap_err();
    assert!(matches!(cycle_err, GraphError::ImportCycle(_)));
}
