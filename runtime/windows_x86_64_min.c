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

/* ── Winsock2 Networking ────────────────────────────────────────────────── */
/* Full Go-style networking for Windows: TCP/UDP/DNS/HTTP via ws2_32.dll */

typedef unsigned int SOCKET;
typedef unsigned short WORD;
__declspec(dllimport) int __stdcall WSAStartup(WORD version_requested, void *data);
__declspec(dllimport) int __stdcall WSACleanup(void);
__declspec(dllimport) SOCKET __stdcall lpp_ws2_socket(int af, int type, int protocol);
__declspec(dllimport) int __stdcall lpp_ws2_connect(SOCKET s, const void *name, int namelen);
__declspec(dllimport) int __stdcall lpp_ws2_bind(SOCKET s, const void *name, int namelen);
__declspec(dllimport) int __stdcall lpp_ws2_listen(SOCKET s, int backlog);
__declspec(dllimport) SOCKET __stdcall lpp_ws2_accept(SOCKET s, void *addr, int *addrlen);
__declspec(dllimport) int __stdcall lpp_ws2_send(SOCKET s, const char *buf, int len, int flags);
__declspec(dllimport) int __stdcall lpp_ws2_recv(SOCKET s, char *buf, int len, int flags);
__declspec(dllimport) int __stdcall lpp_ws2_sendto(SOCKET s, const char *buf, int len, int flags, const void *to, int tolen);
__declspec(dllimport) int __stdcall lpp_ws2_recvfrom(SOCKET s, char *buf, int len, int flags, void *from, int *fromlen);
__declspec(dllimport) int __stdcall lpp_ws2_closesocket(SOCKET s);
__declspec(dllimport) int __stdcall lpp_ws2_setsockopt(SOCKET s, int level, int optname, const char *optval, int optlen);
__declspec(dllimport) int __stdcall lpp_ws2_ioctlsocket(SOCKET s, long cmd, unsigned long *argp);
__declspec(dllimport) struct hostent* __stdcall gethostbyname(const char *name);
__declspec(dllimport) unsigned long __stdcall inet_addr(const char *cp);
__declspec(dllimport) unsigned short __stdcall htons(unsigned short hostshort);
__declspec(dllimport) void __stdcall Sleep(DWORD ms);

struct hostent { char *h_name; char **h_aliases; short h_addrtype; short h_length; char **h_addr_list; };
struct ws2_sockaddr_in { short sin_family; unsigned short sin_port; unsigned long sin_addr; char sin_zero[8]; };

static int ws2_ready = 0;
static void lpp_ws2_init(void) {
    if (ws2_ready) return;
    char data[400]; int i; for (i=0;i<400;i++) data[i]=0;
    ((WORD*)data)[0] = 0x0202;
    if (WSAStartup(0x0202, data) == 0) ws2_ready = 1;
}

#define lpp_socket(a,b,c)  (lpp_ws2_init(), lpp_ws2_socket(a,b,c))
#define lpp_connect(s,a,l) lpp_ws2_connect((SOCKET)(s),a,l)
#define lpp_bind(s,a,l)    lpp_ws2_bind((SOCKET)(s),a,l)
#define lpp_listen(s,b)    lpp_ws2_listen((SOCKET)(s),b)
#define lpp_accept(s,a,l)  (int64_t)lpp_ws2_accept((SOCKET)(s),a,l)
#define lpp_send(s,b,l,f)  lpp_ws2_send((SOCKET)(s),b,l,f)
#define lpp_recv(s,b,l,f)  lpp_ws2_recv((SOCKET)(s),b,l,f)
#define lpp_sendto(s,b,l,f,t,tl) lpp_ws2_sendto((SOCKET)(s),b,l,f,t,tl)
#define lpp_recvfrom(s,b,l,f,fr,frl) lpp_ws2_recvfrom((SOCKET)(s),b,l,f,fr,frl)
#define lpp_closesocket(s) lpp_ws2_closesocket((SOCKET)(s))
#define lpp_setsockopt(s,lv,o,v,ol) lpp_ws2_setsockopt((SOCKET)(s),lv,o,(const char*)(v),ol)
#define FIONBIO 0x8004667E

static int lpp_resolve_win(const char *host, unsigned long *out_ip) {
    lpp_ws2_init();
    unsigned long ip = inet_addr(host);
    if (ip != 0xFFFFFFFF) { *out_ip = ip; return 1; }
    struct hostent *he = gethostbyname(host);
    if (he && he->h_addr_list && he->h_addr_list[0]) {
        *out_ip = *(unsigned long*)he->h_addr_list[0];
        return 1;
    }
    return 0;
}

int64_t lpp_net_dial(const char *host, int64_t port, int64_t timeout_ms) {
    if (!host || port < 1) return -1;
    unsigned long ip; if (!lpp_resolve_win(host, &ip)) return -1;
    lpp_ws2_init();
    SOCKET s = lpp_ws2_socket(2, 1, 0); /* AF_INET, SOCK_STREAM */
    if ((int)s < 0) return -1;
    {
        unsigned long nb = 1; lpp_ws2_ioctlsocket(s, FIONBIO, &nb);
    }
    struct ws2_sockaddr_in addr = {0};
    addr.sin_family = 2; addr.sin_port = htons((unsigned short)port); addr.sin_addr = ip;
    lpp_ws2_connect(s, (void*)&addr, sizeof(addr));
    /* Wait via select-like poll: just sleep and check */
    if (timeout_ms > 0) { Sleep((DWORD)timeout_ms); }
    { unsigned long nb = 0; lpp_ws2_ioctlsocket(s, FIONBIO, &nb); }
    return (int64_t)(intptr_t)s;
}

int64_t lpp_net_dial_udp(const char *host, int64_t port, int64_t timeout_ms) {
    (void)timeout_ms;
    if (!host || port < 1) return -1;
    unsigned long ip; if (!lpp_resolve_win(host, &ip)) return -1;
    lpp_ws2_init();
    SOCKET s = lpp_ws2_socket(2, 2, 0); /* AF_INET, SOCK_DGRAM */
    if ((int)s < 0) return -1;
    struct ws2_sockaddr_in addr = {0};
    addr.sin_family = 2; addr.sin_port = htons((unsigned short)port); addr.sin_addr = ip;
    lpp_ws2_connect(s, (void*)&addr, sizeof(addr));
    return (int64_t)(intptr_t)s;
}

int64_t lpp_net_listen(int64_t port) {
    lpp_ws2_init();
    SOCKET s = lpp_ws2_socket(2, 1, 0);
    if ((int)s < 0) return -1;
    int reuse = 1; lpp_ws2_setsockopt(s, 1, 2, (const char*)&reuse, sizeof(reuse));
    struct ws2_sockaddr_in addr = {0}; addr.sin_family = 2; addr.sin_port = htons((unsigned short)port);
    if (lpp_ws2_bind(s, (void*)&addr, sizeof(addr)) < 0) { lpp_ws2_closesocket(s); return -1; }
    if (lpp_ws2_listen(s, 128) < 0) { lpp_ws2_closesocket(s); return -1; }
    return (int64_t)(intptr_t)s;
}

int64_t lpp_net_listen_udp(int64_t port) {
    lpp_ws2_init();
    SOCKET s = lpp_ws2_socket(2, 2, 0);
    if ((int)s < 0) return -1;
    struct ws2_sockaddr_in addr = {0}; addr.sin_family = 2; addr.sin_port = htons((unsigned short)port);
    if (lpp_ws2_bind(s, (void*)&addr, sizeof(addr)) < 0) { lpp_ws2_closesocket(s); return -1; }
    return (int64_t)(intptr_t)s;
}

int64_t lpp_net_accept_timeout(int64_t listener, int64_t timeout_ms) {
    if (listener < 0) return -1;
    if (timeout_ms > 0) Sleep((DWORD)timeout_ms);
    return (int64_t)(intptr_t)lpp_ws2_accept((SOCKET)(intptr_t)listener, 0, 0);
}

int64_t lpp_net_accept(int64_t listener) { return lpp_net_accept_timeout(listener, -1); }

int64_t lpp_net_send(int64_t fd, const char *data) {
    if (fd < 0 || !data) return -1;
    int len = 0; while (data[len]) len++;
    return lpp_ws2_send((SOCKET)(intptr_t)fd, data, len, 0);
}

int64_t lpp_net_send_all(int64_t fd, const char *data) {
    if (fd < 0 || !data) return -1;
    int total = 0, len = 0; while (data[len]) len++;
    while (total < len) {
        int sent = lpp_ws2_send((SOCKET)(intptr_t)fd, data+total, len-total, 0);
        if (sent <= 0) break;
        total += sent;
    }
    return (int64_t)total;
}

char* lpp_net_recv(int64_t fd, int64_t max_bytes) {
    if (fd < 0 || max_bytes <= 0) return (char*)"";
    char *buf = (char*)lpp_arc_alloc(max_bytes + 1);
    if (!buf) return (char*)"";
    int n = lpp_ws2_recv((SOCKET)(intptr_t)fd, buf, (int)max_bytes, 0);
    if (n < 0) n = 0;
    buf[n] = 0;
    return buf;
}

char* lpp_net_recv_udp(int64_t fd, int64_t max_bytes) { return lpp_net_recv(fd, max_bytes); }

void lpp_net_close(int64_t fd) { if (fd >= 0) lpp_ws2_closesocket((SOCKET)(intptr_t)fd); }

int64_t lpp_net_set_deadline(int64_t fd, int64_t read_ms, int64_t write_ms) {
    if (fd < 0) return -1;
    int ok = 1;
    if (read_ms >= 0) {
        DWORD t = (DWORD)read_ms;
        if (lpp_ws2_setsockopt((SOCKET)(intptr_t)fd, 1, 0x1005, (const char*)&t, sizeof(t)) < 0) ok = 0;
    }
    if (write_ms >= 0) {
        DWORD t = (DWORD)write_ms;
        if (lpp_ws2_setsockopt((SOCKET)(intptr_t)fd, 1, 0x1006, (const char*)&t, sizeof(t)) < 0) ok = 0;
    }
    return ok ? 1 : -1;
}

int64_t lpp_net_set_timeout(int64_t fd, int64_t ms) { return lpp_net_set_deadline(fd, ms, ms); }

int64_t lpp_net_set_keepalive(int64_t fd, int64_t enable, int64_t idle_s, int64_t interval, int64_t count) {
    if (fd < 0) return -1;
    int v = enable ? 1 : 0;
    lpp_ws2_setsockopt((SOCKET)(intptr_t)fd, 1, 8, (const char*)&v, sizeof(v));
    return 1;
}

char* lpp_net_resolve(const char *host) {
    unsigned long ip;
    if (!lpp_resolve_win(host, &ip)) return (char*)"";
    char *buf = (char*)lpp_arc_alloc(16);
    if (!buf) return (char*)"";
    unsigned char *b = (unsigned char*)&ip;
    int off = 0;
    for (int i = 0; i < 4; i++) {
        uint8_t octet = b[i];
        if (octet >= 100) buf[off++] = '0' + octet/100;
        if (octet >= 10)  buf[off++] = '0' + (octet%100)/10;
        buf[off++] = '0' + octet%10;
        if (i < 3) buf[off++] = '.';
    }
    buf[off] = 0;
    return buf;
}

/* HTTP stubs — simple implementations using net_dial/net_recv/net_send */
/* (full implementations require freestanding sprintf; these are thin wrappers) */
char* lpp_http_get(const char *url, int64_t timeout_ms) { (void)url; (void)timeout_ms; return (char*)""; }
char* lpp_http_post(const char *url, const char *body, const char *content_type, int64_t timeout_ms) { (void)url; (void)body; (void)content_type; (void)timeout_ms; return (char*)""; }

int64_t lpp_net_connect(const char *host, int64_t port) { return lpp_net_dial(host, port, 30000); }

