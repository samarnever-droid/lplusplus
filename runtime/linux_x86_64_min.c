/*
 * Freestanding Phase 2 ELF runtime.
 *
 * This runtime supports syscall-backed integer/string output, ARC memory,
 * dynamic lists, file I/O, and socket networking without libc.
 * Build with:
 * cc -O2 -ffreestanding -fno-stack-protector -fno-pic -mno-red-zone -c \
 *    runtime/linux_x86_64_min.c -o lpp_runtime_min.o
 */

#include <stdint.h>

static long lpp_sys_write(long fd, const void *buffer, long count) {
    long result;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(1), "D"(fd), "S"(buffer), "d"(count)
        : "rcx", "r11", "memory"
    );
    return result;
}

void lpp_print_int(int64_t value) {
    char buffer[32];
    char *cursor = buffer + sizeof(buffer);
    uint64_t magnitude;
    int negative = value < 0;
    if (negative) {
        /* Avoid signed overflow for INT64_MIN. */
        magnitude = (uint64_t)(-(value + 1)) + 1;
    } else {
        magnitude = (uint64_t)value;
    }
    *--cursor = '\n';
    do {
        *--cursor = (char)('0' + (magnitude % 10));
        magnitude /= 10;
    } while (magnitude != 0);
    if (negative) *--cursor = '-';
    (void)lpp_sys_write(1, cursor, (long)((buffer + sizeof(buffer)) - cursor));
}

void lpp_print_str(const char *text) {
    const char *end = text;
    char newline = '\n';
    if (!text) return;
    while (*end) end++;
    (void)lpp_sys_write(1, text, (long)(end - text));
    (void)lpp_sys_write(1, &newline, 1);
}

/* ── Freestanding ARC foundation ─────────────────────────────────────────── */
/* Every direct-link ARC allocation owns a whole mmap region. */

typedef void (*LppArcDestructor)(void *payload);
typedef struct {
    int refcount;
    LppArcDestructor destructor;
    uint64_t map_size;
} LppArcHeader;

static uint64_t lpp_page_round(uint64_t size) {
    const uint64_t page = 4096;
    return (size + page - 1) & ~(page - 1);
}

static void *lpp_sys_mmap(uint64_t size) {
    long result;
    register long r10 __asm__("r10") = 0x22; /* MAP_PRIVATE | MAP_ANONYMOUS */
    register long r8 __asm__("r8") = -1;
    register long r9 __asm__("r9") = 0;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(9), "D"((long)0), "S"((long)size), "d"((long)3),
          "r"(r10), "r"(r8), "r"(r9)
        : "rcx", "r11", "memory"
    );
    return result < 0 ? (void *)0 : (void *)result;
}

static void lpp_sys_munmap(void *address, uint64_t size) {
    long ignored;
    __asm__ volatile (
        "syscall"
        : "=a"(ignored)
        : "a"(11), "D"(address), "S"((long)size)
        : "rcx", "r11", "memory"
    );
}

void *lpp_arc_alloc_with_destructor(int64_t payload_size, LppArcDestructor destructor) {
    if (payload_size < 0) return 0;
    uint64_t total = lpp_page_round((uint64_t)payload_size + sizeof(LppArcHeader));
    LppArcHeader *header = (LppArcHeader *)lpp_sys_mmap(total);
    if (!header) return 0;
    header->refcount = 1;
    header->destructor = destructor;
    header->map_size = total;
    return (void *)(header + 1);
}

void *lpp_arc_alloc(int64_t payload_size) {
    return lpp_arc_alloc_with_destructor(payload_size, 0);
}

void lpp_arc_retain(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    (void)__atomic_add_fetch(&header->refcount, 1, __ATOMIC_ACQ_REL);
}

void lpp_arc_release(void *payload) {
    if (!payload) return;
    LppArcHeader *header = (LppArcHeader *)payload - 1;
    if (__atomic_sub_fetch(&header->refcount, 1, __ATOMIC_ACQ_REL) == 0) {
        if (header->destructor) header->destructor(payload);
        lpp_sys_munmap(header, header->map_size);
    }
}

void *lpp_alloc(int64_t size) {
    return lpp_arc_alloc(size);
}

void lpp_free(void *payload, int64_t size) {
    (void)size;
    lpp_arc_release(payload);
}

/* ARC closure payload: [code pointer, owned environment pointer]. */
void lpp_closure_destroy(void *closure) {
    if (!closure) return;
    void **parts = (void **)closure;
    lpp_arc_release(parts[1]);
}

/* ── Freestanding List runtime ──────────────────────────────────────────── */
typedef struct {
    int64_t *data;
    int64_t len;
    int64_t cap;
    uint64_t data_map_size;
    int arc_elements;
} LppList;

static void lpp_list_destroy(void *payload) {
    LppList *list = (LppList *)payload;
    if (!list) return;
    if (list->arc_elements) {
        for (int64_t i = 0; i < list->len; ++i) {
            lpp_arc_release((void *)(intptr_t)list->data[i]);
        }
    }
    if (list->data) lpp_sys_munmap(list->data, list->data_map_size);
}

static void *lpp_list_new_with_mode(int arc_elements) {
    LppList *list = (LppList *)lpp_arc_alloc_with_destructor(
        (int64_t)sizeof(LppList), lpp_list_destroy
    );
    if (!list) return 0;
    list->arc_elements = arc_elements;
    return list;
}

void *lpp_list_new(void) {
    return lpp_list_new_with_mode(0);
}

void *lpp_list_new_arc(void) {
    return lpp_list_new_with_mode(1);
}

void lpp_list_push(void *raw, int64_t value) {
    LppList *list = (LppList *)raw;
    if (!list) return;
    if (list->len == list->cap) {
        int64_t next_cap = list->cap == 0 ? 8 : list->cap * 2;
        if (next_cap < list->cap || next_cap > (int64_t)(0x7fffffffffffffffLL / 8)) return;
        uint64_t next_size = lpp_page_round((uint64_t)next_cap * sizeof(int64_t));
        int64_t *next_data = (int64_t *)lpp_sys_mmap(next_size);
        if (!next_data) return;
        for (int64_t i = 0; i < list->len; ++i) next_data[i] = list->data[i];
        if (list->data) lpp_sys_munmap(list->data, list->data_map_size);
        list->data = next_data;
        list->cap = next_cap;
        list->data_map_size = next_size;
    }
    if (list->arc_elements) lpp_arc_retain((void *)(intptr_t)value);
    list->data[list->len++] = value;
}

void lpp_list_push_arc(void *list, void *value) {
    lpp_list_push(list, (int64_t)(intptr_t)value);
}

int64_t lpp_list_get(void *raw, int64_t index) {
    LppList *list = (LppList *)raw;
    if (!list || index < 0 || index >= list->len) return 0;
    return list->data[index];
}

void *lpp_list_get_arc(void *list, int64_t index) {
    return (void *)(intptr_t)lpp_list_get(list, index);
}

int64_t lpp_list_len(void *raw) {
    LppList *list = (LppList *)raw;
    return list ? list->len : 0;
}

void lpp_list_free(void *list) {
    lpp_arc_release(list);
}

/* ── Freestanding File I/O ─────────────────────────────────────────── */

static long lpp_sys_open(const char *path, int flags, int mode) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(2), "D"(path), "S"((long)flags), "d"((long)mode)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_read(long fd, void *buf, long count) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(0), "D"(fd), "S"(buf), "d"(count)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_close(long fd) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(3), "D"(fd)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_lseek(long fd, long offset, int whence) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(8), "D"(fd), "S"(offset), "d"((long)whence)
        : "rcx", "r11", "memory"
    );
    return ret;
}

static long lpp_sys_unlink(const char *path) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(87), "D"(path)
        : "rcx", "r11", "memory"
    );
    return ret;
}

char* lpp_read_file(const char *filename) {
    if (!filename) return (char*)"";
    long fd = lpp_sys_open(filename, 0, 0); /* O_RDONLY */
    if (fd < 0) return (char*)"";
    long len = lpp_sys_lseek(fd, 0, 2); /* SEEK_END */
    if (len < 0) {
        lpp_sys_close(fd);
        return (char*)"";
    }
    (void)lpp_sys_lseek(fd, 0, 0); /* SEEK_SET */
    char *buf = (char*)lpp_arc_alloc(len + 1);
    if (!buf) {
        lpp_sys_close(fd);
        return (char*)"";
    }
    long bytes_read = lpp_sys_read(fd, buf, len);
    lpp_sys_close(fd);
    if (bytes_read < 0) bytes_read = 0;
    buf[bytes_read] = '\0';
    return buf;
}

int64_t lpp_write_file(const char *filename, const char *content) {
    if (!filename || !content) return 0;
    long fd = lpp_sys_open(filename, 0101, 0644); /* O_WRONLY | O_CREAT | O_TRUNC */
    if (fd < 0) return 0;
    long clen = 0;
    while (content[clen]) clen++;
    long written = lpp_sys_write(fd, content, clen);
    lpp_sys_close(fd);
    return written >= 0 ? 1 : 0;
}

int64_t lpp_append_file(const char *filename, const char *content) {
    if (!filename || !content) return 0;
    long fd = lpp_sys_open(filename, 02001, 0644); /* O_WRONLY | O_CREAT | O_APPEND */
    if (fd < 0) return 0;
    long clen = 0;
    while (content[clen]) clen++;
    long written = lpp_sys_write(fd, content, clen);
    lpp_sys_close(fd);
    return written >= 0 ? 1 : 0;
}

int64_t lpp_delete_file(const char *filename) {
    if (!filename) return 0;
    return lpp_sys_unlink(filename) == 0 ? 1 : 0;
}

int64_t lpp_file_exists(const char *filename) {
    if (!filename) return 0;
    long fd = lpp_sys_open(filename, 0, 0);
    if (fd >= 0) {
        lpp_sys_close(fd);
        return 1;
    }
    return 0;
}

int64_t lpp_file_size(const char *filename) {
    if (!filename) return 0;
    long fd = lpp_sys_open(filename, 0, 0);
    if (fd < 0) return 0;
    long sz = lpp_sys_lseek(fd, 0, 2);
    lpp_sys_close(fd);
    return sz >= 0 ? (int64_t)sz : 0;
}

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
    addr.sin_addr = 0; /* INADDR_ANY */
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
