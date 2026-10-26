/* ── Freestanding Socket Networking ────────────────────────── */

struct lpp_sockaddr_in {
    uint16_t sin_family;
    uint16_t sin_port;
    uint32_t sin_addr;
    char sin_zero[8];
};

static uint16_t lpp_htons(uint16_t val) {
    return (uint16_t)((val << 8) | (val >> 8));
}

static long lpp_sys_socket(int domain, int type, int protocol) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(41), "D"((long)domain), "S"((long)type), "d"((long)protocol)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_connect(long fd, const void *addr, int addrlen) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(42), "D"(fd), "S"(addr), "d"((long)addrlen)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_accept(long fd, void *addr, void *addrlen) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(43), "D"(fd), "S"(addr), "d"(addrlen)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_sendto(long fd, const void *buf, long len, int flags) {
    long ret;
    register long r10 __asm__("r10") = (long)flags;
    register long r8 __asm__("r8") = 0;
    register long r9 __asm__("r9") = 0;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(44), "D"(fd), "S"(buf), "d"(len), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_recvfrom(long fd, void *buf, long len, int flags) {
    long ret;
    register long r10 __asm__("r10") = (long)flags;
    register long r8 __asm__("r8") = 0;
    register long r9 __asm__("r9") = 0;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(45), "D"(fd), "S"(buf), "d"(len), "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_bind(long fd, const void *addr, int addrlen) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(49), "D"(fd), "S"(addr), "d"((long)addrlen)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_listen(long fd, int backlog) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(50), "D"(fd), "S"((long)backlog)
        : "rcx", "r11", "memory"
    );
    return ret;
}

int64_t lpp_net_listen(int64_t port) {
    long sock = lpp_sys_socket(2, 1, 0); /* AF_INET, SOCK_STREAM */
    if (sock < 0) return -1;
    struct lpp_sockaddr_in addr = {0};
    addr.sin_family = 2;
    addr.sin_port = lpp_htons((uint16_t)port);
    /* Bind Web backends to loopback; callers must explicitly add a
       separately-reviewed network listener if LAN exposure is desired. */
    addr.sin_addr = 0x0100007f; /* 127.0.0.1 */
    if (lpp_sys_bind(sock, &addr, sizeof(addr)) < 0) {
        lpp_sys_close(sock);
        return -1;
    }
    if (lpp_sys_listen(sock, 128) < 0) {
        lpp_sys_close(sock);
        return -1;
    }
    return (int64_t)sock;
}

int64_t lpp_net_accept(int64_t listener) {
    if (listener < 0) return -1;
    long client = lpp_sys_accept((long)listener, 0, 0);
    return (int64_t)client;
}

int64_t lpp_net_connect(const char *host, int64_t port) {
    (void)host;
    long sock = lpp_sys_socket(2, 1, 0);
    if (sock < 0) return -1;
    struct lpp_sockaddr_in addr = {0};
    addr.sin_family = 2;
    addr.sin_port = lpp_htons((uint16_t)port);
    addr.sin_addr = 0x0100007f; /* 127.0.0.1 in network byte order */
    if (lpp_sys_connect(sock, &addr, sizeof(addr)) < 0) {
        lpp_sys_close(sock);
        return -1;
    }
    return (int64_t)sock;
}

int64_t lpp_net_send(int64_t fd, const char *data) {
    if (fd < 0 || !data) return -1;
    long len = 0;
    while (data[len]) len++;
    long sent = lpp_sys_sendto((long)fd, data, len, 0x4000); /* MSG_NOSIGNAL */
    return (int64_t)sent;
}

int64_t lpp_net_send_all(int64_t fd, const char *data) {
    if (fd < 0 || !data) return -1;
    long total = 0;
    long len = 0;
    while (data[len]) len++;
    while (total < len) {
        long sent = lpp_sys_sendto((long)fd, data + total, len - total, 0x4000);
        if (sent <= 0) break;
        total += sent;
    }
    return (int64_t)total;
}

char* lpp_net_recv(int64_t fd, int64_t max_bytes) {
    if (fd < 0 || max_bytes <= 0) return (char*)"";
    char *buf = (char*)lpp_arc_alloc(max_bytes + 1);
    if (!buf) return (char*)"";
    long recvd = lpp_sys_recvfrom((long)fd, buf, max_bytes, 0);
    if (recvd < 0) recvd = 0;
    buf[recvd] = '\0';
    return buf;
}

void lpp_net_close(int64_t fd) {
    if (fd >= 0) {
        lpp_sys_close((long)fd);
    }
}

int64_t lpp_net_set_timeout(int64_t fd, int64_t ms) {
    (void)fd; (void)ms;
    return 1;
}

struct lpp_pollfd {
    int fd;
    short events;
    short revents;
};

static long lpp_sys_poll(struct lpp_pollfd *fds, unsigned long nfds, int timeout) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(7), "D"(fds), "S"(nfds), "d"((long)timeout)
        : "rcx", "r11", "memory"
    );
    return ret;
}

int64_t lpp_net_poll(int64_t handle, int64_t timeout_ms) {
    if (handle < 0) return -1;
    struct lpp_pollfd pfd;
    pfd.fd = (int)handle;
    pfd.events = 0x0001; /* POLLIN */
    pfd.revents = 0;
    long ret = lpp_sys_poll(&pfd, 1, (int)timeout_ms);
    return (ret > 0 && (pfd.revents & 0x0001)) ? 1 : 0;
}

static long lpp_sys_fcntl(long fd, unsigned int cmd, unsigned long arg) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(72), "D"(fd), "S"((long)cmd), "d"(arg)
        : "rcx", "r11", "memory"
    );
    return ret;
}

int64_t lpp_net_set_nonblocking(int64_t handle, int64_t enable) {
    if (handle < 0) return -1;
    long flags = lpp_sys_fcntl(handle, 3, 0); /* F_GETFL */
    if (flags < 0) flags = 0;
    if (enable) {
        flags |= 04000; /* O_NONBLOCK */
    } else {
        flags &= ~04000;
    }
    return lpp_sys_fcntl(handle, 4, (unsigned long)flags) >= 0 ? 1 : -1; /* F_SETFL */
}

