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
#if !defined(_WIN32) && !defined(_POSIX_C_SOURCE)
#  define _POSIX_C_SOURCE 200112L
#endif

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <limits.h>
#include <errno.h>

/* ── I/O ──────────────────────────────────────────────────────────────────── */

void lpp_print_int(int64_t value) {
    printf("%lld\n", (long long)value);
    fflush(stdout);
}

void lpp_print_float(double value) {
    printf("%f\n", value);
    fflush(stdout);
}

void lpp_print_str(const char *ptr) {
    if (ptr) { puts(ptr); fflush(stdout); }
}

/* Read one line from stdin (strips trailing newline).
   Returns a heap-allocated string; caller frees with lpp_free_str. */
char *lpp_input(void) {
    char buf[4096];
    if (!fgets(buf, sizeof(buf), stdin)) return NULL;
    size_t len = strlen(buf);
    if (len > 0 && buf[len - 1] == '\n') buf[--len] = '\0';
    char *result = (char *)malloc(len + 1);
    if (!result) return NULL;
    memcpy(result, buf, len + 1);
    return result;
}

void lpp_free_str(char *ptr) {
    free(ptr);
}

int64_t lpp_parse_int(const char *str) {
    if (!str || *str == '\0') {
        fprintf(stderr, "[L++ Runtime Error] Invalid integer format: empty string\n");
        exit(1);
    }
    
    // Skip leading whitespace
    const char *p = str;
    while (*p == ' ' || *p == '\t' || *p == '\r' || *p == '\n') {
        p++;
    }
    
    if (*p == '\0') {
        fprintf(stderr, "[L++ Runtime Error] Invalid integer format: \"%s\"\n", str);
        exit(1);
    }
    
    char *endptr;
    errno = 0;
    long long val = strtoll(p, &endptr, 10);
    
    // Check for overflow/underflow
    if (errno == ERANGE) {
        fprintf(stderr, "[L++ Runtime Error] Integer overflow/underflow: \"%s\" exceeds 64-bit limits\n", str);
        exit(1);
    }
    
    // Check for trailing garbage (invalid chars)
    while (*endptr == ' ' || *endptr == '\t' || *endptr == '\r' || *endptr == '\n') {
        endptr++;
    }
    if (*endptr != '\0') {
        fprintf(stderr, "[L++ Runtime Error] Invalid integer format: \"%s\"\n", str);
        exit(1);
    }
    
    return (int64_t)val;
}

/* ── File I/O ─────────────────────────────────────────────────────────────── */

/* Read entire file contents. Returns heap-allocated string or NULL on error. */
char *lpp_read_file(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (size < 0) { fclose(f); return NULL; }
    char *buf = (char *)malloc((size_t)size + 1);
    if (!buf) { fclose(f); return NULL; }
    size_t wanted = (size_t)size;
    size_t read = fread(buf, 1, wanted, f);
    if (read != wanted && ferror(f)) {
        free(buf);
        fclose(f);
        return NULL;
    }
    /* A short read at EOF is valid; return precisely the bytes obtained. */
    buf[read] = '\0';
    fclose(f);
    return buf;
}

/* Write data to file. Returns 0 on success, -1 on error. */
int64_t lpp_write_file(const char *path, const char *data) {
    FILE *f = fopen(path, "wb");
    if (!f) return -1;
    size_t len = data ? strlen(data) : 0;
    fwrite(data, 1, len, f);
    fclose(f);
    return 0;
}

/* Append data to file. Returns 0 on success, -1 on error. */
int64_t lpp_append_file(const char *path, const char *data) {
    FILE *f = fopen(path, "ab");
    if (!f) return -1;
    size_t len = data ? strlen(data) : 0;
    fwrite(data, 1, len, f);
    fclose(f);
    return 0;
}

/* Delete file. Returns 0 on success, -1 on error. */
int64_t lpp_delete_file(const char *path) {
    if (!path) return -1;
    return remove(path) == 0 ? 0 : -1;
}

/* Check if file exists. Returns 1 if exists, 0 if not. */
int8_t lpp_file_exists(const char *path) {
    if (!path) return 0;
    FILE *f = fopen(path, "rb");
    if (f) {
        fclose(f);
        return 1;
    }
    return 0;
}

/* Returns file length in bytes, or -1 for an invalid/unreadable path. */
int64_t lpp_file_size(const char *path) {
    if (!path) return -1;
    FILE *f = fopen(path, "rb");
    if (!f) return -1;
    if (fseek(f, 0, SEEK_END) != 0) { fclose(f); return -1; }
    long size = ftell(f);
    fclose(f);
    return size < 0 ? -1 : (int64_t)size;
}

/* Copies through a bounded buffer, checking every read/write/close result. */
int64_t lpp_file_copy(const char *source, const char *destination) {
    if (!source || !destination) return -1;
    FILE *in = fopen(source, "rb");
    if (!in) return -1;
    FILE *out = fopen(destination, "wb");
    if (!out) { fclose(in); return -1; }
    unsigned char buffer[8192];
    int failed = 0;
    for (;;) {
        size_t read = fread(buffer, 1, sizeof(buffer), in);
        if (read && fwrite(buffer, 1, read, out) != read) { failed = 1; break; }
        if (read < sizeof(buffer)) { if (ferror(in)) failed = 1; break; }
    }
    if (fclose(in) != 0 || fclose(out) != 0) failed = 1;
    if (failed) { remove(destination); return -1; }
    return 0;
}

/* Rename is atomic on a single filesystem on POSIX and delegates to the host
 * CRT on Windows. It never reports success when rename itself failed. */
int64_t lpp_file_move(const char *source, const char *destination) {
    if (!source || !destination) return -1;
    return rename(source, destination) == 0 ? 0 : -1;
}


/* ── ARC (Automatic Reference Counting) ──────────────────────────────────── */
/*
 * Layout: every ARC-managed object is preceded in memory by an LppArcHeader.
 * lpp_arc_alloc(size) allocates  sizeof(LppArcHeader) + size  bytes, inits
 * the refcount to 1, and returns a pointer to the byte immediately after the
 * header (i.e. to the user-visible payload).  Retain/release operate on the
 * hidden header that sits sizeof(LppArcHeader) bytes before the user pointer.
 *
 * Atomic ops use C11 stdatomic on GCC/Clang and MSVC interlocked on Windows.
 */

#if defined(_MSC_VER)
#  define WIN32_LEAN_AND_MEAN
#  include <windows.h>
   typedef volatile LONG lpp_atomic32_t;
#  define LPP_ARC_LOAD(p)         ((int32_t)InterlockedAdd((p), 0))
#  define LPP_ARC_INC(p)          InterlockedIncrement((p))
#  define LPP_ARC_DEC(p)          InterlockedDecrement((p))
#else
#  include <stdatomic.h>
   typedef _Atomic(int32_t) lpp_atomic32_t;
#  define LPP_ARC_LOAD(p)         atomic_load_explicit((p), memory_order_acquire)
#  define LPP_ARC_INC(p)          atomic_fetch_add_explicit((p), 1, memory_order_acq_rel)
#  define LPP_ARC_DEC(p)          atomic_fetch_sub_explicit((p), 1, memory_order_acq_rel)
#endif

typedef void (*LppArcDestructor)(void *payload);

typedef struct {
    lpp_atomic32_t refcount;
    /* Called exactly once, immediately before the payload is freed. */
    LppArcDestructor destructor;
} LppArcHeader;

/* Allocate an ARC object with an optional type-specific destructor. */
void *lpp_arc_alloc_with_destructor(int64_t size, LppArcDestructor destructor) {
    LppArcHeader *hdr = (LppArcHeader *)calloc(1, sizeof(LppArcHeader) + (size_t)size);
    if (!hdr) return NULL;
#if defined(_MSC_VER)
    hdr->refcount = 1;
#else
    atomic_init(&hdr->refcount, 1);
#endif
    hdr->destructor = destructor;
    return (void *)(hdr + 1); /* return pointer to payload, past the header */
}

/* Backwards-compatible allocation for runtime values with no child owners. */
void *lpp_arc_alloc(int64_t size) {
    return lpp_arc_alloc_with_destructor(size, NULL);
}

/* Increment the reference count. Safe to call with NULL. */
void lpp_arc_retain(void *ptr) {
    if (!ptr) return;
    LppArcHeader *hdr = (LppArcHeader *)ptr - 1;
    LPP_ARC_INC(&hdr->refcount);
}

/* Decrement the reference count. Free when it reaches zero. */
void lpp_arc_release(void *ptr) {
    if (!ptr) return;
    LppArcHeader *hdr = (LppArcHeader *)ptr - 1;
    int32_t prev = (int32_t)LPP_ARC_DEC(&hdr->refcount);
    if (prev == 1) {
        /* Refcount just hit zero. Destroy owned child references before the
         * payload/header are released; child releases may recursively invoke
         * their own generated destructors. */
        if (hdr->destructor) hdr->destructor(ptr);
        free(hdr);
    }
}

/* An ARC-managed closure payload is two pointer-sized words:
 * [code pointer, environment pointer].  The code pointer is non-owning; the
 * environment is an owned ARC reference transferred into the closure. */
void lpp_closure_destroy(void *closure) {
    if (!closure) return;
    void **parts = (void **)closure;
    lpp_arc_release(parts[1]);
}

/* ── Allocator ───────────────────────────────────────────────────────────── */

void *lpp_alloc(int64_t size) {
    return calloc(1, (size_t)size);
}

void lpp_free(void *ptr, int64_t size) {
    (void)size;
    free(ptr);
}

/* ── List<Int> ───────────────────────────────────────────────────────────── */

typedef void (*LppListElementFn)(int64_t value);

typedef struct {
    int64_t *data;
    int64_t  len;
    int64_t  cap;
    /* NULL for value elements; retain/drop callbacks for ARC pointer elements. */
    LppListElementFn retain_element;
    LppListElementFn drop_element;
} LppList;

static void lpp_list_arc_retain_element(int64_t value) {
    lpp_arc_retain((void *)(intptr_t)value);
}

static void lpp_list_arc_drop_element(int64_t value) {
    lpp_arc_release((void *)(intptr_t)value);
}

static void lpp_list_destroy(void *payload) {
    LppList *l = (LppList *)payload;
    if (!l) return;
    if (l->drop_element) {
        for (int64_t i = 0; i < l->len; ++i) {
            l->drop_element(l->data[i]);
        }
    }
    free(l->data);
    l->data = NULL;
    l->len = 0;
    l->cap = 0;
}

static void *lpp_list_new_with_ownership(
    LppListElementFn retain_element,
    LppListElementFn drop_element
) {
    LppList *l = (LppList *)lpp_arc_alloc_with_destructor(
        (int64_t)sizeof(LppList), lpp_list_destroy
    );
    if (!l) {
        fprintf(stderr, "[L++ Runtime Error] out of memory while creating list\n");
        abort();
    }
    l->retain_element = retain_element;
    l->drop_element = drop_element;
    return l;
}

/* List[Int] stores values and owns no element references. */
void *lpp_list_new(void) {
    return lpp_list_new_with_ownership(NULL, NULL);
}

/* List[ARC Object] owns one retained reference per element. */
void *lpp_list_new_arc(void) {
    return lpp_list_new_with_ownership(
        lpp_list_arc_retain_element,
        lpp_list_arc_drop_element
    );
}

void lpp_list_push(void *list, int64_t value) {
    LppList *l = (LppList *)list;
    if (!l) {
        fprintf(stderr, "[L++ Runtime Error] push to null list\n");
        abort();
    }
    if (l->len == l->cap) {
        if (l->cap > INT64_MAX / 2) {
            fprintf(stderr, "[L++ Runtime Error] list capacity overflow\n");
            abort();
        }
        int64_t new_cap = l->cap == 0 ? 8 : l->cap * 2;
        if (new_cap > INT64_MAX / (int64_t)sizeof(int64_t)) {
            fprintf(stderr, "[L++ Runtime Error] list allocation size overflow\n");
            abort();
        }
        int64_t *new_data = (int64_t *)realloc(l->data, (size_t)new_cap * sizeof(int64_t));
        if (!new_data) {
            fprintf(stderr, "[L++ Runtime Error] out of memory while growing list\n");
            abort();
        }
        l->data = new_data;
        l->cap = new_cap;
    }
    if (l->retain_element) l->retain_element(value);
    l->data[l->len++] = value;
}

/* Store one ARC object reference in List[T]. */
void lpp_list_push_arc(void *list, void *value) {
    lpp_list_push(list, (int64_t)(intptr_t)value);
}

int64_t lpp_list_get(void *list, int64_t index) {
    LppList *l = (LppList *)list;
    if (!l || index < 0 || index >= l->len) {
        fprintf(stderr, "[L++ Runtime Error] list index out of bounds: %lld\n", (long long)index);
        abort();
    }
    return l->data[index];
}

/* List element reads are borrowed; callers retain only when they create an
 * additional owner (assignment/return/store). */
void *lpp_list_get_arc(void *list, int64_t index) {
    return (void *)(intptr_t)lpp_list_get(list, index);
}

int64_t lpp_list_len(void *list) {
    LppList *l = (LppList *)list;
    return l ? l->len : 0;
}

void lpp_list_free(void *list) {
    /* Compatibility entry point. In ownership-aware AOT code list lifetime is
     * automatic, so this is only a single reference release, never raw free. */
    lpp_arc_release(list);
}

#if !defined(LPP_NO_NETWORK)
/* Network sockets */
#if defined(_WIN32)
#if defined(_MSC_VER)
#pragma comment(lib, "Ws2_32.lib")
#endif
#include <winsock2.h>
#include <ws2tcpip.h>
typedef SOCKET lpp_socket_t;
#define LPP_INVALID_SOCKET INVALID_SOCKET
#define lpp_close_socket closesocket
static int lpp__net_started = 0;
static void lpp__net_init(void) {
    if (!lpp__net_started) {
        WSADATA wsa;
        if (WSAStartup(MAKEWORD(2, 2), &wsa) == 0) lpp__net_started = 1;
    }
}
#else
#include <sys/types.h>
#include <sys/socket.h>
#include <netdb.h>
#include <arpa/inet.h>
#include <unistd.h>
#include <sys/time.h>
typedef int lpp_socket_t;
#define LPP_INVALID_SOCKET (-1)
#define lpp_close_socket close
static void lpp__net_init(void) {}
#endif

static lpp_socket_t lpp__socket_table[256];

static int64_t lpp__socket_store(lpp_socket_t sock) {
    for (int64_t i = 0; i < 256; ++i) {
        if (lpp__socket_table[i] == 0 || lpp__socket_table[i] == LPP_INVALID_SOCKET) {
            lpp__socket_table[i] = sock;
            return i + 1;
        }
    }
    return 0;
}

static lpp_socket_t lpp__socket_load(int64_t handle) {
    if (handle <= 0 || handle > 256) return LPP_INVALID_SOCKET;
    return lpp__socket_table[handle - 1];
}

static void lpp__socket_clear(int64_t handle) {
    if (handle > 0 && handle <= 256) lpp__socket_table[handle - 1] = 0;
}

int64_t lpp_net_connect(const char *host, int64_t port) {
    lpp__net_init();
    if (!host) return 0;
    char port_buf[32];
    snprintf(port_buf, sizeof(port_buf), "%lld", (long long)port);
    struct addrinfo hints, *result = NULL, *rp = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, port_buf, &hints, &result) != 0) return 0;
    lpp_socket_t sock = LPP_INVALID_SOCKET;
    for (rp = result; rp; rp = rp->ai_next) {
        sock = (lpp_socket_t)socket(rp->ai_family, rp->ai_socktype, rp->ai_protocol);
        if (sock == LPP_INVALID_SOCKET) continue;
        if (connect(sock, rp->ai_addr, (int)rp->ai_addrlen) == 0) break;
        lpp_close_socket(sock);
        sock = LPP_INVALID_SOCKET;
    }
    freeaddrinfo(result);
    if (sock == LPP_INVALID_SOCKET) return 0;
    return lpp__socket_store(sock);
}

int64_t lpp_net_listen(int64_t port) {
    lpp__net_init();
    lpp_socket_t sock = (lpp_socket_t)socket(AF_INET, SOCK_STREAM, 0);
    if (sock == LPP_INVALID_SOCKET) return 0;
    int yes = 1;
    setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, (const char *)&yes, sizeof(yes));
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_ANY);
    addr.sin_port = htons((unsigned short)port);
    if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) != 0 || listen(sock, 16) != 0) {
        lpp_close_socket(sock);
        return 0;
    }
    return lpp__socket_store(sock);
}

int64_t lpp_net_accept(int64_t listener) {
    lpp_socket_t server = lpp__socket_load(listener);
    if (server == LPP_INVALID_SOCKET) return 0;
    lpp_socket_t client = accept(server, NULL, NULL);
    if (client == LPP_INVALID_SOCKET) return 0;
    return lpp__socket_store(client);
}

/* Write the complete NUL-terminated L++ string. A successful send(2) is
 * permitted to write fewer bytes than requested; exposing that behaviour as a
 * successful protocol write corrupts HTTP and framed protocols. */
int64_t lpp_net_send_all(int64_t handle, const char *data) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET || !data) return -1;
    size_t length = strlen(data);
    size_t sent_total = 0;
    while (sent_total < length) {
        size_t remaining = length - sent_total;
#ifdef _WIN32
        int chunk = remaining > (size_t)INT_MAX ? INT_MAX : (int)remaining;
        int sent = send(sock, data + sent_total, chunk, 0);
#else
        int flags = 0;
# ifdef MSG_NOSIGNAL
        flags |= MSG_NOSIGNAL;
# endif
        ssize_t sent = send(sock, data + sent_total, remaining, flags);
#endif
        if (sent <= 0) return -1;
        sent_total += (size_t)sent;
    }
    return (int64_t)sent_total;
}

/* net_send keeps its original API but now has complete-write semantics. */
int64_t lpp_net_send(int64_t handle, const char *data) {
    return lpp_net_send_all(handle, data);
}

int64_t lpp_net_set_timeout(int64_t handle, int64_t milliseconds) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET || milliseconds <= 0) return 0;
#ifdef _WIN32
    DWORD timeout = milliseconds > 0xFFFFFFFFLL ? (DWORD)0xFFFFFFFFUL : (DWORD)milliseconds;
    return setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, (const char *)&timeout, sizeof(timeout)) == 0
        && setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, (const char *)&timeout, sizeof(timeout)) == 0;
#else
    struct timeval timeout;
    timeout.tv_sec = (time_t)(milliseconds / 1000);
    timeout.tv_usec = (suseconds_t)((milliseconds % 1000) * 1000);
    return setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout)) == 0
        && setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout)) == 0;
#endif
}

char *lpp_net_recv(int64_t handle, int64_t max_bytes) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET || max_bytes <= 0) {
        char *empty = (char *)malloc(1);
        if (empty) empty[0] = '\0';
        return empty;
    }
    int size = (int)max_bytes;
    char *buf = (char *)malloc((size_t)size + 1);
    if (!buf) return NULL;
    int received = recv(sock, buf, size, 0);
    if (received <= 0) {
        buf[0] = '\0';
        return buf;
    }
    buf[received] = '\0';
    return buf;
}

void lpp_net_close(int64_t handle) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET) return;
    lpp_close_socket(sock);
    lpp__socket_clear(handle);
}

/* ── Extended networking (net_dial, UDP, deadlines, keepalive, DNS, HTTP) ─── */

static char* lpp_net_strdup_impl(const char *s) {
    if (!s) return NULL;
    size_t len = strlen(s);
    char *d = (char *)malloc(len + 1);
    if (d) { memcpy(d, s, len); d[len] = 0; }
    return d;
}

int64_t lpp_net_dial(const char *host, int64_t port, int64_t timeout_ms) {
    lpp__net_init();
    if (!host || port < 1 || port > 65535) return 0;
    char port_buf[32]; snprintf(port_buf, sizeof(port_buf), "%lld", (long long)port);
    struct addrinfo hints, *result = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC; hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, port_buf, &hints, &result) != 0) return 0;
    lpp_socket_t sock = LPP_INVALID_SOCKET;
    struct addrinfo *rp;
    for (rp = result; rp; rp = rp->ai_next) {
        sock = (lpp_socket_t)socket(rp->ai_family, rp->ai_socktype, rp->ai_protocol);
        if (sock == LPP_INVALID_SOCKET) continue;
        if (connect(sock, rp->ai_addr, (int)rp->ai_addrlen) == 0) break;
        lpp_close_socket(sock); sock = LPP_INVALID_SOCKET;
    }
    freeaddrinfo(result);
    if (sock == LPP_INVALID_SOCKET) return 0;
    int64_t handle = lpp__socket_store(sock);
    if (timeout_ms > 0) lpp_net_set_timeout(handle, timeout_ms);
    return handle;
}

int64_t lpp_net_dial_udp(const char *host, int64_t port, int64_t timeout_ms) {
    lpp__net_init();
    if (!host || port < 1 || port > 65535) return 0;
    char port_buf[32]; snprintf(port_buf, sizeof(port_buf), "%lld", (long long)port);
    struct addrinfo hints, *result = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC; hints.ai_socktype = SOCK_DGRAM;
    if (getaddrinfo(host, port_buf, &hints, &result) != 0) return 0;
    lpp_socket_t sock = LPP_INVALID_SOCKET;
    struct addrinfo *rp;
    for (rp = result; rp; rp = rp->ai_next) {
        sock = (lpp_socket_t)socket(rp->ai_family, rp->ai_socktype, rp->ai_protocol);
        if (sock == LPP_INVALID_SOCKET) continue;
        if (connect(sock, rp->ai_addr, (int)rp->ai_addrlen) == 0) break;
        lpp_close_socket(sock); sock = LPP_INVALID_SOCKET;
    }
    freeaddrinfo(result);
    if (sock == LPP_INVALID_SOCKET) return 0;
    int64_t handle = lpp__socket_store(sock);
    if (timeout_ms > 0) lpp_net_set_timeout(handle, timeout_ms);
    return handle;
}

int64_t lpp_net_listen_udp(int64_t port) {
    lpp__net_init();
    lpp_socket_t sock = (lpp_socket_t)socket(AF_INET, SOCK_DGRAM, 0);
    if (sock == LPP_INVALID_SOCKET) return 0;
    struct sockaddr_in addr; memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET; addr.sin_addr.s_addr = htonl(INADDR_ANY);
    addr.sin_port = htons((unsigned short)port);
    if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) != 0) { lpp_close_socket(sock); return 0; }
    return lpp__socket_store(sock);
}

int64_t lpp_net_accept_timeout(int64_t listener, int64_t timeout_ms) {
    lpp_socket_t server = lpp__socket_load(listener);
    if (server == LPP_INVALID_SOCKET) return 0;
    if (timeout_ms > 0) {
#ifdef _WIN32
        DWORD t = (DWORD)timeout_ms;
        setsockopt(server, SOL_SOCKET, SO_RCVTIMEO, (const char*)&t, sizeof(t));
#else
        struct timeval tv; tv.tv_sec = (time_t)(timeout_ms/1000); tv.tv_usec = (suseconds_t)((timeout_ms%1000)*1000);
        setsockopt(server, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv));
#endif
    }
    lpp_socket_t client = accept(server, NULL, NULL);
    if (client == LPP_INVALID_SOCKET) return 0;
    return lpp__socket_store(client);
}

int64_t lpp_net_set_deadline(int64_t fd, int64_t read_ms, int64_t write_ms) {
    lpp_socket_t sock = lpp__socket_load(fd);
    if (sock == LPP_INVALID_SOCKET) return 0;
    int ok = 1;
    if (read_ms >= 0) {
#ifdef _WIN32
        DWORD t = (DWORD)read_ms;
        if (setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, (const char*)&t, sizeof(t)) < 0) ok = 0;
#else
        struct timeval tv; tv.tv_sec = (time_t)(read_ms/1000); tv.tv_usec = (suseconds_t)((read_ms%1000)*1000);
        if (setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, &tv, sizeof(tv)) < 0) ok = 0;
#endif
    }
    if (write_ms >= 0) {
#ifdef _WIN32
        DWORD t = (DWORD)write_ms;
        if (setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, (const char*)&t, sizeof(t)) < 0) ok = 0;
#else
        struct timeval tv; tv.tv_sec = (time_t)(write_ms/1000); tv.tv_usec = (suseconds_t)((write_ms%1000)*1000);
        if (setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, &tv, sizeof(tv)) < 0) ok = 0;
#endif
    }
    return ok;
}

int64_t lpp_net_set_keepalive(int64_t fd, int64_t enable, int64_t idle_s, int64_t interval, int64_t count) {
    (void)idle_s; (void)interval; (void)count;
    lpp_socket_t sock = lpp__socket_load(fd);
    if (sock == LPP_INVALID_SOCKET) return 0;
    int v = enable ? 1 : 0;
    return setsockopt(sock, SOL_SOCKET, SO_KEEPALIVE, (const char*)&v, sizeof(v)) == 0;
}

char* lpp_net_resolve(const char *host) {
    if (!host || !*host) return lpp_net_strdup_impl("");
    struct addrinfo hints, *result = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_INET; hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, NULL, &hints, &result) != 0) return lpp_net_strdup_impl("");
    char ip[INET_ADDRSTRLEN]; ip[0] = 0;
    if (result && result->ai_addr) {
        struct sockaddr_in *addr = (struct sockaddr_in *)result->ai_addr;
        inet_ntop(AF_INET, &addr->sin_addr, ip, sizeof(ip));
        freeaddrinfo(result);
        return lpp_net_strdup_impl(ip);
    }
    if (result) freeaddrinfo(result);
    return lpp_net_strdup_impl("");
}

char* lpp_net_recv_udp(int64_t fd, int64_t max_bytes) { return lpp_net_recv(fd, max_bytes); }

char* lpp_http_get(const char *url, int64_t timeout_ms) {
    if (!url) return lpp_net_strdup_impl("");
    const char *p = url;
    if (strncmp(p, "http://", 7) == 0) p += 7;
    else return lpp_net_strdup_impl("");
    char host[256]; int hl = 0;
    while (*p && *p != ':' && *p != '/' && hl < 255) host[hl++] = *p++;
    host[hl] = 0;
    int port = 80;
    if (*p == ':') { p++; port = atoi(p); while (*p >= '0' && *p <= '9') p++; }
    const char *path = (*p == '/') ? p : "/";
    if (!*path) path = "/";
    int64_t fd = lpp_net_dial(host, (int64_t)port, timeout_ms);
    if (fd <= 0) return lpp_net_strdup_impl("");
    char req[2048];
    snprintf(req, sizeof(req), "GET %s HTTP/1.1\r\nHost: %s\r\nConnection: close\r\nAccept: */*\r\nUser-Agent: L++/0.1.3\r\n\r\n", path, host);
    lpp_net_send_all(fd, req);
    char *body = lpp_net_recv(fd, 65536);
    lpp_net_close(fd);
    if (!body) return lpp_net_strdup_impl("");
    char *sep = strstr(body, "\r\n\r\n");
    if (sep) { sep += 4; char *r = lpp_net_strdup_impl(sep); free(body); return r; }
    return body;
}

char* lpp_http_post(const char *url, const char *data, const char *content_type, int64_t timeout_ms) {
    if (!url) return lpp_net_strdup_impl("");
    const char *p = url;
    if (strncmp(p, "http://", 7) == 0) p += 7;
    else return lpp_net_strdup_impl("");
    char host[256]; int hl = 0;
    while (*p && *p != ':' && *p != '/' && hl < 255) host[hl++] = *p++;
    host[hl] = 0;
    int port = 80;
    if (*p == ':') { p++; port = atoi(p); while (*p >= '0' && *p <= '9') p++; }
    const char *path = (*p == '/') ? p : "/";
    if (!*path) path = "/";
    if (!data) data = "";
    if (!content_type) content_type = "application/x-www-form-urlencoded";
    int64_t fd = lpp_net_dial(host, (int64_t)port, timeout_ms);
    if (fd <= 0) return lpp_net_strdup_impl("");
    char req[4096];
    snprintf(req, sizeof(req),
        "POST %s HTTP/1.1\r\nHost: %s\r\nContent-Type: %s\r\nContent-Length: %d\r\nConnection: close\r\nAccept: */*\r\nUser-Agent: L++/0.1.3\r\n\r\n%s",
        path, host, content_type, (int)strlen(data), data);
    lpp_net_send_all(fd, req);
    char *body = lpp_net_recv(fd, 65536);
    lpp_net_close(fd);
    if (!body) return lpp_net_strdup_impl("");
    char *sep = strstr(body, "\r\n\r\n");
    if (sep) { sep += 4; char *r = lpp_net_strdup_impl(sep); free(body); return r; }
    return body;
}

#endif /* !LPP_NO_NETWORK */

/* ── Thread (minimal) ────────────────────────────────────────────────────── */

#if defined(_WIN32)
#include <windows.h>
typedef struct { void (*fn)(void*); void *env; } ThreadArg;
static DWORD WINAPI thread_trampoline(LPVOID arg) {
    ThreadArg *a = (ThreadArg *)arg;
    a->fn(a->env);
    free(a);
    return 0;
}
void lpp_thread_spawn(void (*fn)(void*), void *env) {
    ThreadArg *a = (ThreadArg *)malloc(sizeof(ThreadArg));
    a->fn = fn; a->env = env;
    CreateThread(NULL, 0, thread_trampoline, a, 0, NULL);
}
#else
#include <pthread.h>
typedef struct { void (*fn)(void*); void *env; } ThreadArg;
static void *thread_trampoline(void *arg) {
    ThreadArg *a = (ThreadArg *)arg;
    a->fn(a->env);
    free(a);
    return NULL;
}
void lpp_thread_spawn(void (*fn)(void*), void *env) {
    ThreadArg *a = (ThreadArg *)malloc(sizeof(ThreadArg));
    a->fn = fn; a->env = env;
    pthread_t t; pthread_create(&t, NULL, thread_trampoline, a);
    pthread_detach(t);
}
#endif

/* ── JSON Parser and Accessors (Builtin Standard Library) ────────────────── */

typedef struct lpp_JsonNode {
    char *key;
    int type; // 0=int, 1=str, 2=obj
    union {
        int64_t int_val;
        char *str_val;
        struct lpp_JsonNode *obj_val;
    } value;
    struct lpp_JsonNode *next;
} lpp_JsonNode;

static void skip_json_ws(const char **p) {
    while (**p == ' ' || **p == '\t' || **p == '\r' || **p == '\n') {
        (*p)++;
    }
}

static char *parse_json_string(const char **p) {
    skip_json_ws(p);
    if (**p != '"') return NULL;
    (*p)++; // skip '"'
    const char *start = *p;
    while (**p && **p != '"') {
        (*p)++;
    }
    size_t len = *p - start;
    char *res = malloc(len + 1);
    memcpy(res, start, len);
    res[len] = '\0';
    if (**p == '"') (*p)++; // skip '"'
    return res;
}

static lpp_JsonNode *parse_json_object(const char **p);

static lpp_JsonNode *parse_json_value(const char **p) {
    skip_json_ws(p);
    if (**p == '{') {
        return parse_json_object(p);
    } else if (**p == '"') {
        char *s = parse_json_string(p);
        lpp_JsonNode *n = calloc(1, sizeof(lpp_JsonNode));
        n->type = 1;
        n->value.str_val = s;
        return n;
    } else if ((**p >= '0' && **p <= '9') || **p == '-') {
        char *end;
        long long val = strtoll(*p, &end, 10);
        *p = end;
        lpp_JsonNode *n = calloc(1, sizeof(lpp_JsonNode));
        n->type = 0;
        n->value.int_val = (int64_t)val;
        return n;
    }
    return NULL;
}

static lpp_JsonNode *parse_json_object(const char **p) {
    skip_json_ws(p);
    if (**p != '{') return NULL;
    (*p)++; // skip '{'
    
    lpp_JsonNode *head = NULL;
    lpp_JsonNode *tail = NULL;
    
    while (**p && **p != '}') {
        skip_json_ws(p);
        if (**p == '}') break;
        char *key = parse_json_string(p);
        skip_json_ws(p);
        if (**p != ':') {
            free(key);
            break;
        }
        (*p)++; // skip ':'
        lpp_JsonNode *val = parse_json_value(p);
        if (val) {
            val->key = key;
            if (!head) {
                head = val;
                tail = val;
            } else {
                tail->next = val;
                tail = val;
            }
        } else {
            free(key);
        }
        skip_json_ws(p);
        if (**p == ',') {
            (*p)++; // skip ','
        } else if (**p != '}') {
            break;
        }
    }
    if (**p == '}') (*p)++; // skip '}'
    
    lpp_JsonNode *n = calloc(1, sizeof(lpp_JsonNode));
    n->type = 2;
    n->value.obj_val = head;
    return n;
}

void *lpp_json_parse(const char *str) {
    if (!str) return NULL;
    const char *p = str;
    return parse_json_value(&p);
}

int64_t lpp_json_get_int(void *json, const char *key) {
    lpp_JsonNode *node = (lpp_JsonNode *)json;
    if (!node) return 0;
    if (node->type == 2) {
        lpp_JsonNode *curr = node->value.obj_val;
        while (curr) {
            if (curr->key && strcmp(curr->key, key) == 0) {
                if (curr->type == 0) return curr->value.int_val;
                return 0;
            }
            curr = curr->next;
        }
    }
    return 0;
}

const char *lpp_json_get_str(void *json, const char *key) {
    lpp_JsonNode *node = (lpp_JsonNode *)json;
    if (!node) return "";
    if (node->type == 2) {
        lpp_JsonNode *curr = node->value.obj_val;
        while (curr) {
            if (curr->key && strcmp(curr->key, key) == 0) {
                if (curr->type == 1) return curr->value.str_val ? curr->value.str_val : "";
                return "";
            }
            curr = curr->next;
        }
    }
    return "";
}

void *lpp_json_get_obj(void *json, const char *key) {
    lpp_JsonNode *node = (lpp_JsonNode *)json;
    if (!node) return NULL;
    if (node->type == 2) {
        lpp_JsonNode *curr = node->value.obj_val;
        while (curr) {
            if (curr->key && strcmp(curr->key, key) == 0) {
                if (curr->type == 2) return curr;
                return NULL;
            }
            curr = curr->next;
        }
    }
    return NULL;
}

static void lpp_json_free_node(lpp_JsonNode *node) {
    if (!node) return;
    if (node->key) free(node->key);
    if (node->type == 1) {
        if (node->value.str_val) free(node->value.str_val);
    } else if (node->type == 2) {
        lpp_JsonNode *curr = node->value.obj_val;
        while (curr) {
            lpp_JsonNode *next = curr->next;
            lpp_json_free_node(curr);
            curr = next;
        }
    }
    free(node);
}

void lpp_json_free(void *json) {
    lpp_json_free_node((lpp_JsonNode *)json);
}

#include "runtime/lpp_str.c"
#include "runtime/lpp_exec.c"
#include "runtime/lpp_dir.c"
#include "runtime/lpp_buf.c"

