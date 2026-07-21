/*
 * Freestanding Windows x86-64 direct-link runtime — Phase 4.
 *
 * This object is compiled by MSVC and merged by lpp-link PE. Its only external
 * dependencies are Kernel32 imports emitted into the PE import directory.
 *
 * Full API: print, ARC, closures, lists, threads, string, exec, dir builtins.
 */

#include <stdint.h>
#include <intrin.h>

/* ── Windows types ─────────────────────────────────────────────────────── */
typedef void (*LppArcDestructor)(void *payload);
typedef void *HANDLE;
typedef unsigned long DWORD;
typedef int BOOL;
typedef unsigned long long SIZE_T;
typedef void *LPVOID;
typedef const char *LPCSTR;
typedef unsigned short WORD;

__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD standard_handle);
__declspec(dllimport) BOOL __stdcall WriteFile(HANDLE handle, const void *buffer, DWORD bytes_to_write, DWORD *bytes_written, void *overlapped);
__declspec(dllimport) LPVOID __stdcall VirtualAlloc(LPVOID address, SIZE_T size, DWORD allocation_type, DWORD protect);
__declspec(dllimport) BOOL __stdcall VirtualFree(LPVOID address, SIZE_T size, DWORD free_type);
__declspec(dllimport) BOOL __stdcall CreateProcessA(LPCSTR app, LPCSTR cmd, void *proc_attrs, void *thread_attrs, BOOL inherit_handles, DWORD flags, void *env, LPCSTR cur_dir, void *startup, void *proc_info);
__declspec(dllimport) DWORD __stdcall WaitForSingleObject(HANDLE handle, DWORD ms);
__declspec(dllimport) BOOL __stdcall CloseHandle(HANDLE handle);
__declspec(dllimport) BOOL __stdcall GetExitCodeProcess(HANDLE process, DWORD *code);
__declspec(dllimport) BOOL __stdcall CreatePipe(HANDLE *read_pipe, HANDLE *write_pipe, void *attrs, DWORD size);
__declspec(dllimport) BOOL __stdcall ReadFile(HANDLE file, void *buf, DWORD bytes, DWORD *read, void *overlapped);
__declspec(dllimport) DWORD __stdcall GetEnvironmentVariableA(LPCSTR name, char *buf, DWORD size);
__declspec(dllimport) BOOL __stdcall SetEnvironmentVariableA(LPCSTR name, LPCSTR value);
__declspec(dllimport) BOOL __stdcall CreateDirectoryA(LPCSTR path, void *attrs);
__declspec(dllimport) BOOL __stdcall RemoveDirectoryA(LPCSTR path);
__declspec(dllimport) HANDLE __stdcall FindFirstFileA(LPCSTR pattern, void *data);
__declspec(dllimport) BOOL __stdcall FindNextFileA(HANDLE find, void *data);
__declspec(dllimport) BOOL __stdcall FindClose(HANDLE find);
__declspec(dllimport) DWORD __stdcall GetFileAttributesA(LPCSTR path);
__declspec(dllimport) BOOL __stdcall DeleteFileA(LPCSTR path);
#define CreateThread _lpp_CreateThread
__declspec(dllimport) HANDLE __stdcall CreateThread(void *sec, SIZE_T stack, void *(*start)(void*), void *param, DWORD flags, DWORD *tid);
#undef CreateThread
__declspec(dllimport) void __stdcall Sleep(DWORD ms);

#define STD_OUTPUT_HANDLE ((DWORD)-11)
#define MEM_COMMIT  0x00001000UL
#define MEM_RESERVE 0x00002000UL
#define MEM_RELEASE 0x00008000UL
#define PAGE_READWRITE 0x00000004UL
#define INVALID_HANDLE_VALUE ((HANDLE)(intptr_t)-1)
#define INVALID_FILE_ATTRIBUTES ((DWORD)-1)
#define INFINITE 0xFFFFFFFF
#define MAX_PATH 260
#define STARTF_USESTDHANDLES 0x100
#define CREATE_NO_WINDOW 0x08000000

typedef struct { DWORD c; LPVOID r; LPVOID w; DWORD f; WORD so; WORD sx; WORD sy; WORD sx2; WORD sy2; LPVOID r2; LPVOID r3; LPVOID r4; LPVOID r5; DWORD f2; WORD si; WORD sx3; } STARTUPINFOA;
typedef struct { HANDLE p; HANDLE t; DWORD pi; DWORD ti; } PROCESS_INFORMATION;
typedef struct { void *b1; DWORD b2; DWORD b3; char cFileName[260]; char cAlt[14]; DWORD f1; DWORD f2; DWORD f3; DWORD f4; DWORD f5; int64_t f6; int64_t f7; DWORD f8; char f9[20]; DWORD f10; } WIN32_FIND_DATAA;

/* ── ARC header ────────────────────────────────────────────────────────── */
typedef struct { long refcount; LppArcDestructor destructor; uint64_t allocation_size; } LppArcHeader;
typedef struct { int64_t *data; int64_t len; int64_t cap; uint64_t data_bytes; int arc_elements; } LppList;

/* ── Minimal freestanding libc replacements ────────────────────────────── */
static uint64_t lpp_page_round(uint64_t size) { return (size + 4095ULL) & ~4095ULL; }
static int lpp_strlen(const char *s) { int n=0; while(s&&s[n])n++; return n; }
static void lpp_memcpy(char *d, const char *s, int n) { for(int i=0;i<n;i++)d[i]=s[i]; }
static int lpp_strcmp(const char *a, const char *b) { while(*a&&*a==*b){a++;b++;} return *a-*b; }
static void lpp_strcpy(char *d, const char *s) { while((*d++=*s++)); }
static char* lpp_strdup(const char *s) { if(!s)return 0; int n=lpp_strlen(s); char*d=(char*)VirtualAlloc(0,lpp_page_round(n+1),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE); if(d){lpp_memcpy(d,s,n);d[n]=0;} return d; }
static char* lpp_strstr(const char *h, const char *n) { int nl=lpp_strlen(n); if(!nl)return(char*)h; while(*h){int i=0;while(i<nl&&h[i]==n[i])i++;if(i==nl)return(char*)h;h++;} return 0; }
static int lpp_isspace(char c) { return c==' '||c=='\t'||c=='\n'||c=='\r'; }
static void lpp_free_str(char *p) { if(p) VirtualFree(p,0,MEM_RELEASE); }

/* ── Output ────────────────────────────────────────────────────────────── */
static void lpp_write(const char *buffer, DWORD length) {
    DWORD written = 0;
    WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), buffer, length, &written, 0);
}

void lpp_print_int(int64_t value) {
    char buffer[32]; char *cursor = buffer + sizeof(buffer);
    uint64_t magnitude = value < 0 ? (uint64_t)(-(value + 1)) + 1 : (uint64_t)value;
    *--cursor = '\n';
    do { *--cursor = (char)('0' + magnitude % 10); magnitude /= 10; } while (magnitude);
    if (value < 0) *--cursor = '-';
    lpp_write(cursor, (DWORD)((buffer + sizeof(buffer)) - cursor));
}

void lpp_print_str(const char *text) {
    if (!text) return;
    int len = lpp_strlen(text);
    lpp_write(text, (DWORD)len);
    lpp_write("\n", 1);
}

/* ── ARC ───────────────────────────────────────────────────────────────── */
void *lpp_arc_alloc_with_destructor(int64_t payload_size, LppArcDestructor destructor) {
    if (payload_size < 0) return 0;
    uint64_t total = lpp_page_round((uint64_t)payload_size + sizeof(LppArcHeader));
    LppArcHeader *header = (LppArcHeader *)VirtualAlloc(0, total, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if (!header) return 0;
    header->refcount = 1; header->destructor = destructor; header->allocation_size = total;
    return header + 1;
}
void *lpp_arc_alloc(int64_t size) { return lpp_arc_alloc_with_destructor(size, 0); }
void lpp_arc_retain(void *payload) { if(payload)(void)_InterlockedIncrement(&((LppArcHeader *)payload - 1)->refcount); }
void lpp_arc_release(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    if (_InterlockedDecrement(&header->refcount) == 0) {
        if (header->destructor) header->destructor(payload);
        VirtualFree(header, 0, MEM_RELEASE);
    }
}
void *lpp_alloc(int64_t size) { return lpp_arc_alloc(size); }
void lpp_free(void *payload, int64_t size) { (void)size; lpp_arc_release(payload); }
void lpp_closure_destroy(void *closure) { if(closure) lpp_arc_release(((void **)closure)[1]); }

/* ── Lists ─────────────────────────────────────────────────────────────── */
static void lpp_list_destroy(void *payload) {
    LppList *list = (LppList *)payload; if (!list) return;
    if (list->arc_elements) { for (int64_t i = 0; i < list->len; ++i) lpp_arc_release((void *)(intptr_t)list->data[i]); }
    if (list->data) VirtualFree(list->data, 0, MEM_RELEASE);
}
static void *lpp_list_new_with_mode(int arc_elements) {
    LppList *list = (LppList *)lpp_arc_alloc_with_destructor((int64_t)sizeof(LppList), lpp_list_destroy);
    if (!list) return 0; list->arc_elements = arc_elements; return list;
}
void *lpp_list_new(void) { return lpp_list_new_with_mode(0); }
void *lpp_list_new_arc(void) { return lpp_list_new_with_mode(1); }
void lpp_list_push(void *raw, int64_t value) {
    LppList *list = (LppList *)raw; if (!list) return;
    if (list->len == list->cap) {
        int64_t next_cap = list->cap == 0 ? 8 : list->cap * 2;
        if (next_cap < list->cap || next_cap > (int64_t)(0x7fffffffffffffffLL / 8)) return;
        uint64_t next_bytes = lpp_page_round((uint64_t)next_cap * sizeof(int64_t));
        int64_t *next_data = (int64_t *)VirtualAlloc(0, next_bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        if (!next_data) return;
        for (int64_t i = 0; i < list->len; ++i) next_data[i] = list->data[i];
        if (list->data) VirtualFree(list->data, 0, MEM_RELEASE);
        list->data = next_data; list->cap = next_cap; list->data_bytes = next_bytes;
    }
    if (list->arc_elements) lpp_arc_retain((void *)(intptr_t)value);
    list->data[list->len++] = value;
}
void lpp_list_push_arc(void *list, void *value) { lpp_list_push(list, (int64_t)(intptr_t)value); }
int64_t lpp_list_get(void *raw, int64_t index) { LppList *l=(LppList*)raw; return (!l||index<0||index>=l->len)?0:l->data[index]; }
void *lpp_list_get_arc(void *list, int64_t index) { return (void*)(intptr_t)lpp_list_get(list,index); }
int64_t lpp_list_len(void *raw) { return raw ? ((LppList *)raw)->len : 0; }
void lpp_list_free(void *list) { lpp_arc_release(list); }

/* ── Threads ───────────────────────────────────────────────────────────── */
typedef DWORD (__stdcall *LppThreadProc)(void *param);
void lpp_thread_spawn(void *func_ptr, void *env_ptr) {
    HANDLE h = CreateThread(0, 0, (DWORD(__stdcall*)(void*))func_ptr, env_ptr, 0, 0);
    if (h) { WaitForSingleObject(h, INFINITE); CloseHandle(h); }
}

/* ═══════════════════════════════════════════════════════════════════════════
 * STRING BUILTINS  (freestanding, Kernel32-only)
 * ═══════════════════════════════════════════════════════════════════════════ */

char *lpp_str_concat(const char *a, const char *b) {
    if (!a) a = ""; if (!b) b = "";
    int la = lpp_strlen(a), lb = lpp_strlen(b);
    char *out = (char *)lpp_arc_alloc((int64_t)(la + lb + 1));
    if (!out) return (char *)"";
    lpp_memcpy(out, a, la); lpp_memcpy(out + la, b, lb);
    out[la + lb] = 0;
    return out;
}

void *lpp_str_split(const char *s, int64_t delim) {
    void *list = lpp_list_new_arc();
    if (!list) return 0;
    if (!s || !*s) return list;
    char ch = (char)delim;
    const char *start = s;
    for (;;) {
        if (*s == ch || *s == 0) {
            int64_t len = (int64_t)(s - start);
            char *piece = (char *)lpp_arc_alloc(len + 1);
            if (piece) {
                lpp_memcpy(piece, start, (int)len); piece[len] = 0;
                lpp_list_push_arc(list, piece);
                lpp_arc_release(piece);
            }
            if (*s == 0) break;
            start = s + 1;
        }
        s++;
    }
    return list;
}

int64_t lpp_str_find(const char *haystack, const char *needle) {
    if (!haystack || !needle) return -1;
    const char *found = lpp_strstr(haystack, needle);
    if (!found) return -1;
    return (int64_t)(found - haystack);
}

char *lpp_str_replace(const char *s, const char *old, const char *new_) {
    if (!s) s = ""; if (!old || !*old) return (char *)s; if (!new_) new_ = "";
    int slen = lpp_strlen(s), olen = lpp_strlen(old), nlen = lpp_strlen(new_);
    int64_t count = 0;
    const char *scan = s;
    while ((scan = lpp_strstr(scan, old))) { count++; scan += olen; }
    int outlen = slen + (int)count * (nlen - olen) + 1;
    char *out = (char *)lpp_arc_alloc((int64_t)outlen);
    if (!out) return (char *)"";
    char *dst = out; const char *src = s;
    while (*src) {
        const char *next = lpp_strstr(src, old);
        if (!next) { lpp_strcpy(dst, src); break; }
        int prefix = (int)(next - src);
        lpp_memcpy(dst, src, prefix); dst += prefix;
        lpp_memcpy(dst, new_, nlen);  dst += nlen;
        src = next + olen;
    }
    return out;
}

char *lpp_str_substr(const char *s, int64_t start, int64_t length) {
    if (!s) s = ""; int slen = lpp_strlen(s);
    if (start < 0) start = 0; if (start > (int64_t)slen) return (char *)"";
    int remain = slen - (int)start;
    int copy = (length < 0 || (size_t)length > (size_t)remain) ? remain : (int)length;
    char *out = (char *)lpp_arc_alloc((int64_t)(copy + 1));
    if (!out) return (char *)"";
    lpp_memcpy(out, s + start, copy); out[copy] = 0;
    return out;
}

char *lpp_str_trim(const char *s) {
    if (!s) return (char *)"";
    while (lpp_isspace(*s)) s++;
    int len = lpp_strlen(s);
    while (len > 0 && lpp_isspace(s[len - 1])) len--;
    char *out = (char *)lpp_arc_alloc((int64_t)(len + 1));
    if (!out) return (char *)"";
    lpp_memcpy(out, s, len); out[len] = 0;
    return out;
}

/* ═══════════════════════════════════════════════════════════════════════════
 * EXEC BUILTINS  (freestanding, Kernel32-only)
 * ═══════════════════════════════════════════════════════════════════════════ */

int64_t lpp_command_exec(const char *cmdline) {
    if (!cmdline || !*cmdline) return -1;
    char *dup = lpp_strdup(cmdline);
    if (!dup) return -1;
    STARTUPINFOA si; for(int i=0;i<(int)sizeof(si);i++) ((char*)&si)[i]=0;
    si.c = sizeof(si); si.dwFlags = STARTF_USESTDHANDLES;
    PROCESS_INFORMATION pi;
    BOOL ok = CreateProcessA(NULL, dup, NULL, NULL, FALSE, CREATE_NO_WINDOW, NULL, NULL, (void*)&si, &pi);
    lpp_free_str(dup);
    if (!ok) return -1;
    WaitForSingleObject(pi.p, INFINITE);
    DWORD code; GetExitCodeProcess(pi.p, &code);
    CloseHandle(pi.p); CloseHandle(pi.t);
    return (int64_t)(int)code;
}

char *lpp_command_output(const char *cmdline) {
    if (!cmdline) return (char *)"";
    HANDLE hRead, hWrite;
    if (!CreatePipe(&hRead, &hWrite, NULL, 0)) return (char *)"";

    STARTUPINFOA si; for(int i=0;i<(int)sizeof(si);i++) ((char*)&si)[i]=0;
    si.c = sizeof(si); si.dwFlags = STARTF_USESTDHANDLES;
    si.w = hWrite; si.w = hWrite; /* stdout = stderr = pipe write end */

    char *dup = lpp_strdup(cmdline);
    PROCESS_INFORMATION pi;
    BOOL ok = CreateProcessA(NULL, dup, NULL, NULL, TRUE, CREATE_NO_WINDOW, NULL, NULL, (void*)&si, &pi);
    lpp_free_str(dup);
    CloseHandle(hWrite);
    if (!ok) { CloseHandle(hRead); return (char *)""; }

    WaitForSingleObject(pi.p, INFINITE);
    CloseHandle(pi.p); CloseHandle(pi.t);

    int cap = 4096, len = 0;
    char *buf = (char *)lpp_arc_alloc((int64_t)(cap + 1));
    if (!buf) { CloseHandle(hRead); return (char *)""; }
    for (;;) {
        if (len + 1024 >= cap) {
            int nc = cap * 2;
            char *nb = (char *)lpp_arc_alloc((int64_t)(nc + 1));
            if (!nb) break;
            lpp_memcpy(nb, buf, len); lpp_arc_release(buf);
            buf = nb; cap = nc;
        }
        DWORD n;
        if (!ReadFile(hRead, buf + len, (DWORD)(cap - len), &n, NULL) || n == 0) break;
        len += (int)n;
    }
    CloseHandle(hRead);
    buf[len] = 0;
    return buf;
}

char *lpp_env_get(const char *name) {
    if (!name) return (char *)"";
    char val[4096];
    DWORD n = GetEnvironmentVariableA(name, val, sizeof(val));
    if (n == 0 || n >= sizeof(val)) return (char *)"";
    char *out = (char *)lpp_arc_alloc((int64_t)(n + 1));
    if (!out) return (char *)"";
    lpp_memcpy(out, val, (int)n);
    out[n] = 0;
    return out;
}

int64_t lpp_env_set(const char *name, const char *value) {
    if (!name) return -1;
    return SetEnvironmentVariableA(name, value ? value : "") ? 0 : -1;
}

/* ═══════════════════════════════════════════════════════════════════════════
 * DIR BUILTINS  (freestanding, Kernel32-only)
 * ═══════════════════════════════════════════════════════════════════════════ */

int64_t lpp_dir_create(const char *path) {
    if (!path) return -1;
    return CreateDirectoryA(path, NULL) ? 0 : -1;
}

void *lpp_dir_list(const char *path) {
    void *list = lpp_list_new_arc();
    if (!list) return 0; if (!path) return list;

    char pattern[MAX_PATH + 4];
    int plen = lpp_strlen(path);
    lpp_memcpy(pattern, path, plen);
    pattern[plen] = '\\'; pattern[plen + 1] = '*'; pattern[plen + 2] = 0;

    WIN32_FIND_DATAA fd;
    HANDLE h = FindFirstFileA(pattern, &fd);
    if (h == INVALID_HANDLE_VALUE) return list;

    do {
        if (lpp_strcmp(fd.cFileName, ".") == 0 || lpp_strcmp(fd.cFileName, "..") == 0) continue;
        int len = lpp_strlen(fd.cFileName);
        char *copy = (char *)lpp_arc_alloc((int64_t)(len + 1));
        if (copy) { lpp_memcpy(copy, fd.cFileName, len); copy[len] = 0;
                    lpp_list_push_arc(list, copy); lpp_arc_release(copy); }
    } while (FindNextFileA(h, &fd));
    FindClose(h);
    return list;
}

int64_t lpp_dir_remove(const char *path) {
    if (!path) return -1;
    /* Recursive: delete all files then the directory itself */
    void *files = lpp_dir_list(path);
    if (files) {
        int n = (int)lpp_list_len(files);
        for (int i = 0; i < n; i++) {
            char *name = (char *)lpp_list_get_arc(files, (int64_t)i);
            if (name && *name) {
                char full[MAX_PATH * 2];
                int plen = lpp_strlen(path);
                lpp_memcpy(full, path, plen);
                full[plen] = '\\';
                lpp_strcpy(full + plen + 1, name);
                DeleteFileA(full);
            }
        }
        lpp_list_free(files);
    }
    return RemoveDirectoryA(path) ? 0 : -1;
}

int64_t lpp_path_exists(const char *path) {
    if (!path) return 0;
    DWORD attr = GetFileAttributesA(path);
    return (attr != INVALID_FILE_ATTRIBUTES) ? 1 : 0;
}

char *lpp_path_join(const char *base, const char *child) {
    if (!base) base = ""; if (!child) child = "";
    int blen = lpp_strlen(base), clen = lpp_strlen(child);
    int need_sep = (blen > 0 && base[blen - 1] != '\\' && base[blen - 1] != '/');
    int64_t total = (int64_t)(blen + (need_sep ? 1 : 0) + clen + 1);
    char *out = (char *)lpp_arc_alloc(total);
    if (!out) return (char *)"";
    lpp_memcpy(out, base, blen);
    int off = blen;
    if (need_sep) out[off++] = '\\';
    lpp_memcpy(out + off, child, clen);
    out[off + clen] = 0;
    return out;
}
