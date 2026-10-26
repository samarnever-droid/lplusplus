/* ── JSON Stubs for Freestanding ── */
int64_t lpp_json_parse(const char *json) { (void)json; return 0; }
char *lpp_json_get_str(int64_t handle, const char *key) { (void)handle; (void)key; return ""; }
int64_t lpp_json_get_int(int64_t handle, const char *key) { (void)handle; (void)key; return 0; }
double lpp_json_get_float(int64_t handle, const char *key) { (void)handle; (void)key; return 0.0; }
char *lpp_json_stringify(int64_t handle) { (void)handle; return "{}"; }
void lpp_json_free(int64_t handle) { (void)handle; }
static int lpp_wsa_started = 0;

static int lpp_net_startup(void) {
    if (lpp_wsa_started) return 1;
    WSADATA data;
    if (WSAStartup(MAKEWORD(2, 2), &data) != 0) return 0;
    lpp_wsa_started = 1;
    return 1;
}

int64_t lpp_net_listen(int64_t port) {
    if (!lpp_net_startup() || port <= 0 || port > 65535) return -1;
    SOCKET fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (fd == INVALID_SOCKET) return -1;
    BOOL reuse = TRUE;
    setsockopt(fd, SOL_SOCKET, SO_REUSEADDR, (const char*)&reuse, sizeof(reuse));
    struct sockaddr_in addr = {0};
    addr.sin_family = AF_INET;
    addr.sin_port = htons((u_short)port);
    addr.sin_addr.s_addr = inet_addr("127.0.0.1");
    if (bind(fd, (struct sockaddr*)&addr, sizeof(addr)) == SOCKET_ERROR || listen(fd, 128) == SOCKET_ERROR) {
        closesocket(fd);
        return -1;
    }
    return (int64_t)(uintptr_t)fd;
}

int64_t lpp_net_accept(int64_t listener) {
    if (listener < 0) return -1;
    SOCKET client = accept((SOCKET)(uintptr_t)listener, NULL, NULL);
    return client == INVALID_SOCKET ? -1 : (int64_t)(uintptr_t)client;
}

int64_t lpp_net_connect(const char *host, int64_t port) {
    if (!lpp_net_startup() || !host || port <= 0 || port > 65535) return -1;
    SOCKET fd = socket(AF_INET, SOCK_STREAM, IPPROTO_TCP);
    if (fd == INVALID_SOCKET) return -1;
    struct sockaddr_in addr = {0};
    addr.sin_family = AF_INET;
    addr.sin_port = htons((u_short)port);
    addr.sin_addr.s_addr = inet_addr(host);
    if (addr.sin_addr.s_addr == INADDR_NONE && lpp_strcmp(host, "255.255.255.255") != 0) {
        closesocket(fd);
        return -1;
    }
    if (connect(fd, (struct sockaddr*)&addr, sizeof(addr)) == SOCKET_ERROR) {
        closesocket(fd);
        return -1;
    }
    return (int64_t)(uintptr_t)fd;
}

int64_t lpp_net_send(int64_t socket_handle, const char *data) {
    if (socket_handle < 0 || !data) return -1;
    int len = lpp_strlen(data);
    return (int64_t)send((SOCKET)(uintptr_t)socket_handle, data, len, 0);
}

char *lpp_net_recv(int64_t socket_handle, int64_t max_bytes) {
    if (socket_handle < 0 || max_bytes <= 0 || max_bytes > 1024 * 1024) return lpp_empty_str();
    char *buf = (char*)VirtualAlloc(0, lpp_page_round((uint64_t)max_bytes + 1), MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE);
    if (!buf) return lpp_empty_str();
    int got = recv((SOCKET)(uintptr_t)socket_handle, buf, (int)max_bytes, 0);
    if (got <= 0) got = 0;
    buf[got] = 0;
    char *result = (char*)lpp_arc_alloc((int64_t)got + 1);
    if (result) lpp_memcpy(result, buf, got + 1);
    VirtualFree(buf, 0, MEM_RELEASE);
    return result ? result : lpp_empty_str();
}

void lpp_net_close(int64_t socket_handle) {
    if (socket_handle >= 0) closesocket((SOCKET)(uintptr_t)socket_handle);
}

int64_t lpp_net_set_nonblocking(int64_t handle, int64_t enable) {
    if (handle < 0) return -1;
    u_long mode = enable ? 1 : 0;
    return ioctlsocket((SOCKET)(uintptr_t)handle, FIONBIO, &mode) == 0 ? 1 : -1;
}

int64_t lpp_net_poll(int64_t handle, int64_t timeout_ms) {
    if (handle < 0) return -1;
    fd_set set;
    FD_ZERO(&set);
    FD_SET((SOCKET)(uintptr_t)handle, &set);
    struct timeval timeout;
    timeout.tv_sec = (long)(timeout_ms / 1000);
    timeout.tv_usec = (long)((timeout_ms % 1000) * 1000);
    return select(0, &set, NULL, NULL, &timeout) > 0 ? 1 : 0;
}
int64_t lpp_net_bind_udp(int64_t port) { (void)port; return -1; }
int64_t lpp_net_send_udp(int64_t socket, const char *host, int64_t port, const char *data) { (void)socket; (void)host; (void)port; (void)data; return -1; }
char *lpp_net_recv_udp(int64_t socket, int64_t max_bytes) { (void)socket; (void)max_bytes; return ""; }
char *lpp_http_get(const char *url) { (void)url; return ""; }
char *lpp_http_post(const char *url, const char *body, const char *content_type) { (void)url; (void)body; (void)content_type; return ""; }

void *lpp_json_get_obj(void *json, const char *key) {
    (void)json; (void)key; return NULL;
}

int64_t lpp_net_accept_timeout(int64_t listener, int64_t timeout_ms) {
    if (listener < 0) return -1;
    if (lpp_net_poll(listener, timeout_ms) != 1) return -1;
    return lpp_net_accept(listener);
}

int64_t lpp_net_dial(const char *host, int64_t port, int64_t timeout_ms) {
    (void)timeout_ms; return lpp_net_connect(host, port);
}

int64_t lpp_net_dial_udp(const char *host, int64_t port, int64_t timeout_ms) {
    (void)host; (void)port; (void)timeout_ms; return -1;
}

int64_t lpp_net_listen_udp(int64_t port) {
    (void)port; return -1;
}

char *lpp_net_resolve(const char *host) {
    (void)host; return lpp_empty_str();
}

int64_t lpp_net_send_all(int64_t handle, const char *data) {
    if (handle < 0 || !data) return -1;
    long total = 0;
    long len = 0;
    while (data[len]) len++;
    while (total < len) {
        int64_t sent = lpp_net_send(handle, data + total);
        if (sent <= 0) return -1;
        total += sent;
    }
    return 0;
}

int64_t lpp_net_set_deadline(int64_t fd, int64_t read_ms, int64_t write_ms) {
    (void)fd; (void)read_ms; (void)write_ms; return -1;
}

int64_t lpp_net_set_keepalive(int64_t fd, int64_t enable, int64_t idle_s, int64_t interval, int64_t count) {
    (void)fd; (void)enable; (void)idle_s; (void)interval; (void)count; return -1;
}

int64_t lpp_net_set_timeout(int64_t handle, int64_t milliseconds) {
    if (handle < 0 || milliseconds < 0) return -1;
    DWORD timeout = (DWORD)milliseconds;
    if (setsockopt((SOCKET)(uintptr_t)handle, SOL_SOCKET, SO_RCVTIMEO, (const char*)&timeout, sizeof(timeout)) != 0) return -1;
    if (setsockopt((SOCKET)(uintptr_t)handle, SOL_SOCKET, SO_SNDTIMEO, (const char*)&timeout, sizeof(timeout)) != 0) return -1;
    return 1;
}

int64_t lpp_vec_i64_checksum(int64_t n) {
    if (n < 0) return 0;
    int64_t total = 0;
    for (int64_t i = 0; i < n; ++i) total += (i * 3) ^ (i >> 1);
    return total;
}

