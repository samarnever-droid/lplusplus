use super::*;

pub(super) const PE_IMAGE_BASE: u64 = 0x140000000;
pub(super) const PE_SECTION_RVA: u32 = 0x1000;
pub(super) const PE_FILE_ALIGN: usize = 0x200;
pub(super) const PE_SECT_ALIGN: usize = 0x1000;

pub(super) const AMD64_ADDR64: u16 = 1;
pub(super) const AMD64_ADDR32: u16 = 2;
pub(super) const AMD64_ADDR32NB: u16 = 3;
pub(super) const AMD64_REL32: u16 = 4;
pub(super) const AMD64_REL32_1: u16 = 5;
pub(super) const AMD64_REL32_2: u16 = 6;
pub(super) const AMD64_REL32_3: u16 = 7;
pub(super) const AMD64_REL32_4: u16 = 8;
pub(super) const AMD64_REL32_5: u16 = 9;
pub(super) const AMD64_SECTION: u16 = 10;
pub(super) const AMD64_SECREL: u16 = 11;

pub(super) const ARM64_ADDR32: u16 = 1;
pub(super) const ARM64_ADDR32NB: u16 = 2;
pub(super) const ARM64_BRANCH26: u16 = 3;
pub(super) const ARM64_PAGEBASE_REL21: u16 = 4;
pub(super) const ARM64_REL21: u16 = 5;
pub(super) const ARM64_PAGEOFFSET_12A: u16 = 6;
pub(super) const ARM64_PAGEOFFSET_12L: u16 = 7;
pub(super) const ARM64_SECREL: u16 = 8;
pub(super) const ARM64_SECTION: u16 = 13;
pub(super) const ARM64_ADDR64: u16 = 14;
pub(super) const ARM64_BRANCH19: u16 = 15;
pub(super) const ARM64_BRANCH14: u16 = 16;
pub(super) const ARM64_REL32: u16 = 17;

pub(super) fn pe_align(v: usize, a: usize) -> usize {
    align_up(v, a)
}

pub(super) fn classify_dll(name: &str, extra: &HashMap<String, String>) -> Option<String> {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    if let Some(d) = extra.get(clean).or_else(|| extra.get(name)) {
        return Some(d.clone());
    }
    if is_user32_symbol(clean) {
        return Some("USER32.dll".into());
    }
    if is_gdi32_symbol(clean) {
        return Some("GDI32.dll".into());
    }
    if is_ws2_32_symbol(clean) {
        return Some("WS2_32.dll".into());
    }
    if is_advapi32_symbol(clean) {
        return Some("ADVAPI32.dll".into());
    }
    if is_shell32_symbol(clean) {
        return Some("SHELL32.dll".into());
    }
    if is_ole32_symbol(clean) {
        return Some("OLE32.dll".into());
    }
    if is_oleaut32_symbol(clean) {
        return Some("OLEAUT32.dll".into());
    }
    if is_ntdll_symbol(clean) {
        return Some("ntdll.dll".into());
    }
    if is_bcrypt_symbol(clean) {
        return Some("BCRYPT.dll".into());
    }
    if is_crt_symbol(clean) {
        return Some("msvcrt.dll".into());
    }
    if is_kernel32_symbol(clean) {
        return Some("KERNEL32.dll".into());
    }
    if clean.starts_with("lpp_") {
        return Some("lpp_runtime.dll".into());
    }
    None
}

pub(super) fn is_crt_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "malloc"
            | "free"
            | "realloc"
            | "calloc"
            | "printf"
            | "puts"
            | "memset"
            | "memcpy"
            | "memmove"
            | "memcmp"
            | "strlen"
            | "strcmp"
            | "strncmp"
            | "strcpy"
            | "strncpy"
            | "strcat"
            | "strchr"
            | "strstr"
            | "sprintf"
            | "snprintf"
            | "sscanf"
            | "exit"
            | "abort"
            | "sin"
            | "cos"
            | "tan"
            | "pow"
            | "sqrt"
            | "ceil"
            | "floor"
            | "fmod"
            | "fabs"
            | "abs"
            | "labs"
            | "llabs"
            | "getpid"
            | "_getpid"
            | "atan2"
            | "log"
            | "exp"
            | "getchar"
            | "putchar"
            | "fopen"
            | "fclose"
            | "fread"
            | "fwrite"
            | "fflush"
            | "fprintf"
            | "fseek"
            | "ftell"
            | "getenv"
            | "system"
            | "time"
            | "clock"
            | "_errno"
            | "__getmainargs"
            | "__set_app_type"
            | "_acmdln"
            | "_initterm"
            | "_initterm_e"
            | "_configthreadlocale"
            | "lpp_c_malloc"
            | "lpp_c_free"
            | "lpp_c_load_u8"
            | "lpp_c_store_u8"
            | "lpp_c_load_i32"
            | "lpp_c_store_i32"
            | "lpp_c_load_i64"
            | "lpp_c_store_i64"
            | "dlopen"
            | "dlsym"
            | "dlclose"
            | "dlerror"
            | "vsnprintf"
            | "vfprintf"
            | "atoi"
            | "atol"
            | "strtol"
            | "strtoul"
            | "strtod"
            | "qsort"
            | "bsearch"
            | "rand"
            | "srand"
            | "tolower"
            | "toupper"
            | "isdigit"
            | "isalpha"
            | "isspace"
            | "_beginthreadex"
            | "_endthreadex"
            | "__iob_func"
            | "__acrt_iob_func"
            | "_fdopen"
            | "_fileno"
            | "rewind"
            | "fgets"
            | "fputs"
            | "ungetc"
            | "setvbuf"
            | "perror"
            | "strerror"
            | "memchr"
            | "strdup"
            | "_strdup"
            | "strncpy_s"
            | "strcpy_s"
    )
}

pub(super) fn is_kernel32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "ExitProcess"
            | "GetTickCount64"
            | "LoadLibraryA"
            | "LoadLibraryW"
            | "FreeLibrary"
            | "GetProcAddress"
            | "GetStdHandle"
            | "WriteFile"
            | "ReadFile"
            | "VirtualAlloc"
            | "VirtualFree"
            | "VirtualProtect"
            | "CreateThread"
            | "WaitForSingleObject"
            | "WaitForMultipleObjects"
            | "CloseHandle"
            | "CreateFileA"
            | "CreateFileW"
            | "GetFileSize"
            | "GetFileSizeEx"
            | "SetFilePointer"
            | "SetFilePointerEx"
            | "SetEndOfFile"
            | "DeleteFileA"
            | "MoveFileA"
            | "GetFileAttributesA"
            | "CreateDirectoryA"
            | "RemoveDirectoryA"
            | "FindFirstFileA"
            | "FindNextFileA"
            | "FindClose"
            | "Sleep"
            | "CreateProcessA"
            | "GetExitCodeProcess"
            | "GetExitCodeThread"
            | "SwitchToThread"
            | "ResumeThread"
            | "SuspendThread"
            | "TerminateThread"
            | "CreatePipe"
            | "GetEnvironmentVariableA"
            | "SetEnvironmentVariableA"
            | "GetModuleFileNameA"
            | "GetModuleHandleA"
            | "GetModuleHandleW"
            | "GetLastError"
            | "SetLastError"
            | "QueryPerformanceCounter"
            | "QueryPerformanceFrequency"
            | "GetCommandLineA"
            | "GetCommandLineW"
            | "GetProcessHeap"
            | "HeapAlloc"
            | "HeapFree"
            | "HeapReAlloc"
            | "FormatMessageA"
            | "GetConsoleMode"
            | "SetConsoleMode"
            | "FlushFileBuffers"
            | "GetSystemTimeAsFileTime"
            | "InitializeCriticalSection"
            | "InitializeCriticalSectionAndSpinCount"
            | "EnterCriticalSection"
            | "TryEnterCriticalSection"
            | "LeaveCriticalSection"
            | "DeleteCriticalSection"
            | "InitializeSRWLock"
            | "AcquireSRWLockShared"
            | "AcquireSRWLockExclusive"
            | "ReleaseSRWLockShared"
            | "ReleaseSRWLockExclusive"
            | "SetThreadAffinityMask"
            | "GetCurrentThread"
            | "TlsAlloc"
            | "TlsGetValue"
            | "TlsSetValue"
            | "TlsFree"
            | "GetCurrentThreadId"
            | "GetCurrentProcessId"
            | "GetCurrentProcess"
            | "TerminateProcess"
            | "IsDebuggerPresent"
            | "SetUnhandledExceptionFilter"
            | "AddVectoredExceptionHandler"
            | "RaiseException"
            | "RtlCaptureContext"
            | "RtlLookupFunctionEntry"
            | "RtlVirtualUnwind"
            | "MultiByteToWideChar"
            | "WideCharToMultiByte"
            | "GetACP"
            | "GetConsoleOutputCP"
            | "WriteConsoleA"
            | "ReadConsoleA"
            | "GetFileType"
            | "SetHandleInformation"
            | "DuplicateHandle"
            | "CreateEventA"
            | "SetEvent"
            | "ResetEvent"
            | "CreateMutexA"
            | "ReleaseMutex"
            | "GetSystemInfo"
            | "GetNativeSystemInfo"
            | "GlobalMemoryStatusEx"
            | "GetDiskFreeSpaceExA"
            | "OutputDebugStringA"
            | "DebugBreak"
            | "InitializeSListHead"
            | "EncodePointer"
            | "DecodePointer"
            | "GetStartupInfoW"
            | "GetStartupInfoA"
            | "SetDefaultDllDirectories"
            | "GetTempPathA"
            | "GetFullPathNameA"
            | "GetEnvironmentVariableW"
            | "CreateDirectoryW"
    ) || clean.starts_with("K32")
}

pub(super) fn is_user32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "CreateWindowExA"
            | "CreateWindowExW"
            | "DestroyWindow"
            | "DefWindowProcA"
            | "DefWindowProcW"
            | "PostQuitMessage"
            | "RegisterClassA"
            | "RegisterClassExA"
            | "RegisterClassW"
            | "RegisterClassExW"
            | "GetDC"
            | "ReleaseDC"
            | "LoadCursorA"
            | "LoadCursorW"
            | "PeekMessageA"
            | "PeekMessageW"
            | "GetMessageA"
            | "GetMessageW"
            | "TranslateMessage"
            | "DispatchMessageA"
            | "DispatchMessageW"
            | "GetAsyncKeyState"
            | "GetKeyState"
            | "GetCursorPos"
            | "ScreenToClient"
            | "ClientToScreen"
            | "FillRect"
            | "ShowWindow"
            | "UpdateWindow"
            | "SetForegroundWindow"
            | "MessageBoxA"
            | "MessageBoxW"
            | "LoadIconA"
            | "LoadIconW"
            | "SetWindowPos"
            | "BringWindowToTop"
            | "BeginPaint"
            | "EndPaint"
            | "SetProcessDPIAware"
            | "AdjustWindowRectEx"
            | "GetClientRect"
            | "GetWindowRect"
            | "InvalidateRect"
            | "SetWindowTextA"
            | "GetWindowTextA"
            | "ShowCursor"
            | "SetCursor"
            | "LoadImageA"
            | "SendMessageA"
            | "PostMessageA"
            | "KillTimer"
            | "SetTimer"
            | "GetSystemMetrics"
            | "MonitorFromWindow"
            | "GetMonitorInfoA"
            | "EnumDisplaySettingsA"
            | "ChangeDisplaySettingsA"
            | "ReleaseCapture"
            | "SetCapture"
            | "TrackMouseEvent"
            | "wsprintfA"
            | "wsprintfW"
            | "wvsprintfA"
            | "wvsprintfW"
            | "MsgWaitForMultipleObjects"
    )
}

pub(super) fn is_gdi32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "CreateCompatibleDC"
            | "CreateCompatibleBitmap"
            | "SelectObject"
            | "DeleteDC"
            | "DeleteObject"
            | "CreateSolidBrush"
            | "CreatePen"
            | "RoundRect"
            | "TextOutA"
            | "SetBkMode"
            | "SetTextColor"
            | "BitBlt"
            | "StretchBlt"
            | "Ellipse"
            | "MoveToEx"
            | "LineTo"
            | "GetTextExtentPoint32A"
            | "CreateFontA"
            | "SetStretchBltMode"
            | "SetBrushOrgEx"
            | "SetMapMode"
            | "SetGraphicsMode"
            | "SetTextCharacterExtra"
            | "SetTextAlign"
            | "SetLayout"
            | "GetStockObject"
            | "Rectangle"
            | "Polygon"
            | "Polyline"
            | "CreateDIBSection"
            | "GetDIBits"
            | "SetDIBits"
            | "ChoosePixelFormat"
            | "SetPixelFormat"
            | "SwapBuffers"
            | "DescribePixelFormat"
    )
}

pub(super) fn is_ws2_32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "WSAStartup"
            | "WSACleanup"
            | "WSAGetLastError"
            | "WSAIoctl"
            | "WSASocketA"
            | "WSARecv"
            | "WSASend"
            | "socket"
            | "bind"
            | "listen"
            | "accept"
            | "connect"
            | "send"
            | "recv"
            | "sendto"
            | "recvfrom"
            | "closesocket"
            | "shutdown"
            | "select"
            | "htons"
            | "htonl"
            | "ntohs"
            | "ntohl"
            | "getaddrinfo"
            | "freeaddrinfo"
            | "gethostname"
            | "getsockname"
            | "getpeername"
            | "setsockopt"
            | "getsockopt"
            | "ioctlsocket"
            | "inet_ntoa"
            | "inet_addr"
            | "WSAPoll"
            | "WSADuplicateSocketA"
    )
}

pub(super) fn is_advapi32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "RegOpenKeyExA"
            | "RegCloseKey"
            | "RegQueryValueExA"
            | "RegSetValueExA"
            | "RegCreateKeyExA"
            | "RegDeleteKeyA"
            | "RegEnumKeyExA"
            | "CryptAcquireContextA"
            | "CryptReleaseContext"
            | "CryptGenRandom"
            | "GetUserNameA"
            | "OpenProcessToken"
            | "GetTokenInformation"
            | "ConvertSidToStringSidA"
            | "SystemFunction036"
    )
}

pub(super) fn is_shell32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "SHGetFolderPathA"
            | "SHGetKnownFolderPath"
            | "ShellExecuteA"
            | "DragQueryFileA"
            | "DragFinish"
            | "CommandLineToArgvW"
            | "SHGetFileInfoA"
    )
}

pub(super) fn is_ole32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "CoInitializeEx"
            | "CoUninitialize"
            | "CoCreateInstance"
            | "CoTaskMemFree"
            | "CoTaskMemAlloc"
            | "OleInitialize"
            | "OleUninitialize"
            | "CLSIDFromString"
            | "StringFromGUID2"
            | "CoInitializeSecurity"
    )
}

pub(super) fn is_oleaut32_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "SysAllocString"
            | "SysFreeString"
            | "SysStringLen"
            | "VariantInit"
            | "VariantClear"
            | "VariantChangeType"
            | "SafeArrayCreate"
            | "SafeArrayDestroy"
    )
}

pub(super) fn is_ntdll_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "RtlGetVersion"
            | "NtQueryInformationProcess"
            | "NtDelayExecution"
            | "RtlNtStatusToDosError"
            | "NtQuerySystemInformation"
            | "RtlAddFunctionTable"
            | "RtlDeleteFunctionTable"
    )
}

pub(super) fn is_bcrypt_symbol(name: &str) -> bool {
    let clean = name.strip_prefix("__imp_").unwrap_or(name);
    matches!(
        clean,
        "BCryptOpenAlgorithmProvider"
            | "BCryptCloseAlgorithmProvider"
            | "BCryptGenRandom"
            | "BCryptCreateHash"
            | "BCryptHashData"
            | "BCryptFinishHash"
            | "BCryptDestroyHash"
            | "BCryptGenerateSymmetricKey"
            | "BCryptEncrypt"
            | "BCryptDecrypt"
            | "BCryptDestroyKey"
    )
}

pub(super) struct ImportData {
    pub(super) data: Vec<u8>,
    pub(super) iat_rvas: HashMap<String, u32>,
    pub(super) iat_rva: u32,
    pub(super) iat_size: u32,
}

pub(super) fn build_imports(
    raw_imports: &[String],
    section_rva: u32,
    extra: &HashMap<String, String>,
) -> Result<ImportData, LinkError> {
    let mut by_dll: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for imp in raw_imports {
        let clean = imp.strip_prefix("__imp_").unwrap_or(imp).to_string();
        let dll = classify_dll(&clean, extra).ok_or_else(|| {
            format!(
                "unresolved PE import '{clean}' — provide it with --import DLL={clean} \
                 or add support for its DLL to the import table"
            )
        })?;
        let slot = by_dll.entry(dll).or_default();
        if !slot.contains(&clean) {
            slot.push(clean);
        }
    }
    let dll_count = by_dll.len();
    let desc_size = if dll_count == 0 {
        0
    } else {
        (dll_count + 1) * 20
    };
    let mut total_entries = 0;
    for funcs in by_dll.values() {
        total_entries += funcs.len() + 1;
    }
    let ilt_size = total_entries * 8;
    let iat_size = total_entries * 8;
    let ilt_off = align_up(desc_size, 8);
    let iat_off = ilt_off + ilt_size;
    let mut data = vec![0u8; iat_off + iat_size];
    let mut iat_rvas = HashMap::new();

    if dll_count > 0 {
        let mut cur_desc = 0;
        let mut cur_ilt = ilt_off;
        let mut cur_iat = iat_off;
        for (dll_name, funcs) in &by_dll {
            let dll_name_off = data.len();
            data.extend_from_slice(dll_name.as_bytes());
            data.push(0);
            if data.len() % 2 != 0 {
                data.push(0);
            }
            let mut hint_off = HashMap::new();
            for f in funcs {
                let h = data.len();
                data.extend_from_slice(&[0u8, 0]); // hint
                data.extend_from_slice(f.as_bytes());
                data.push(0);
                if data.len() % 2 != 0 {
                    data.push(0);
                }
                hint_off.insert(f.clone(), h);
            }
            let this_ilt = section_rva + cur_ilt as u32;
            let this_iat = section_rva + cur_iat as u32;
            let this_name = section_rva + dll_name_off as u32;
            put_u32(&mut data, cur_desc, this_ilt);
            put_u32(&mut data, cur_desc + 12, this_name);
            put_u32(&mut data, cur_desc + 16, this_iat);
            cur_desc += 20;
            for f in funcs {
                let name_rva = section_rva + hint_off[f] as u32;
                let thunk = name_rva as u64;
                data[cur_ilt..cur_ilt + 8].copy_from_slice(&thunk.to_le_bytes());
                data[cur_iat..cur_iat + 8].copy_from_slice(&thunk.to_le_bytes());
                let iat_entry = section_rva + cur_iat as u32;
                iat_rvas.insert(format!("__imp_{f}"), iat_entry);
                iat_rvas.insert(f.clone(), iat_entry);
                cur_ilt += 8;
                cur_iat += 8;
            }
            cur_ilt += 8;
            cur_iat += 8;
        }
    }
    Ok(ImportData {
        data,
        iat_rvas,
        iat_rva: section_rva + iat_off as u32,
        iat_size: iat_size as u32,
    })
}

pub(super) fn generate_base_relocs_from_rvas(rvas: &[u32]) -> Vec<u8> {
    if rvas.is_empty() {
        return Vec::new();
    }
    let mut sorted = rvas.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut reloc = Vec::new();
    let page_size = 0x1000u32;
    let mut i = 0;
    while i < sorted.len() {
        let page_rva = sorted[i] & !(page_size - 1);
        let mut entries = Vec::new();
        while i < sorted.len() && (sorted[i] & !(page_size - 1)) == page_rva {
            let off = (sorted[i] & (page_size - 1)) as u16;
            entries.push(0xA000u16 | off); // IMAGE_REL_BASED_DIR64
            i += 1;
        }
        let block = 8 + entries.len() * 2;
        let padded = align_up(block, 4);
        let start = reloc.len();
        reloc.resize(start + padded, 0);
        put_u32(&mut reloc, start, page_rva);
        put_u32(&mut reloc, start + 4, padded as u32);
        for (idx, e) in entries.iter().enumerate() {
            put_u16(&mut reloc, start + 8 + idx * 2, *e);
        }
    }
    reloc
}
