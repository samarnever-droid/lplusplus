/* ── Borrowed zero-copy slices ──────────────────────────────────────────── */
typedef struct {
    void *base;
    int64_t start;
    int64_t length;
    int64_t generation;
    int64_t kind; /* 0 = UTF-8 byte string, 1 = List[T] slots */
} LppSlice;

static void *lpp_slice_checked_base(const LppSlice *view) {
    if (!view || !view->base || view->generation == 0) {
        lpp_panic("use of an uninitialized slice view");
    }
    int64_t raw = lpp_weak_get((int64_t)(intptr_t)view->base, view->generation);
    if (!raw) lpp_panic("borrowed slice source is no longer live");
    return (void *)(intptr_t)raw;
}

void *lpp_slice_init(void *storage, void *base, int64_t start, int64_t length, int64_t kind) {
    if (!storage || !base) lpp_panic("slice construction requires live storage and base");
    if (start < 0 || length < 0 || start > INT64_MAX - length) {
        lpp_panic("invalid slice range: start %lld, len %lld", (long long)start, (long long)length);
    }
    int64_t source_length = kind == 0
        ? (int64_t)strlen((const char *)base)
        : lpp_list_len(base);
    if (start > source_length || length > source_length - start) {
        lpp_panic("slice range out of bounds: start %lld, len %lld, source len %lld",
            (long long)start, (long long)length, (long long)source_length);
    }
    LppSlice *view = (LppSlice *)storage;
    view->base = base;
    view->start = start;
    view->length = length;
    view->generation = lpp_weak_generation(base);
    view->kind = kind;
    if (view->generation == 0) lpp_panic("slice source is not an ARC-managed value");
    return view;
}

int64_t lpp_slice_len(void *raw_view) {
    LppSlice *view = (LppSlice *)raw_view;
    (void)lpp_slice_checked_base(view);
    return view->length;
}

int64_t lpp_slice_get(void *raw_view, int64_t index) {
    LppSlice *view = (LppSlice *)raw_view;
    void *base = lpp_slice_checked_base(view);
    if (index < 0 || index >= view->length) {
        lpp_panic("slice index out of bounds: index %lld, len %lld",
            (long long)index, (long long)view->length);
    }
    if (view->kind != 1) lpp_panic("numeric slice_get requires Slice[T]");
    return lpp_list_get(base, view->start + index);
}

double lpp_slice_get_float(void *raw_view, int64_t index) {
    int64_t bits = lpp_slice_get(raw_view, index);
    double value;
    memcpy(&value, &bits, sizeof(value));
    return value;
}

int8_t lpp_slice_get_bool(void *raw_view, int64_t index) {
    return lpp_slice_get(raw_view, index) != 0;
}

char *lpp_str_slice_get(void *raw_view, int64_t index) {
    LppSlice *view = (LppSlice *)raw_view;
    const char *base = (const char *)lpp_slice_checked_base(view);
    if (view->kind != 0) lpp_panic("string slice_get requires StrSlice");
    if (index < 0 || index >= view->length) {
        lpp_panic("string slice index out of bounds: index %lld, len %lld",
            (long long)index, (long long)view->length);
    }
    char *result = (char *)lpp_arc_alloc(2);
    if (!result) lpp_panic("out of memory while reading string slice");
    result[0] = base[view->start + index];
    result[1] = '\0';
    return result;
}

char *lpp_str_slice_to_str(void *raw_view) {
    LppSlice *view = (LppSlice *)raw_view;
    const char *base = (const char *)lpp_slice_checked_base(view);
    if (view->kind != 0) lpp_panic("slice_to_str requires StrSlice");
    char *result = (char *)lpp_arc_alloc(view->length + 1);
    if (!result) lpp_panic("out of memory while copying string slice");
    memcpy(result, base + view->start, (size_t)view->length);
    result[view->length] = '\0';
    return result;
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
#include <fcntl.h>
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
    if (sock == LPP_INVALID_SOCKET) {
        #ifdef _WIN32
        printf("[Lreact Debug] socket() failed with error code: %d\n", WSAGetLastError());
        fflush(stdout);
        #endif
        return 0;
    }
    int yes = 1;
    setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, (const char *)&yes, sizeof(yes));
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_ANY);
    addr.sin_port = htons((unsigned short)port);
    if (bind(sock, (struct sockaddr *)&addr, sizeof(addr)) != 0 || listen(sock, 16) != 0) {
        #ifdef _WIN32
        printf("[Lreact Debug] socket bind error code: %d\n", WSAGetLastError());
        fflush(stdout);
        #endif
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

int64_t lpp_net_set_nonblocking(int64_t handle, int64_t enable) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET) return 0;
#ifdef _WIN32
    u_long mode = enable ? 1 : 0;
    return ioctlsocket(sock, FIONBIO, &mode) == 0 ? 1 : 0;
#else
    int flags = fcntl(sock, F_GETFL, 0);
    if (flags < 0) return 0;
    flags = enable ? (flags | O_NONBLOCK) : (flags & ~O_NONBLOCK);
    return fcntl(sock, F_SETFL, flags) == 0 ? 1 : 0;
#endif
}

int64_t lpp_net_poll(int64_t handle, int64_t timeout_ms) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET) return 0;
    fd_set fds;
    FD_ZERO(&fds);
    FD_SET(sock, &fds);
    struct timeval tv;
    tv.tv_sec = (long)(timeout_ms / 1000);
    tv.tv_usec = (long)((timeout_ms % 1000) * 1000);
    int res = select((int)(sock + 1), &fds, NULL, NULL, &tv);
    return res > 0 ? 1 : 0;
}

char *lpp_net_recv(int64_t handle, int64_t max_bytes) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET || max_bytes <= 0) {
        char *empty = (char *)lpp_arc_alloc(1);
        if (empty) empty[0] = '\0';
        return empty;
    }
    int size = (int)max_bytes;
    char *buf = (char *)lpp_arc_alloc((size_t)size + 1);
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
#if defined(_WIN32)
    shutdown(sock, SD_SEND);
#else
    shutdown(sock, SHUT_WR);
#endif
    lpp_close_socket(sock);
    lpp__socket_clear(handle);
}

/* ── Extended networking (net_dial, UDP, deadlines, keepalive, DNS, HTTP) ─── */

/* Results are handed to generated L++ code as `Str`, so they must carry an ARC
 * header like every other owned string. */
static char* lpp_net_strdup_impl(const char *s) {
    if (!s) return NULL;
    size_t len = strlen(s);
    char *d = (char *)lpp_arc_alloc((int64_t)(len + 1));
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
    /* Size the request to its actual contents instead of a fixed stack array:
     * snprintf would silently truncate a long path or host, emitting a
     * malformed request (and for POST, a Content-Length that no longer matches
     * the body actually sent). */
    size_t path_len = strlen(path);
    size_t host_len = strlen(host);
    if (path_len > (size_t)INT64_MAX || host_len > (size_t)INT64_MAX ||
        path_len + host_len + 160 < path_len) {   /* size_t overflow guard */
        lpp_net_close(fd);
        return lpp_net_strdup_impl("");
    }
    size_t need = path_len + host_len + 160;      /* request head + NUL */
    char *req = (char *)lpp_arc_alloc((int64_t)need);
    if (!req) { lpp_net_close(fd); return lpp_net_strdup_impl(""); }
    int n = snprintf(req, need,
        "GET %s HTTP/1.1\r\nHost: %s\r\nConnection: close\r\nAccept: */*\r\nUser-Agent: L++/0.1.3\r\n\r\n",
        path, host);
    if (n < 0 || (size_t)n >= need) { lpp_arc_release(req); lpp_net_close(fd); return lpp_net_strdup_impl(""); }
    lpp_net_send_all(fd, req);
    lpp_arc_release(req);
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
    size_t path_len = strlen(path);
    size_t host_len = strlen(host);
    size_t ct_len = strlen(content_type);
    size_t data_len = strlen(data);
    if (path_len > (size_t)INT64_MAX || host_len > (size_t)INT64_MAX ||
        ct_len > (size_t)INT64_MAX || data_len > (size_t)INT64_MAX ||
        path_len + host_len + ct_len + data_len + 320 < path_len) {
        lpp_net_close(fd);
        return lpp_net_strdup_impl("");
    }
    size_t need = path_len + host_len + ct_len + data_len + 320;
    char *req = (char *)lpp_arc_alloc((int64_t)need);
    if (!req) { lpp_net_close(fd); return lpp_net_strdup_impl(""); }
    int n = snprintf(req, need,
        "POST %s HTTP/1.1\r\nHost: %s\r\nContent-Type: %s\r\nContent-Length: %d\r\nConnection: close\r\nAccept: */*\r\nUser-Agent: L++/0.1.3\r\n\r\n%s",
        path, host, content_type, (int)data_len, data);
    if (n < 0 || (size_t)n >= need) { lpp_arc_release(req); lpp_net_close(fd); return lpp_net_strdup_impl(""); }
    lpp_net_send_all(fd, req);
    lpp_arc_release(req);
    char *body = lpp_net_recv(fd, 65536);
    lpp_net_close(fd);
    if (!body) return lpp_net_strdup_impl("");
    char *sep = strstr(body, "\r\n\r\n");
    if (sep) { sep += 4; char *r = lpp_net_strdup_impl(sep); free(body); return r; }
    return body;
}

#endif /* !LPP_NO_NETWORK */

/* Thread and synchronization primitives are implemented in runtime/lpp_concur.c */

