/* ── Minimal freestanding helpers ──────────────────────────────────────── */

static int lpp_parse_ip4(const char *s, uint32_t *out) {
    uint32_t a=0,b=0,c=0,d=0; int dots=0;
    while (*s>='0'&&*s<='9') a=a*10+(*(s++)-'0'); if (a>255) return 0;
    if (*s!='.') return 0; s++; dots++;
    while (*s>='0'&&*s<='9') b=b*10+(*(s++)-'0'); if (b>255) return 0;
    if (*s!='.') return 0; s++; dots++;
    while (*s>='0'&&*s<='9') c=c*10+(*(s++)-'0'); if (c>255) return 0;
    if (*s!='.') return 0; s++; dots++;
    while (*s>='0'&&*s<='9') d=d*10+(*(s++)-'0'); if (d>255) return 0;
    *out = (a<<24)|(b<<16)|(c<<8)|d; return 1;
}

static void lpp_itoa(char *buf, int val) {
    if (val==0) { buf[0]='0'; buf[1]=0; return; }
    char tmp[16]; int i=0;
    while (val>0) { tmp[i++]='0'+(val%10); val/=10; }
    int j=0; while (i>0) buf[j++]=tmp[--i];
    buf[j]=0;
}
static void lpp_strcpy(char *d, const char *s) { while ((*d++=*s++)); }
static int lpp_strlen(const char *s) { int n=0; while (s[n]) n++; return n; }

static int lpp_mini_sprintf(char *b, const char *f, ...) {
    char *o=b; __builtin_va_list a; __builtin_va_start(a,f);
    for (const char *p=f; *p; p++) {
        if (*p=='%' && p[1]) { p++;
            if (*p=='s') { const char *s=__builtin_va_arg(a,const char*); if(!s) s=""; while(*s)*o++=*s++; }
            else if (*p=='d') { int v=__builtin_va_arg(a,int); if(v<0){*o++='-';v=-v;} char t[16]; lpp_itoa(t,v); char *x=t; while(*x)*o++=*x++; }
            else { *o++='%'; *o++=*p; }
        } else *o++=*p;
    } __builtin_va_end(a); *o=0; return (int)(o-b);
}

/* ── Freestanding Socket Networking ────────────────────────────────────── */

struct lpp_sockaddr_in {
    uint16_t sin_family;
    uint16_t sin_port;
    uint32_t sin_addr;
    char sin_zero[8];
};

struct lpp_sockaddr_in6 {
    uint16_t sin6_family;
    uint16_t sin6_port;
    uint32_t sin6_flowinfo;
    unsigned char sin6_addr[16];
    uint32_t sin6_scope_id;
};

struct lpp_timeval {
    long tv_sec;
    long tv_usec;
};

static uint16_t lpp_htons(uint16_t val) { return (uint16_t)((val << 8) | (val >> 8)); }
static uint32_t lpp_htonl(uint32_t val) {
    return ((val & 0xff) << 24) | ((val & 0xff00) << 8) |
           ((val >> 8) & 0xff00) | ((val >> 24) & 0xff);
}
static uint16_t lpp_ntohs(uint16_t val) { return lpp_htons(val); }

/* ── Syscall wrappers ──────────────────────────────────────────────────── */

static long lpp_sys_socket(int domain, int type, int protocol) {
    long ret; __asm__ volatile ("syscall" : "=a"(ret) : "a"(41), "D"((long)domain), "S"((long)type), "d"((long)protocol) : "rcx","r11","memory"); return ret;
}
static long lpp_sys_connect(long fd, const void *addr, int addrlen) {
    long ret; __asm__ volatile ("syscall" : "=a"(ret) : "a"(42), "D"(fd), "S"(addr), "d"((long)addrlen) : "rcx","r11","memory"); return ret;
}
static long lpp_sys_accept(long fd, void *addr, void *addrlen) {
    long ret; __asm__ volatile ("syscall" : "=a"(ret) : "a"(43), "D"(fd), "S"(addr), "d"(addrlen) : "rcx","r11","memory"); return ret;
}
static long lpp_sys_sendto(long fd, const void *buf, long len, int flags) {
    long ret; register long r10 __asm__("r10") = (long)flags; register long r8 __asm__("r8") = 0; register long r9 __asm__("r9") = 0;
    __asm__ volatile ("syscall" : "=a"(ret) : "a"(44), "D"(fd), "S"(buf), "d"(len), "r"(r10), "r"(r8), "r"(r9) : "rcx","r11","memory"); return ret;
}
static long lpp_sys_recvfrom(long fd, void *buf, long len, int flags) {
    long ret; register long r10 __asm__("r10") = (long)flags; register long r8 __asm__("r8") = 0; register long r9 __asm__("r9") = 0;
    __asm__ volatile ("syscall" : "=a"(ret) : "a"(45), "D"(fd), "S"(buf), "d"(len), "r"(r10), "r"(r8), "r"(r9) : "rcx","r11","memory"); return ret;
}
static long lpp_sys_bind(long fd, const void *addr, int addrlen) {
    long ret; __asm__ volatile ("syscall" : "=a"(ret) : "a"(49), "D"(fd), "S"(addr), "d"((long)addrlen) : "rcx","r11","memory"); return ret;
}
static long lpp_sys_listen(long fd, int backlog) {
    long ret; __asm__ volatile ("syscall" : "=a"(ret) : "a"(50), "D"(fd), "S"((long)backlog) : "rcx","r11","memory"); return ret;
}
static long lpp_sys_setsockopt(long fd, int level, int optname, const void *optval, int optlen) {
    long ret; register long r10 __asm__("r10") = (long)optlen;
    __asm__ volatile ("syscall" : "=a"(ret) : "a"(54), "D"(fd), "S"((long)level), "d"((long)optname), "r"(optval), "r"(r10) : "rcx","r11","memory"); return ret;
}
static long lpp_sys_getsockname(long fd, void *addr, void *addrlen) {
    long ret; __asm__ volatile ("syscall" : "=a"(ret) : "a"(51), "D"(fd), "S"(addr), "d"(addrlen) : "rcx","r11","memory"); return ret;
}
static long lpp_sys_poll(void *fds, long nfds, int timeout_ms) {
    long ret; register long r10 __asm__("r10") = (long)timeout_ms;
    __asm__ volatile ("syscall" : "=a"(ret) : "a"(7), "D"(fds), "S"(nfds), "r"(r10) : "rcx","r11","memory"); return ret;
}
static long lpp_sys_fcntl(long fd, int cmd, long arg) {
    long ret; __asm__ volatile ("syscall" : "=a"(ret) : "a"(72), "D"(fd), "S"((long)cmd), "d"(arg) : "rcx","r11","memory"); return ret;
}

/* ── Deadline helpers ──────────────────────────────────────────────────── */

static long lpp_net_sock_timeout(long fd, int ms, int is_recv) {
    if (fd < 0 || ms < 0) return 0;
    struct lpp_timeval tv;
    tv.tv_sec = ms / 1000;
    tv.tv_usec = (ms % 1000) * 1000;
    int opt = is_recv ? 20 : 21; /* SO_RCVTIMEO=20, SO_SNDTIMEO=21 */
    return lpp_sys_setsockopt(fd, 1, opt, &tv, sizeof(tv));
}

/* ── Keepalive ─────────────────────────────────────────────────────────── */

static long lpp_net_set_keepalive(long fd, int enable, int idle_sec, int interval, int count) {
    if (fd < 0) return -1;
    int val = enable ? 1 : 0;
    if (lpp_sys_setsockopt(fd, 1, 9, &val, sizeof(val)) < 0) return -1;   /* SO_KEEPALIVE=9 */
    if (enable) {
        if (lpp_sys_setsockopt(fd, 6, 4, &idle_sec, sizeof(idle_sec)) < 0) return -1;     /* TCP_KEEPIDLE=4 */
        if (lpp_sys_setsockopt(fd, 6, 5, &interval, sizeof(interval)) < 0) return -1;      /* TCP_KEEPINTVL=5 */
        if (lpp_sys_setsockopt(fd, 6, 6, &count, sizeof(count)) < 0) return -1;            /* TCP_KEEPCNT=6 */
    }
    return 0;
}

/* ── Non-blocking toggle ───────────────────────────────────────────────── */

static long lpp_net_set_nonblock(long fd, int enable) {
    if (fd < 0) return -1;
    long flags = lpp_sys_fcntl(fd, 3, 0); /* F_GETFL=3 */
    if (flags < 0) return -1;
    if (enable) flags |= 0x800;   /* O_NONBLOCK=0x800 */
    else        flags &= ~0x800L;
    return lpp_sys_fcntl(fd, 4, flags); /* F_SETFL=4 */
}

/* ── DNS resolver (native, no libc) ────────────────────────────────────── */

typedef struct { uint16_t id, flags, qdcount, ancount, nscount, arcount; } dns_header_t;

static int lpp_dns_build_query(char *buf, const char *host) {
    dns_header_t *h = (dns_header_t *)buf;
    h->id = lpp_htons(0x4c50); /* "LP" */
    h->flags = lpp_htons(0x0100); /* standard query, recursion desired */
    h->qdcount = lpp_htons(1);
    h->ancount = h->nscount = h->arcount = 0;
    int off = 12;
    /* Encode hostname as label sequence */
    const char *p = host; int start = off;
    while (*p) {
        const char *dot = p; while (*dot && *dot != '.') dot++;
        int len = (int)(dot - p);
        if (len > 63 || off + len + 2 > 512) return -1;
        buf[off++] = (char)len;
        for (int i = 0; i < len; i++) buf[off++] = p[i];
        if (*dot == '.') dot++;
        p = dot;
    }
    buf[off++] = 0;
    /* Query type A (1), class IN (1) */
    uint16_t qtype = lpp_htons(1), qclass = lpp_htons(1);
    buf[off++] = (char)(qtype >> 8); buf[off++] = (char)(qtype & 0xff);
    buf[off++] = (char)(qclass >> 8); buf[off++] = (char)(qclass & 0xff);
    return off;
}

static int lpp_dns_parse_a(const char *response, int len, uint32_t *out_ip) {
    if (len < 12) return 0;
    dns_header_t *h = (dns_header_t *)response;
    int qdcount = lpp_ntohs(h->qdcount);
    int ancount = lpp_ntohs(h->ancount);
    if (ancount < 1) return 0;

    int off = 12;
    /* Skip question section */
    while (qdcount-- > 0 && off < len) {
        while (off < len && response[off]) {
            if ((response[off] & 0xc0) == 0xc0) { off += 2; break; }
            off += response[off] + 1;
        }
        if (!(response[off - 1] & 0xc0)) off++; /* null terminator */
        off += 4; /* QTYPE + QCLASS */
    }
    /* Parse answers */
    for (int i = 0; i < ancount && off + 10 <= len; i++) {
        /* Skip name (may be a pointer) */
        if ((response[off] & 0xc0) == 0xc0) { off += 2; }
        else { while (off < len && response[off]) { off += response[off] + 1; } off++; }
        uint16_t atype = ((uint16_t)(unsigned char)response[off] << 8) | (unsigned char)response[off+1];
        uint16_t aclass = ((uint16_t)(unsigned char)response[off+2] << 8) | (unsigned char)response[off+3];
        int rdlen = ((int)(unsigned char)response[off+8] << 8) | (unsigned char)response[off+9];
        off += 10;
        if (atype == 1 && aclass == 1 && rdlen == 4 && off + 4 <= len) {
            *out_ip = ((uint32_t)(unsigned char)response[off] << 24) |
                      ((uint32_t)(unsigned char)response[off+1] << 16) |
                      ((uint32_t)(unsigned char)response[off+2] << 8) |
                      (uint32_t)(unsigned char)response[off+3];
            return 1;
        }
        off += rdlen;
    }
    return 0;
}

static int lpp_resolve_host(const char *host, uint32_t *out_ip, int timeout_ms) {
    if (!host || !out_ip) return 0;

    /* Try parsing as literal IP first */
    {   uint32_t a=0,b=0,c=0,d=0; int dots=0;
        const char *s = host;
        a=0; while (*s>='0'&&*s<='9') a=a*10+(*s++-'0'); if (*s=='.') {s++;dots++;}
        b=0; while (*s>='0'&&*s<='9') b=b*10+(*s++-'0'); if (*s=='.') {s++;dots++;}
        c=0; while (*s>='0'&&*s<='9') c=c*10+(*s++-'0'); if (*s=='.') {s++;dots++;}
        d=0; while (*s>='0'&&*s<='9') d=d*10+(*s++-'0');
        if (dots==3 && a<256 && b<256 && c<256 && d<256 && *s==0) {
            *out_ip = (a<<24)|(b<<16)|(c<<8)|d; return 1;
        }
    }

    /* Read /etc/resolv.conf for nameserver */
    long resolv_fd = lpp_sys_open("/etc/resolv.conf", 0, 0);
    if (resolv_fd < 0) return 0;
    char resolv_buf[512]; long resolv_len = lpp_sys_read(resolv_fd, resolv_buf, sizeof(resolv_buf)-1);
    lpp_sys_close(resolv_fd);
    if (resolv_len < 0) resolv_len = 0;
    resolv_buf[resolv_len] = 0;

    uint32_t ns_ip = 0x08080808; /* fallback 8.8.8.8 */
    char *line = resolv_buf;
    while (*line) {
        while (*line == ' ' || *line == '\t') line++;
        if (line[0]=='n'&&line[1]=='a'&&line[2]=='m'&&line[3]=='e'&&line[4]=='s'&&line[5]=='e'&&line[6]=='r'&&line[7]=='v'&&line[8]=='e'&&line[9]=='r') {
            line += 10; while (*line==' '||*line=='\t') line++;
            if (lpp_parse_ip4(line, &ns_ip)) { break; }
        }
        while (*line && *line!='\n') line++;
        if (*line=='\n') line++;
    }

    /* Send DNS query via UDP */
    long s = lpp_sys_socket(2, 2, 0); /* AF_INET, SOCK_DGRAM */
    if (s < 0) return 0;

    /* Set timeout on the DNS socket */
    if (timeout_ms > 0) {
        struct lpp_timeval tv; tv.tv_sec = timeout_ms/1000; tv.tv_usec = (timeout_ms%1000)*1000;
        lpp_sys_setsockopt(s, 1, 20, &tv, sizeof(tv));
    }

    struct lpp_sockaddr_in ns_addr = {0};
    ns_addr.sin_family = 2;
    ns_addr.sin_port = lpp_htons(53);
    ns_addr.sin_addr = ns_ip; /* network byte order already */

    if (lpp_sys_connect(s, &ns_addr, sizeof(ns_addr)) < 0) { lpp_sys_close(s); return 0; }

    char query[512];
    int qlen = lpp_dns_build_query(query, host);
    if (qlen < 0) { lpp_sys_close(s); return 0; }

    if (lpp_sys_sendto(s, query, qlen, 0) < 0) { lpp_sys_close(s); return 0; }

    char response[512];
    long rlen = lpp_sys_recvfrom(s, response, sizeof(response), 0);
    lpp_sys_close(s);

    if (rlen < 12) return 0;
    return lpp_dns_parse_a(response, (int)rlen, out_ip);
}

/* ── Public API ────────────────────────────────────────────────────────── */

/* net_dial(host, port) -> fd  (TCP with DNS + deadline support) */
int64_t lpp_net_dial(const char *host, int64_t port, int64_t timeout_ms) {
    if (!host || port < 1 || port > 65535) return -1;
    uint32_t ip;
    if (!lpp_resolve_host(host, &ip, (int)timeout_ms)) return -1;

    long fd = lpp_sys_socket(2, 1, 0); /* AF_INET, SOCK_STREAM */
    if (fd < 0) return -1;

    if (timeout_ms > 0) {
        lpp_net_set_nonblock(fd, 1);

        struct lpp_sockaddr_in addr = {0};
        addr.sin_family = 2;
        addr.sin_port = lpp_htons((uint16_t)port);
        addr.sin_addr = ip;

        long cr = lpp_sys_connect(fd, &addr, sizeof(addr));
        if (cr < 0) {
            /* EINPROGRESS = -115, check with poll */
            struct { int fd; short events; short revents; } pfd;
            pfd.fd = (int)fd; pfd.events = 4; /* POLLOUT=4 */
            long pr = lpp_sys_poll(&pfd, 1, (int)timeout_ms);
            if (pr <= 0 || !(pfd.revents & 4)) { lpp_sys_close(fd); return -1; }
        }
        lpp_net_set_nonblock(fd, 0);
        /* Set SO_SNDTIMEO/SO_RCVTIMEO for subsequent operations */
        lpp_net_sock_timeout(fd, (int)timeout_ms, 1);
        lpp_net_sock_timeout(fd, (int)timeout_ms, 0);
    } else {
        struct lpp_sockaddr_in addr = {0};
        addr.sin_family = 2;
        addr.sin_port = lpp_htons((uint16_t)port);
        addr.sin_addr = ip;
        if (lpp_sys_connect(fd, &addr, sizeof(addr)) < 0) { lpp_sys_close(fd); return -1; }
    }
    return (int64_t)fd;
}

/* net_dial_udp(host, port) -> fd */
int64_t lpp_net_dial_udp(const char *host, int64_t port, int64_t timeout_ms) {
    if (!host || port < 1 || port > 65535) return -1;
    uint32_t ip;
    if (!lpp_resolve_host(host, &ip, (int)timeout_ms)) return -1;

    long fd = lpp_sys_socket(2, 2, 0); /* AF_INET, SOCK_DGRAM */
    if (fd < 0) return -1;

    struct lpp_sockaddr_in addr = {0};
    addr.sin_family = 2;
    addr.sin_port = lpp_htons((uint16_t)port);
    addr.sin_addr = ip;
    if (lpp_sys_connect(fd, &addr, sizeof(addr)) < 0) { lpp_sys_close(fd); return -1; }
    if (timeout_ms > 0) {
        lpp_net_sock_timeout(fd, (int)timeout_ms, 1);
        lpp_net_sock_timeout(fd, (int)timeout_ms, 0);
    }
    return (int64_t)fd;
}

/* net_listen(port) -> fd  (TCP) */
int64_t lpp_net_listen(int64_t port) {
    long sock = lpp_sys_socket(2, 1, 0);
    if (sock < 0) return -1;
    int reuse = 1;
    lpp_sys_setsockopt(sock, 1, 2, &reuse, sizeof(reuse)); /* SO_REUSEADDR=2 */
    struct lpp_sockaddr_in addr = {0};
    addr.sin_family = 2;
    addr.sin_port = lpp_htons((uint16_t)port);
    addr.sin_addr = 0;
    if (lpp_sys_bind(sock, &addr, sizeof(addr)) < 0) { lpp_sys_close(sock); return -1; }
    if (lpp_sys_listen(sock, 128) < 0) { lpp_sys_close(sock); return -1; }
    return (int64_t)sock;
}

/* net_listen_udp(port) -> fd */
int64_t lpp_net_listen_udp(int64_t port) {
    long sock = lpp_sys_socket(2, 2, 0);
    if (sock < 0) return -1;
    struct lpp_sockaddr_in addr = {0};
    addr.sin_family = 2;
    addr.sin_port = lpp_htons((uint16_t)port);
    addr.sin_addr = 0;
    if (lpp_sys_bind(sock, &addr, sizeof(addr)) < 0) { lpp_sys_close(sock); return -1; }
    return (int64_t)sock;
}

/* net_accept(listener_fd, timeout_ms) -> client_fd */
int64_t lpp_net_accept_timeout(int64_t listener, int64_t timeout_ms) {
    if (listener < 0) return -1;
    if (timeout_ms > 0) {
        struct { int fd; short events; short revents; } pfd;
        pfd.fd = (int)listener; pfd.events = 1; /* POLLIN=1 */
        long pr = lpp_sys_poll(&pfd, 1, (int)timeout_ms);
        if (pr <= 0) return -1;
    }
    long client = lpp_sys_accept((long)listener, 0, 0);
    return (int64_t)client;
}

int64_t lpp_net_accept(int64_t listener) { return lpp_net_accept_timeout(listener, -1); }

/* net_send(fd, data) -> bytes_written */
int64_t lpp_net_send(int64_t fd, const char *data) {
    if (fd < 0 || !data) return -1;
    long len = 0; while (data[len]) len++;
    long sent = lpp_sys_sendto((long)fd, data, len, 0x4000); /* MSG_NOSIGNAL */
    return (int64_t)sent;
}

/* net_send_all(fd, data) -> total bytes written */
int64_t lpp_net_send_all(int64_t fd, const char *data) {
    if (fd < 0 || !data) return -1;
    long total = 0, len = 0; while (data[len]) len++;
    while (total < len) {
        long sent = lpp_sys_sendto((long)fd, data + total, len - total, 0x4000);
        if (sent <= 0) break;
        total += sent;
    }
    return (int64_t)total;
}

/* net_recv(fd, max_bytes) -> heap-allocated string */
char* lpp_net_recv(int64_t fd, int64_t max_bytes) {
    if (fd < 0 || max_bytes <= 0) return (char*)"";
    char *buf = (char*)lpp_arc_alloc(max_bytes + 1);
    if (!buf) return (char*)"";
    long recvd = lpp_sys_recvfrom((long)fd, buf, max_bytes, 0);
    if (recvd < 0) recvd = 0;
    buf[recvd] = '\0';
    return buf;
}

/* net_recv_udp(fd, max_bytes) -> string (reads from unconnected UDP) */
char* lpp_net_recv_udp(int64_t fd, int64_t max_bytes) {
    return lpp_net_recv(fd, max_bytes);
}

void lpp_net_close(int64_t fd) { if (fd >= 0) lpp_sys_close((long)fd); }

/* net_set_deadline(fd, read_ms, write_ms) */
int64_t lpp_net_set_deadline(int64_t fd, int64_t read_ms, int64_t write_ms) {
    if (fd < 0) return -1;
    int ok = 1;
    if (read_ms >= 0 && lpp_net_sock_timeout((long)fd, (int)read_ms, 1) < 0) ok = 0;
    if (write_ms >= 0 && lpp_net_sock_timeout((long)fd, (int)write_ms, 0) < 0) ok = 0;
    return ok ? 1 : -1;
}

int64_t lpp_net_set_timeout(int64_t fd, int64_t ms) {
    return lpp_net_set_deadline(fd, ms, ms);
}

/* net_set_keepalive(fd, enable, idle_s, interval, count) */
int64_t lpp_net_set_keepalive(int64_t fd, int64_t enable, int64_t idle_s, int64_t interval, int64_t count) {
    return (int64_t)lpp_net_set_keepalive((long)fd, (int)enable, (int)idle_s, (int)interval, (int)count);
}

/* net_resolve(host) -> IP string */
char* lpp_net_resolve(const char *host) {
    uint32_t ip;
    if (!lpp_resolve_host(host, &ip, 5000)) return (char*)"";
    char *buf = (char*)lpp_arc_alloc(16);
    if (!buf) return (char*)"";
    /* Format as dotted-quad */
    int off = 0;
    for (int shift = 24; shift >= 0; shift -= 8) {
        uint8_t octet = (uint8_t)((ip >> shift) & 0xff);
        if (octet >= 100) buf[off++] = '0' + octet/100;
        if (octet >= 10)  buf[off++] = '0' + (octet%100)/10;
        buf[off++] = '0' + octet%10;
        if (shift > 0) buf[off++] = '.';
    }
    buf[off] = 0;
    return buf;
}

/* ── HTTP client ───────────────────────────────────────────────────────── */

/* http_get(url) -> response body string */
char* lpp_http_get(const char *url, int64_t timeout_ms) {
    if (!url) return (char*)"";
    /* Parse: http://host[:port]/path */
    const char *p = url;
    if (p[0]=='h'&&p[1]=='t'&&p[2]=='t'&&p[3]=='p'&&p[4]==':'&&p[5]=='/'&&p[6]=='/') p += 7;
    else if (p[0]=='h'&&p[1]=='t'&&p[2]=='t'&&p[3]=='p'&&p[4]=='s') return (char*)"http_get does not support HTTPS";
    else return (char*)"invalid URL";

    char host[256]; int host_len = 0;
    while (*p && *p != ':' && *p != '/' && host_len < 255) host[host_len++] = *p++;
    host[host_len] = 0;
    int port = 80;
    if (*p == ':') { p++; port = 0; while (*p >= '0' && *p <= '9') port = port * 10 + (*p++ - '0'); }
    const char *path = (*p == '/') ? p : "/";
    if (*path == 0) path = "/";

    int64_t fd = lpp_net_dial(host, (int64_t)port, timeout_ms);
    if (fd < 0) return (char*)"";

    char req[2048];
    int req_len = 0;
    req_len += lpp_mini_sprintf(req + req_len, "GET %s HTTP/1.1\r\nHost: %s\r\nConnection: close\r\nAccept: */*\r\nUser-Agent: L++/0.1.3\r\n\r\n", path, host);
    if (!req_len) req_len = sizeof(req) - 1;

    if (lpp_net_send_all(fd, req) < 0) { lpp_net_close(fd); return (char*)""; }

    /* Read response into a growable buffer */
    int cap = 4096, len = 0;
    char *buf = (char*)lpp_arc_alloc(cap + 1);
    if (!buf) { lpp_net_close(fd); return (char*)""; }
    while (1) {
        if (len + 1024 >= cap) {
            int new_cap = cap * 2;
            char *new_buf = (char*)lpp_arc_alloc(new_cap + 1);
            if (!new_buf) break;
            for (int i = 0; i < len; i++) new_buf[i] = buf[i];
            lpp_arc_release(buf);
            buf = new_buf; cap = new_cap;
        }
        long n = lpp_sys_recvfrom((long)fd, buf + len, cap - len, 0);
        if (n <= 0) break;
        len += (int)n;
    }
    lpp_net_close(fd);
    buf[len] = 0;

    /* Strip HTTP headers: find \r\n\r\n */
    int body_start = 0;
    for (int i = 0; i < len - 3; i++) {
        if (buf[i]=='\r' && buf[i+1]=='\n' && buf[i+2]=='\r' && buf[i+3]=='\n') {
            body_start = i + 4; break;
        }
    }
    if (body_start > 0 && body_start < len) {
        /* Move body to start, add null */
        for (int i = 0; body_start + i <= len; i++) buf[i] = buf[body_start + i];
    }
    return buf;
}

/* http_post(url, body, content_type, timeout_ms) -> response body string */
char* lpp_http_post(const char *url, const char *body, const char *content_type, int64_t timeout_ms) {
    if (!url) return (char*)"";
    const char *p = url;
    if (p[0]=='h'&&p[1]=='t'&&p[2]=='t'&&p[3]=='p'&&p[4]==':'&&p[5]=='/'&&p[6]=='/') p += 7;
    else return (char*)"invalid URL";

    char host[256]; int host_len = 0;
    while (*p && *p != ':' && *p != '/' && host_len < 255) host[host_len++] = *p++;
    host[host_len] = 0;
    int port = 80;
    if (*p == ':') { p++; port = 0; while (*p >= '0' && *p <= '9') port = port * 10 + (*p++ - '0'); }
    const char *path = (*p == '/') ? p : "/";
    if (*path == 0) path = "/";
    if (!body) body = "";
    if (!content_type) content_type = "application/x-www-form-urlencoded";
    int body_len = 0; while (body[body_len]) body_len++;

    int64_t fd = lpp_net_dial(host, (int64_t)port, timeout_ms);
    if (fd < 0) return (char*)"";

    char req[4096];
    int req_len = lpp_mini_sprintf(req,
        "POST %s HTTP/1.1\r\nHost: %s\r\nContent-Type: %s\r\nContent-Length: %d\r\nConnection: close\r\nAccept: */*\r\nUser-Agent: L++/0.1.3\r\n\r\n%s",
        path, host, content_type, body_len, body);
    if (!req_len) req_len = sizeof(req) - 1;

    if (lpp_net_send_all(fd, req) < 0) { lpp_net_close(fd); return (char*)""; }

    int cap = 4096, len = 0;
    char *buf = (char*)lpp_arc_alloc(cap + 1);
    if (!buf) { lpp_net_close(fd); return (char*)""; }
    while (1) {
        if (len + 1024 >= cap) {
            int new_cap = cap * 2;
            char *new_buf = (char*)lpp_arc_alloc(new_cap + 1);
            if (!new_buf) break;
            for (int i = 0; i < len; i++) new_buf[i] = buf[i];
            lpp_arc_release(buf); buf = new_buf; cap = new_cap;
        }
        long n = lpp_sys_recvfrom((long)fd, buf + len, cap - len, 0);
        if (n <= 0) break;
        len += (int)n;
    }
    lpp_net_close(fd);
    buf[len] = 0;
    int body_start = 0;
    for (int i = 0; i < len - 3; i++) {
        if (buf[i]=='\r' && buf[i+1]=='\n' && buf[i+2]=='\r' && buf[i+3]=='\n') { body_start = i + 4; break; }
    }
    if (body_start > 0 && body_start < len) {
        for (int i = 0; body_start + i <= len; i++) buf[i] = buf[body_start + i];
    }
    return buf;
}

/* ── Legacy compatibility shims ───────────────────────────────────────── */

int64_t lpp_net_connect(const char *host, int64_t port) {
    return lpp_net_dial(host, port, 30000);
}
