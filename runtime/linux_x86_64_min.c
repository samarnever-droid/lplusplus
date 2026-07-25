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

void lpp_print_float(double v) {
    char buffer[64];
    char *cursor = buffer + sizeof(buffer);
    *--cursor = '\n';
    int negative = (v < 0.0);
    if (negative) v = -v;
    int64_t ipart = (int64_t)v;
    double fpart = v - (double)ipart;
    int64_t frac = (int64_t)(fpart * 1000000.0 + 0.5);
    for (int i = 0; i < 6; i++) {
        *--cursor = (char)('0' + (frac % 10));
        frac /= 10;
    }
    *--cursor = '.';
    uint64_t magnitude = (uint64_t)ipart;
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

/* ── Floating-point math primitives ──────────────────────────────────────── */
double fmod(double x, double y) {
    if (y == 0.0) return 0.0;
    int64_t i = (int64_t)(x / y);
    return x - (double)i * y;
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

void lpp_list_push_float(void *list, double value) {
    int64_t ival;
    for (int i = 0; i < 8; i++) ((char*)&ival)[i] = ((char*)&value)[i];
    lpp_list_push(list, ival);
}

int64_t lpp_list_get(void *raw, int64_t index) {
    LppList *list = (LppList *)raw;
    if (!list || index < 0 || index >= list->len) return 0;
    return list->data[index];
}

double lpp_list_get_float(void *list, int64_t index) {
    int64_t ival = lpp_list_get(list, index);
    double fval;
    for (int i = 0; i < 8; i++) ((char*)&fval)[i] = ((char*)&ival)[i];
    return fval;
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

/* ── Freestanding Map Runtime ───────────────────────────────────────────── */
typedef struct LppMapEntry {
    int64_t key;
    int64_t val;
    int is_str_key;
    int occupied;
} LppMapEntry;

typedef struct LppMap {
    LppMapEntry *entries;
    int64_t cap;
    int64_t len;
    uint64_t entries_map_size;
} LppMap;

static uint64_t lpp_hash_str(const char *s) {
    if (!s) return 0;
    uint64_t hash = 14695981039346656037ULL;
    while (*s) {
        hash ^= (unsigned char)(*s++);
        hash *= 1099511628211ULL;
    }
    return hash;
}

static uint64_t lpp_hash_int(int64_t key) {
    uint64_t k = (uint64_t)key;
    k = (~k) + (k << 21);
    k = k ^ (k >> 24);
    k = (k + (k << 3)) + (k << 8);
    k = k ^ (k >> 14);
    k = (k + (k << 2)) + (k << 4);
    k = k ^ (k >> 28);
    k = k + (k << 31);
    return k;
}

static int lpp_map_key_equal(int64_t k1, int64_t k2) {
    if (k1 == k2) return 1;
    if (k1 >= 0x400000 && k2 >= 0x400000) {
        const char *s1 = (const char *)(uintptr_t)k1;
        const char *s2 = (const char *)(uintptr_t)k2;
        int i = 0;
        while (s1[i] && s1[i] == s2[i]) i++;
        if (s1[i] == s2[i]) return 1;
    }
    return 0;
}

void lpp_map_destroy(void *payload) {
    LppMap *m = (LppMap *)payload;
    if (!m) return;
    if (m->entries) lpp_sys_munmap(m->entries, m->entries_map_size);
    m->entries = 0;
    m->cap = 0;
    m->len = 0;
}

void *lpp_map_new(void) {
    LppMap *m = (LppMap *)lpp_arc_alloc_with_destructor((int64_t)sizeof(LppMap), lpp_map_destroy);
    if (!m) return 0;
    m->cap = 16;
    m->len = 0;
    m->entries_map_size = lpp_page_round((uint64_t)m->cap * sizeof(LppMapEntry));
    m->entries = (LppMapEntry *)lpp_sys_mmap(m->entries_map_size);
    return m;
}

static void lpp_map_rehash(LppMap *m) {
    int64_t old_cap = m->cap;
    LppMapEntry *old_entries = m->entries;
    uint64_t old_size = m->entries_map_size;

    m->cap = old_cap * 2;
    m->entries_map_size = lpp_page_round((uint64_t)m->cap * sizeof(LppMapEntry));
    m->entries = (LppMapEntry *)lpp_sys_mmap(m->entries_map_size);
    m->len = 0;

    for (int64_t i = 0; i < old_cap; i++) {
        if (old_entries[i].occupied == 1) {
            int64_t key = old_entries[i].key;
            int64_t val = old_entries[i].val;
            int is_str = old_entries[i].is_str_key;
            uint64_t h = is_str ? lpp_hash_str((const char *)(uintptr_t)key) : lpp_hash_int(key);
            int64_t idx = (int64_t)(h % (uint64_t)m->cap);
            while (m->entries[idx].occupied == 1) {
                idx = (idx + 1) % m->cap;
            }
            m->entries[idx].key = key;
            m->entries[idx].val = val;
            m->entries[idx].is_str_key = is_str;
            m->entries[idx].occupied = 1;
            m->len++;
        }
    }
    if (old_entries) lpp_sys_munmap(old_entries, old_size);
}

static void lpp_map_put_internal(LppMap *m, int64_t key, int64_t val, int is_str) {
    if (!m) return;
    if (m->len * 10 >= m->cap * 7) {
        lpp_map_rehash(m);
    }

    uint64_t h = is_str ? lpp_hash_str((const char *)(uintptr_t)key) : lpp_hash_int(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t first_tombstone = -1;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == is_str) {
            int match = is_str
                ? lpp_map_key_equal(m->entries[idx].key, key)
                : (m->entries[idx].key == key);
            if (match) {
                m->entries[idx].val = val;
                return;
            }
        }
        if (m->entries[idx].occupied == 2 && first_tombstone == -1) {
            first_tombstone = idx;
        }
        idx = (idx + 1) % m->cap;
    }

    if (first_tombstone != -1) {
        idx = first_tombstone;
    }

    m->entries[idx].key = key;
    m->entries[idx].val = val;
    m->entries[idx].is_str_key = is_str;
    m->entries[idx].occupied = 1;
    m->len++;
}

void lpp_map_put(void *map, int64_t key, int64_t val) {
    lpp_map_put_internal((LppMap *)map, key, val, 0);
}

void lpp_map_put_str(void *map, const char *key, int64_t val) {
    lpp_map_put_internal((LppMap *)map, (int64_t)(uintptr_t)key, val, 1);
}

int64_t lpp_map_get(void *map, int64_t key) {
    LppMap *m = (LppMap *)map;
    if (!m || m->len == 0) return 0;

    uint64_t h = lpp_hash_int(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) {
            return m->entries[idx].val;
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
    return 0;
}

int64_t lpp_map_get_str(void *map, const char *key) {
    LppMap *m = (LppMap *)map;
    if (!m || !key || m->len == 0) return 0;

    uint64_t h = lpp_hash_str(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) {
            if (lpp_map_key_equal(m->entries[idx].key, (int64_t)(uintptr_t)key)) {
                return m->entries[idx].val;
            }
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
    return 0;
}

int64_t lpp_map_has(void *map, int64_t key) {
    LppMap *m = (LppMap *)map;
    if (!m || m->len == 0) return 0;

    uint64_t h = lpp_hash_int(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) {
            return 1;
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
    return 0;
}

int64_t lpp_map_has_str(void *map, const char *key) {
    LppMap *m = (LppMap *)map;
    if (!m || !key || m->len == 0) return 0;

    uint64_t h = lpp_hash_str(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) {
            if (lpp_map_key_equal(m->entries[idx].key, (int64_t)(uintptr_t)key)) {
                return 1;
            }
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
    return 0;
}

int64_t lpp_map_len(void *map) {
    LppMap *m = (LppMap *)map;
    return m ? m->len : 0;
}

void lpp_map_remove(void *map, int64_t key) {
    LppMap *m = (LppMap *)map;
    if (!m || m->len == 0) return;

    uint64_t h = lpp_hash_int(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) {
            m->entries[idx].occupied = 2;
            m->len--;
            return;
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
}

void lpp_map_remove_str(void *map, const char *key) {
    LppMap *m = (LppMap *)map;
    if (!m || !key || m->len == 0) return;

    uint64_t h = lpp_hash_str(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) {
            if (lpp_map_key_equal(m->entries[idx].key, (int64_t)(uintptr_t)key)) {
                m->entries[idx].occupied = 2;
                m->len--;
                return;
            }
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
}

void lpp_map_put_float(void *map, int64_t key, double val) {
    int64_t ival;
    for (int i = 0; i < 8; i++) ((char*)&ival)[i] = ((char*)&val)[i];
    lpp_map_put(map, key, ival);
}

double lpp_map_get_float(void *map, int64_t key) {
    int64_t ival = lpp_map_get(map, key);
    double fval;
    for (int i = 0; i < 8; i++) ((char*)&fval)[i] = ((char*)&ival)[i];
    return fval;
}

void lpp_map_put_str_float(void *map, const char *key, double val) {
    int64_t ival;
    for (int i = 0; i < 8; i++) ((char*)&ival)[i] = ((char*)&val)[i];
    lpp_map_put_str(map, key, ival);
}

double lpp_map_get_str_float(void *map, const char *key) {
    int64_t ival = lpp_map_get_str(map, key);
    double fval;
    for (int i = 0; i < 8; i++) ((char*)&fval)[i] = ((char*)&ival)[i];
    return fval;
}

/* ── String builtins (freestanding, using lpp_alloc/lpp_sys_mmap) ── */

static int64_t lpp_strlen(const char *s) {
    if (!s) return 0;
    int64_t n = 0;
    while (s[n]) n++;
    return n;
}

int64_t lpp_str_len(const char *s) {
    return lpp_strlen(s);
}

char *lpp_str_concat(const char *a, const char *b) {
    int64_t alen = lpp_strlen(a);
    int64_t blen = lpp_strlen(b);
    char *out = (char *)lpp_alloc(alen + blen + 1);
    for (int64_t i = 0; i < alen; i++) out[i] = a[i];
    for (int64_t i = 0; i < blen; i++) out[alen + i] = b[i];
    out[alen + blen] = 0;
    return out;
}

char *lpp_str_substr(const char *s, int64_t start, int64_t length) {
    if (!s) return (char *)lpp_alloc(1);
    int64_t slen = lpp_strlen(s);
    if (start < 0) start = 0;
    if (start >= slen || length <= 0) {
        char *out = (char *)lpp_alloc(1);
        out[0] = 0;
        return out;
    }
    if (start + length > slen) length = slen - start;
    char *out = (char *)lpp_alloc(length + 1);
    for (int64_t i = 0; i < length; i++) out[i] = s[start + i];
    out[length] = 0;
    return out;
}

char *lpp_str_repeat(const char *s, int64_t n) {
    if (!s || n <= 0) { char *e = (char *)lpp_alloc(1); e[0] = 0; return e; }
    int64_t slen = lpp_strlen(s);
    int64_t total = slen * n;
    char *out = (char *)lpp_alloc(total + 1);
    for (int64_t i = 0; i < n; i++)
        for (int64_t j = 0; j < slen; j++)
            out[i * slen + j] = s[j];
    out[total] = 0;
    return out;
}

char *lpp_char_at(const char *s, int64_t idx) {
    if (!s) return (char *)lpp_alloc(1);
    int64_t slen = lpp_strlen(s);
    if (idx < 0 || idx >= slen) return (char *)lpp_alloc(1);
    char *out = (char *)lpp_alloc(2);
    out[0] = s[idx];
    out[1] = 0;
    return out;
}

int64_t lpp_ord(const char *s) {
    if (!s || !s[0]) return 0;
    return (int64_t)(unsigned char)s[0];
}

char *lpp_chr(int64_t code) {
    char *out = (char *)lpp_alloc(2);
    out[0] = (char)(code & 0xFF);
    out[1] = 0;
    return out;
}

int64_t lpp_str_find(const char *haystack, const char *needle) {
    if (!haystack || !needle) return -1;
    int64_t hlen = lpp_strlen(haystack);
    int64_t nlen = lpp_strlen(needle);
    if (nlen == 0) return 0;
    if (nlen > hlen) return -1;
    for (int64_t i = 0; i <= hlen - nlen; i++) {
        int64_t j = 0;
        while (j < nlen && haystack[i + j] == needle[j]) j++;
        if (j == nlen) return i;
    }
    return -1;
}

int64_t lpp_str_contains(const char *haystack, const char *needle) {
    return lpp_str_find(haystack, needle) >= 0 ? 1 : 0;
}

int64_t lpp_str_starts_with(const char *s, const char *prefix) {
    if (!s || !prefix) return 0;
    int64_t plen = lpp_strlen(prefix);
    for (int64_t i = 0; i < plen; i++) {
        if (s[i] != prefix[i] || s[i] == 0) return 0;
    }
    return 1;
}

int64_t lpp_str_ends_with(const char *s, const char *suffix) {
    if (!s || !suffix) return 0;
    int64_t slen = lpp_strlen(s);
    int64_t xlen = lpp_strlen(suffix);
    if (xlen > slen) return 0;
    for (int64_t i = 0; i < xlen; i++) {
        if (s[slen - xlen + i] != suffix[i]) return 0;
    }
    return 1;
}

char *lpp_str_upper(const char *s) {
    if (!s) return (char *)lpp_alloc(1);
    int64_t len = lpp_strlen(s);
    char *out = (char *)lpp_alloc(len + 1);
    for (int64_t i = 0; i < len; i++)
        out[i] = (s[i] >= 'a' && s[i] <= 'z') ? s[i] - 32 : s[i];
    out[len] = 0;
    return out;
}

char *lpp_str_lower(const char *s) {
    if (!s) return (char *)lpp_alloc(1);
    int64_t len = lpp_strlen(s);
    char *out = (char *)lpp_alloc(len + 1);
    for (int64_t i = 0; i < len; i++)
        out[i] = (s[i] >= 'A' && s[i] <= 'Z') ? s[i] + 32 : s[i];
    out[len] = 0;
    return out;
}

char *lpp_str_trim(const char *s) {
    if (!s) return (char *)lpp_alloc(1);
    int64_t len = lpp_strlen(s);
    int64_t start = 0, end = len;
    while (start < len && (s[start] == ' ' || s[start] == '\t' || s[start] == '\n' || s[start] == '\r')) start++;
    while (end > start && (s[end-1] == ' ' || s[end-1] == '\t' || s[end-1] == '\n' || s[end-1] == '\r')) end--;
    int64_t rlen = end - start;
    char *out = (char *)lpp_alloc(rlen + 1);
    for (int64_t i = 0; i < rlen; i++) out[i] = s[start + i];
    out[rlen] = 0;
    return out;
}

char *lpp_str_replace(const char *s, const char *old, const char *new_) {
    if (!s || !old || !new_) return (char *)lpp_alloc(1);
    int64_t slen = lpp_strlen(s);
    int64_t olen = lpp_strlen(old);
    int64_t nlen = lpp_strlen(new_);
    if (olen == 0) { /* copy */ char *out = (char *)lpp_alloc(slen + 1); for (int64_t i = 0; i <= slen; i++) out[i] = s[i]; return out; }
    /* count occurrences */
    int64_t count = 0;
    for (int64_t i = 0; i <= slen - olen; i++) {
        int64_t j = 0;
        while (j < olen && s[i+j] == old[j]) j++;
        if (j == olen) { count++; i += olen - 1; }
    }
    int64_t rlen = slen + count * (nlen - olen);
    char *out = (char *)lpp_alloc(rlen + 1);
    int64_t w = 0;
    for (int64_t i = 0; i < slen; ) {
        int64_t j = 0;
        if (i <= slen - olen) { while (j < olen && s[i+j] == old[j]) j++; }
        if (j == olen) {
            for (int64_t k = 0; k < nlen; k++) out[w++] = new_[k];
            i += olen;
        } else {
            out[w++] = s[i++];
        }
    }
    out[rlen] = 0;
    return out;
}

char *lpp_int_to_str(int64_t val) {
    char buf[24];
    int neg = val < 0;
    if (neg) val = -val;
    int i = 23;
    buf[i] = 0;
    do { buf[--i] = '0' + (val % 10); val /= 10; } while (val);
    if (neg) buf[--i] = '-';
    int64_t len = 23 - i;
    char *out = (char *)lpp_alloc(len + 1);
    for (int64_t j = 0; j <= len; j++) out[j] = buf[i + j];
    return out;
}

int64_t lpp_str_to_int(const char *s) {
    if (!s) return 0;
    int64_t val = 0, neg = 0;
    int64_t i = 0;
    while (s[i] == ' ' || s[i] == '\t') i++;
    if (s[i] == '-') { neg = 1; i++; }
    else if (s[i] == '+') i++;
    while (s[i] >= '0' && s[i] <= '9') { val = val * 10 + (s[i] - '0'); i++; }
    return neg ? -val : val;
}

const char *lpp_input(void) {
    /* stub: freestanding has no stdin */
    char *out = (char *)lpp_alloc(1);
    out[0] = 0;
    return out;
}
