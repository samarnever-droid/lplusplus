use super::*;

mod collections;
mod float;
mod io;
mod strings_basic;
mod strings_convert;

pub(super) struct HelperImports<'a> {
    pub(super) fd_helper_idx: u32,
    pub(super) fd_write_import_idx: u32,
    pub(super) fd_read_import_idx: u32,
    pub(super) proc_exit_import_idx: u32,
    pub(super) import_index: &'a BTreeMap<&'a str, u32>,
}

pub(super) fn emit_helper_body(
    fb: &mut FB,
    h: H,
    env: &Env<'_>,
    imports: &HelperImports<'_>,
) -> Result<(), CodegenError> {
    let HelperImports {
        fd_helper_idx,
        fd_write_import_idx,
        fd_read_import_idx,
        proc_exit_import_idx,
        ..
    } = *imports;
    match h {
        H::FdWrite
        | H::FdRead
        | H::Input
        | H::PrintInt
        | H::PrintBool
        | H::PrintStr
        | H::PrintFloat
        | H::WriteStr
        | H::FileExists
        | H::ReadFile
        | H::WriteFile
        | H::AppendFile
        | H::DeleteFile
        | H::StrEq
        | H::StrLen
        | H::Log2
        | H::Exp2
        | H::Pow
        | H::Sin
        | H::Cos
        | H::Alloc
        | H::ArcAlloc
        | H::Retain
        | H::Release => io::emit(fb, h, env, imports)?,
        H::ListNew
        | H::ListPushI64
        | H::ListPushF64
        | H::ListPushPtr
        | H::ListLen
        | H::ListGetI64
        | H::ListGetF64
        | H::ListGetPtr
        | H::ListSetI64
        | H::ListSetF64
        | H::ListSetPtr
        | H::SliceNewList
        | H::SliceNewStr
        | H::SliceLen
        | H::SliceGetI64
        | H::SliceGetF64
        | H::SliceGetPtr
        | H::StrSliceGet
        | H::StrSliceToStr
        | H::MapNew
        | H::MapEnsure
        | H::MapFindI64
        | H::MapFindStr
        | H::MapPutI64
        | H::MapPutStr
        | H::MapGetI64
        | H::MapGetStr
        | H::MapHasI64
        | H::MapHasStr
        | H::MapRemoveI64
        | H::MapRemoveStr
        | H::MapLen
        | H::TaskNew
        | H::TaskPoll
        | H::TaskAwait
        | H::TaskDestroy => collections::emit(
            fb,
            h,
            env,
            fd_helper_idx,
            fd_write_import_idx,
            fd_read_import_idx,
            proc_exit_import_idx,
        )?,
        H::StrConcat
        | H::StrSubstr
        | H::StrRepeat
        | H::StrSplit
        | H::CharAt
        | H::Ord
        | H::Chr
        | H::StrContains
        | H::StrStartsWith
        | H::StrEndsWith
        | H::StrFind => strings_basic::emit(
            fb,
            h,
            env,
            fd_helper_idx,
            fd_write_import_idx,
            fd_read_import_idx,
            proc_exit_import_idx,
        )?,
        H::StrReplace
        | H::StrTrim
        | H::StrLower
        | H::StrUpper
        | H::IntToStr
        | H::U64ToStr
        | H::U64ToHex
        | H::BoolToStr
        | H::StrToInt
        | H::StrToU64 => strings_convert::emit(
            fb,
            h,
            env,
            fd_helper_idx,
            fd_write_import_idx,
            fd_read_import_idx,
            proc_exit_import_idx,
        )?,
        H::FloatToStr => float::emit(
            fb,
            h,
            env,
            fd_helper_idx,
            fd_write_import_idx,
            fd_read_import_idx,
            proc_exit_import_idx,
        )?,
    }
    Ok(())
}
