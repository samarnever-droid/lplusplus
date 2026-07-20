/*
 * Freestanding Windows x86-64 direct-link runtime.
 *
 * This object is compiled by MSVC and merged by lpp-link PE. Its only external
 * dependencies are Kernel32 imports emitted into the PE import directory.
 */

#include <stdint.h>
#include <intrin.h>

typedef void (*LppArcDestructor)(void *payload);
typedef void *HANDLE;
typedef unsigned long DWORD;
typedef int BOOL;
typedef unsigned long long SIZE_T;

__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD standard_handle);
__declspec(dllimport) BOOL __stdcall WriteFile(HANDLE handle, const void *buffer, DWORD bytes_to_write, DWORD *bytes_written, void *overlapped);
__declspec(dllimport) void *__stdcall VirtualAlloc(void *address, SIZE_T size, DWORD allocation_type, DWORD protect);
__declspec(dllimport) BOOL __stdcall VirtualFree(void *address, SIZE_T size, DWORD free_type);

#define STD_OUTPUT_HANDLE ((DWORD)-11)
#define MEM_COMMIT  0x00001000UL
#define MEM_RESERVE 0x00002000UL
#define MEM_RELEASE 0x00008000UL
#define PAGE_READWRITE 0x00000004UL

typedef struct {
    long refcount;
    LppArcDestructor destructor;
    uint64_t allocation_size;
} LppArcHeader;

typedef struct {
    int64_t *data;
    int64_t len;
    int64_t cap;
    uint64_t data_bytes;
    int arc_elements;
} LppList;

static uint64_t lpp_page_round(uint64_t size) {
    return (size + 4095ULL) & ~4095ULL;
}

static void lpp_write(const char *buffer, DWORD length) {
    DWORD written = 0;
    (void)WriteFile(GetStdHandle(STD_OUTPUT_HANDLE), buffer, length, &written, 0);
}

void lpp_print_int(int64_t value) {
    char buffer[32];
    char *cursor = buffer + sizeof(buffer);
    uint64_t magnitude = value < 0 ? (uint64_t)(-(value + 1)) + 1 : (uint64_t)value;
    *--cursor = '\n';
    do {
        *--cursor = (char)('0' + magnitude % 10);
        magnitude /= 10;
    } while (magnitude);
    if (value < 0) *--cursor = '-';
    lpp_write(cursor, (DWORD)((buffer + sizeof(buffer)) - cursor));
}

void lpp_print_str(const char *text) {
    const char *end = text;
    char newline = '\n';
    if (!text) return;
    while (*end) end++;
    lpp_write(text, (DWORD)(end - text));
    lpp_write(&newline, 1);
}

void *lpp_arc_alloc_with_destructor(int64_t payload_size, LppArcDestructor destructor) {
    if (payload_size < 0) return 0;
    uint64_t total = lpp_page_round((uint64_t)payload_size + sizeof(LppArcHeader));
    LppArcHeader *header = (LppArcHeader *)VirtualAlloc(0, total, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    if (!header) return 0;
    header->refcount = 1;
    header->destructor = destructor;
    header->allocation_size = total;
    return header + 1;
}

void *lpp_arc_alloc(int64_t size) { return lpp_arc_alloc_with_destructor(size, 0); }

void lpp_arc_retain(void *payload) {
    if (!payload) return;
    (void)_InterlockedIncrement(&((LppArcHeader *)payload - 1)->refcount);
}

void lpp_arc_release(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    if (_InterlockedDecrement(&header->refcount) == 0) {
        if (header->destructor) header->destructor(payload);
        (void)VirtualFree(header, 0, MEM_RELEASE);
    }
}

void *lpp_alloc(int64_t size) { return lpp_arc_alloc(size); }
void lpp_free(void *payload, int64_t size) { (void)size; lpp_arc_release(payload); }

void lpp_closure_destroy(void *closure) {
    if (!closure) return;
    lpp_arc_release(((void **)closure)[1]);
}

static void lpp_list_destroy(void *payload) {
    LppList *list = (LppList *)payload;
    if (!list) return;
    if (list->arc_elements) {
        for (int64_t i = 0; i < list->len; ++i) {
            lpp_arc_release((void *)(intptr_t)list->data[i]);
        }
    }
    if (list->data) (void)VirtualFree(list->data, 0, MEM_RELEASE);
}

static void *lpp_list_new_with_mode(int arc_elements) {
    LppList *list = (LppList *)lpp_arc_alloc_with_destructor((int64_t)sizeof(LppList), lpp_list_destroy);
    if (!list) return 0;
    list->arc_elements = arc_elements;
    return list;
}

void *lpp_list_new(void) { return lpp_list_new_with_mode(0); }
void *lpp_list_new_arc(void) { return lpp_list_new_with_mode(1); }

void lpp_list_push(void *raw, int64_t value) {
    LppList *list = (LppList *)raw;
    if (!list) return;
    if (list->len == list->cap) {
        int64_t next_cap = list->cap == 0 ? 8 : list->cap * 2;
        if (next_cap < list->cap || next_cap > (int64_t)(0x7fffffffffffffffLL / 8)) return;
        uint64_t next_bytes = lpp_page_round((uint64_t)next_cap * sizeof(int64_t));
        int64_t *next_data = (int64_t *)VirtualAlloc(0, next_bytes, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
        if (!next_data) return;
        for (int64_t i = 0; i < list->len; ++i) next_data[i] = list->data[i];
        if (list->data) (void)VirtualFree(list->data, 0, MEM_RELEASE);
        list->data = next_data;
        list->cap = next_cap;
        list->data_bytes = next_bytes;
    }
    if (list->arc_elements) lpp_arc_retain((void *)(intptr_t)value);
    list->data[list->len++] = value;
}

void lpp_list_push_arc(void *list, void *value) { lpp_list_push(list, (int64_t)(intptr_t)value); }
int64_t lpp_list_get(void *raw, int64_t index) {
    LppList *list = (LppList *)raw;
    return (!list || index < 0 || index >= list->len) ? 0 : list->data[index];
}
void *lpp_list_get_arc(void *list, int64_t index) { return (void *)(intptr_t)lpp_list_get(list, index); }
int64_t lpp_list_len(void *raw) { return raw ? ((LppList *)raw)->len : 0; }
void lpp_list_free(void *list) { lpp_arc_release(list); }

/* ── Thread spawn via CreateThread ─────────────────────────────────────── */
typedef DWORD (__stdcall *LppThreadProc)(void *param);

__declspec(dllimport) void *__stdcall CreateThread(
    void *security, SIZE_T stack_size, LppThreadProc start,
    void *param, DWORD flags, DWORD *thread_id);
__declspec(dllimport) DWORD __stdcall WaitForSingleObject(void *handle, DWORD ms);
__declspec(dllimport) BOOL __stdcall CloseHandle(void *handle);

#define INFINITE 0xFFFFFFFF

void lpp_thread_spawn(void *func_ptr, void *env_ptr) {
    void *handle = CreateThread(0, 0, (LppThreadProc)func_ptr, env_ptr, 0, 0);
    if (handle) {
        WaitForSingleObject(handle, INFINITE);
        CloseHandle(handle);
    }
}
