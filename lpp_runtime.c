/*
 * lpp_runtime.c  —  L++ Runtime Library (C implementation)
 *
 * Compile once into a static library or object file, then link with any
 * L++-generated object file to produce a native executable.
 *
 * Build:
 *   cl.exe  /nologo /O2 /c lpp_runtime.c /Fo:lpp_runtime.obj
 *   gcc -O2 -c lpp_runtime.c -o lpp_runtime.o
 *   clang -O2 -c lpp_runtime.c -o lpp_runtime.o
 */

/* Expose POSIX networking declarations (getaddrinfo, addrinfo) under strict C. */
#if defined(__APPLE__) || defined(__MACH__)
#  ifndef _DARWIN_C_SOURCE
#    define _DARWIN_C_SOURCE
#  endif
#endif
#if !defined(_WIN32)
#  ifndef _GNU_SOURCE
#    define _GNU_SOURCE
#  endif
#  undef _POSIX_C_SOURCE
#  define _POSIX_C_SOURCE 200809L
#endif

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <limits.h>
#include <errno.h>
#include <signal.h>
#include <stdarg.h>
#include <time.h>
#include <math.h>
#include <sys/types.h>
#if defined(_WIN32)
#  include <direct.h>
#else
#  include <sys/stat.h>
#  include <fcntl.h>
#  include <unistd.h>
#endif

/* Forward declarations: the string builtins below hand their results to
 * generated L++ code as owned `Str` values, so they must allocate through ARC
 * even though the ARC implementation appears further down this file. */
void *lpp_arc_alloc(int64_t size);
void lpp_arc_release(void *ptr);
char *lpp_empty_str(void);

#if defined(_WIN32)
#  ifndef WIN32_LEAN_AND_MEAN
#    define WIN32_LEAN_AND_MEAN
#  endif

/* Forward declarations for ARC and List runtime functions */
void *lpp_arc_alloc(int64_t size);
void  lpp_arc_release(void *ptr);
void *lpp_list_new_arc(void);
void  lpp_list_push_arc(void *list, void *value);
int64_t lpp_list_len(void *list);
void *lpp_list_get_arc(void *list, int64_t index);
void  lpp_list_free(void *list);
#  include <winsock2.h>
#  include <ws2tcpip.h>
#  include <windows.h>
typedef USHORT (WINAPI *lpp_CaptureStackBackTrace_fn)(ULONG, ULONG, PVOID*, PULONG);
#elif defined(__linux__) || defined(__APPLE__)
#  include <execinfo.h>
#endif

static int g_lpp_crash_handler_installed = 0;

static void lpp_flush_all(void) {
    fflush(stdout);
    fflush(stderr);
}

#if defined(_WIN32)
typedef struct _LPP_SYMBOL_INFO {
    ULONG SizeOfStruct;
    ULONG TypeIndex;
    ULONG64 Reserved[2];
    ULONG Index;
    ULONG Size;
    ULONG64 ModBase;
    ULONG Flags;
    ULONG64 Value;
    ULONG64 Address;
    ULONG Register;
    ULONG Scope;
    ULONG Tag;
    ULONG NameLen;
    ULONG MaxNameLen;
    CHAR Name[1];
} LPP_SYMBOL_INFO;

typedef struct _LPP_IMAGEHLP_LINE64 {
    DWORD SizeOfStruct;
    PVOID Key;
    DWORD LineNumber;
    PCHAR FileName;
    DWORD64 Address;
} LPP_IMAGEHLP_LINE64;

typedef BOOL (WINAPI *lpp_SymInitialize_fn)(HANDLE, PCSTR, BOOL);
typedef BOOL (WINAPI *lpp_SymFromAddr_fn)(HANDLE, DWORD64, PDWORD64, LPP_SYMBOL_INFO*);
typedef BOOL (WINAPI *lpp_SymGetLineFromAddr64_fn)(HANDLE, DWORD64, PDWORD, LPP_IMAGEHLP_LINE64*);
typedef BOOL (WINAPI *lpp_SymCleanup_fn)(HANDLE);
#endif

void lpp_print_backtrace(void) {
    fprintf(stderr, "\nStack Backtrace:\n");
#if defined(_WIN32)
    void *stack[32];
    HMODULE hKernel32 = GetModuleHandleA("kernel32.dll");
    if (hKernel32) {
        lpp_CaptureStackBackTrace_fn pCapture = (lpp_CaptureStackBackTrace_fn)(void*)GetProcAddress(hKernel32, "RtlCaptureStackBackTrace");
        if (pCapture) {
            USHORT frames = pCapture(0, 32, stack, NULL);
            HMODULE hDbgHelp = LoadLibraryA("dbghelp.dll");
            lpp_SymInitialize_fn pSymInit = hDbgHelp ? (lpp_SymInitialize_fn)(void*)GetProcAddress(hDbgHelp, "SymInitialize") : NULL;
            lpp_SymFromAddr_fn pSymFromAddr = hDbgHelp ? (lpp_SymFromAddr_fn)(void*)GetProcAddress(hDbgHelp, "SymFromAddr") : NULL;
            lpp_SymGetLineFromAddr64_fn pSymGetLine = hDbgHelp ? (lpp_SymGetLineFromAddr64_fn)(void*)GetProcAddress(hDbgHelp, "SymGetLineFromAddr64") : NULL;
            lpp_SymCleanup_fn pSymCleanup = hDbgHelp ? (lpp_SymCleanup_fn)(void*)GetProcAddress(hDbgHelp, "SymCleanup") : NULL;

            HANDLE hProcess = GetCurrentProcess();
            BOOL symInitialized = FALSE;
            if (pSymInit) {
                symInitialized = pSymInit(hProcess, NULL, TRUE);
            }

            char symbolBuffer[sizeof(LPP_SYMBOL_INFO) + 256];
            memset(symbolBuffer, 0, sizeof(symbolBuffer));
            LPP_SYMBOL_INFO *pSymbol = (LPP_SYMBOL_INFO*)symbolBuffer;
            pSymbol->SizeOfStruct = sizeof(LPP_SYMBOL_INFO);
            pSymbol->MaxNameLen = 255;

            LPP_IMAGEHLP_LINE64 lineInfo;
            memset(&lineInfo, 0, sizeof(lineInfo));
            lineInfo.SizeOfStruct = sizeof(LPP_IMAGEHLP_LINE64);

            for (USHORT i = 0; i < frames; i++) {
                DWORD64 addr = (DWORD64)(uintptr_t)stack[i];
                DWORD64 displacement = 0;
                DWORD lineDisplacement = 0;
                if (symInitialized && pSymFromAddr && pSymFromAddr(hProcess, addr, &displacement, pSymbol)) {
                    if (pSymGetLine && pSymGetLine(hProcess, addr, &lineDisplacement, &lineInfo) && lineInfo.FileName) {
                        fprintf(stderr, "  [%2u] 0x%p: %s (%s:%lu)\n", (unsigned int)i, stack[i], pSymbol->Name, lineInfo.FileName, (unsigned long)lineInfo.LineNumber);
                    } else {
                        fprintf(stderr, "  [%2u] 0x%p: %s\n", (unsigned int)i, stack[i], pSymbol->Name);
                    }
                } else {
                    fprintf(stderr, "  [%2u] 0x%p\n", (unsigned int)i, stack[i]);
                }
            }
            if (symInitialized && pSymCleanup) {
                pSymCleanup(hProcess);
            }
            return;
        }
    }
    fprintf(stderr, "  (backtrace unavailable)\n");
#elif defined(__linux__) || defined(__APPLE__)
    void *buffer[32];
    int nptrs = backtrace(buffer, 32);
    char **strings = backtrace_symbols(buffer, nptrs);
    if (strings) {
        for (int i = 0; i < nptrs; i++) {
            fprintf(stderr, "  [%2d] %s\n", i, strings[i]);
        }
        free(strings);
    } else {
        for (int i = 0; i < nptrs; i++) {
            fprintf(stderr, "  [%2d] %p\n", i, buffer[i]);
        }
    }
#else
    fprintf(stderr, "  (backtrace unavailable)\n");
#endif
}

void lpp_panic(const char *fmt, ...) {
    fflush(stdout);
    fprintf(stderr, "\n===================================================================\n");
    fprintf(stderr, "💥 L++ RUNTIME PANIC\n");
    fprintf(stderr, "===================================================================\n");
    fprintf(stderr, "Reason: ");
    va_list args;
    va_start(args, fmt);
    vfprintf(stderr, fmt, args);
    va_end(args);
    fprintf(stderr, "\n");

    lpp_print_backtrace();
    fprintf(stderr, "===================================================================\n\n");
    fflush(stderr);
    fflush(stdout);
    exit(101);
}

#if !defined(_WIN32)
#include <unistd.h>
#endif

static void lpp_signal_handler(int sig) {
    const char *sig_name = "Unknown Signal";
    switch (sig) {
        case SIGSEGV: sig_name = "Segmentation Fault (SIGSEGV) - Null pointer or invalid memory access"; break;
        case SIGFPE:  sig_name = "Floating Point Exception (SIGFPE) - Integer division by zero or arithmetic error"; break;
        case SIGABRT: sig_name = "Abort Signal (SIGABRT) - Process abort triggered"; break;
        case SIGILL:  sig_name = "Illegal Instruction (SIGILL) - Invalid CPU instruction execute attempt"; break;
    }
    fflush(stdout);
    fflush(stderr);
#if defined(_WIN32)
    HANDLE hErr = GetStdHandle(STD_ERROR_HANDLE);
    if (hErr && hErr != INVALID_HANDLE_VALUE) {
        DWORD written = 0;
        const char *p1 = "\n===================================================================\n💥 L++ RUNTIME PANIC (Signal)\n===================================================================\nReason: Fatal Hardware/OS Signal Received: ";
        WriteFile(hErr, p1, (DWORD)strlen(p1), &written, NULL);
        WriteFile(hErr, sig_name, (DWORD)strlen(sig_name), &written, NULL);
        const char *p2 = "\n===================================================================\n\n";
        WriteFile(hErr, p2, (DWORD)strlen(p2), &written, NULL);
    }
    ExitProcess(101);
#else
    const char *p1 = "\n===================================================================\n💥 L++ RUNTIME PANIC (Signal)\n===================================================================\nReason: Fatal Hardware/OS Signal Received: ";
    ssize_t w1 = write(2, p1, strlen(p1));
    ssize_t w2 = write(2, sig_name, strlen(sig_name));
    const char *p2 = "\n===================================================================\n\n";
    ssize_t w3 = write(2, p2, strlen(p2));
    (void)w1; (void)w2; (void)w3;
    _exit(101);
#endif
}

void lpp_init_crash_handler(void) {
    if (g_lpp_crash_handler_installed) return;
    g_lpp_crash_handler_installed = 1;
#if defined(_WIN32)
    SetErrorMode(SEM_FAILCRITICALERRORS | SEM_NOGPFAULTERRORBOX);
#endif
    atexit(lpp_flush_all);
    signal(SIGSEGV, lpp_signal_handler);
    signal(SIGFPE,  lpp_signal_handler);
    signal(SIGABRT, lpp_signal_handler);
    signal(SIGILL,  lpp_signal_handler);
}

#if defined(_MSC_VER)
#pragma section(".CRT$XCU", read)
__declspec(allocate(".CRT$XCU")) static void (*p_lpp_init_crash_handler)(void) = lpp_init_crash_handler;
#elif defined(__GNUC__) || defined(__clang__)
__attribute__((constructor)) static void lpp_auto_init_crash_handler(void) {
    lpp_init_crash_handler();
}
#endif

#include "runtime/lpp_runtime/io.inc"
#include "runtime/lpp_runtime/arc.inc"
#include "runtime/lpp_runtime/cycle.inc"
#include "runtime/lpp_runtime/arc_tasks.inc"
#include "runtime/lpp_runtime/lists.inc"
#include "runtime/lpp_runtime/slices_network.inc"
#include "runtime/lpp_runtime/json.inc"
#include "runtime/lpp_runtime/extras.inc"
#include "runtime/lpp_runtime/pgo.inc"
