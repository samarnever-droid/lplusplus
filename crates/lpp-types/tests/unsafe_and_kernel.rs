use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lpp_hir::{
    FileSystem, FileSystemError, GraphBuilder, GraphRequest, PackageSpec, ResolutionMode,
    lower_package,
};
use lpp_types::{PrimitiveType, ShadowInferenceOptions, TypeError, TypeKind, infer_hir_package};

#[derive(Debug)]
struct MemoryFileSystem {
    files: BTreeMap<PathBuf, String>,
}

impl MemoryFileSystem {
    fn new(source: &str) -> Self {
        Self {
            files: BTreeMap::from([(PathBuf::from("/kernel/main.lpp"), source.to_owned())]),
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
            .ok_or_else(|| FileSystemError::new("read", path, "file not found"))
    }
}

fn check(source: &str) -> Result<lpp_types::ShadowTypeOutput, lpp_types::ShadowTypeError> {
    let filesystem = MemoryFileSystem::new(source);
    let graph = GraphBuilder::new(&filesystem)
        .build(GraphRequest::new(
            "/kernel/main.lpp",
            PackageSpec::new("kernel", "/kernel"),
        ))
        .unwrap();
    let package = lower_package(&graph, ResolutionMode::Namespaced).unwrap();
    infer_hir_package(&package, ShadowInferenceOptions::default())
}

#[test]
fn resolves_kernel_primitives_ptr_and_unsafe_casts() {
    let mut output = check(concat!(
        "@repr(C)\n",
        "@packed\n",
        "@align(8)\n",
        "struct IdtEntry:\n",
        "    offset_low: U16\n",
        "    selector: U16\n",
        "\n",
        "unsafe def write_uart(port: U16, val: U8) -> Void:\n",
        "    port_out8(port, val)\n",
        "\n",
        "def main() -> USize:\n",
        "    sz := size_of[IdtEntry]()\n",
        "    al := align_of[IdtEntry]()\n",
        "    null_p := ptr_null[U8]()\n",
        "    is_null := ptr_is_null[U8](null_p)\n",
        "    mut addr: U64 = 0\n",
        "    unsafe:\n",
        "        p := (0xB8000 as USize) as Ptr[U16]\n",
        "        ptr_write[U16](p, 0x0F41 as U16)\n",
        "        val := mmio_read16(0xB8000 as USize)\n",
        "        addr = (p as U64) + (val as U64)\n",
        "        write_uart(0x3F8 as U16, 65 as U8)\n",
        "        cpu_cli()\n",
        "    if is_null:\n",
        "        return (sz + al) + (addr as USize)\n",
        "    return sz\n",
    ))
    .expect("valid unsafe kernel code should type-check cleanly");

    let usize_ty = output.interner.primitive(PrimitiveType::USize);
    let u8_ty = output.interner.primitive(PrimitiveType::U8);
    let ptr_u8 = output
        .interner
        .intern(TypeKind::Ptr(u8_ty))
        .expect("Ptr[U8] should be interned");
    assert_ne!(ptr_u8, usize_ty);
}

#[test]
fn rejects_unsafe_calls_intrinsics_and_pointer_casts_in_safe_context() {
    let err_call = check(concat!(
        "unsafe def danger() -> Int:\n",
        "    return 1\n",
        "def main() -> Int:\n",
        "    return danger()\n",
    ))
    .unwrap_err();
    assert!(matches!(err_call.error, TypeError::UnsafeRequired { .. }));

    let err_mmio = check(concat!(
        "def main() -> U32:\n",
        "    return mmio_read32(0 as USize)\n",
    ))
    .unwrap_err();
    assert!(matches!(err_mmio.error, TypeError::UnsafeRequired { .. }));

    let err_port = check(concat!(
        "def main():\n",
        "    port_out8(0x3F8 as U16, 65 as U8)\n",
    ))
    .unwrap_err();
    assert!(matches!(err_port.error, TypeError::UnsafeRequired { .. }));

    let err_ptr_cast = check(concat!(
        "def main() -> Ptr[U8]:\n",
        "    return (4096 as USize) as Ptr[U8]\n",
    ))
    .unwrap_err();
    assert!(matches!(
        err_ptr_cast.error,
        TypeError::UnsafeRequired { .. }
    ));
}

#[test]
fn rejects_invalid_casts_between_incompatible_types() {
    let err_str_to_int = check(concat!(
        "def main() -> Int:\n",
        "    return \"hello\" as Int\n",
    ))
    .unwrap_err();
    assert!(matches!(
        err_str_to_int.error,
        TypeError::InvalidCast { .. }
    ));

    let err_struct_cast = check(concat!(
        "struct Pair:\n",
        "    a: Int\n",
        "def main() -> Int:\n",
        "    p := Pair(1)\n",
        "    return p as Int\n",
    ))
    .unwrap_err();
    assert!(matches!(
        err_struct_cast.error,
        TypeError::InvalidCast { .. }
    ));
}
