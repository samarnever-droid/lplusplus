/* ── Binary buffer library (runtime/lpp_buf.c) ─────────────────────────────
 * Layout: [8-byte int64_t size][raw data bytes...]
 * A buffer pointer points to the start of the header.
 */

#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

/* ── Core buffer ops ─────────────────────────────────────────────────────── */

void *lpp_buf_alloc(int64_t size) {
    if (size < 0) return NULL;
    uint8_t *buf = (uint8_t *)calloc(1, (size_t)(8 + size));
    if (!buf) return NULL;
    *(int64_t *)buf = size;
    return buf;
}

void lpp_buf_free(void *ptr) {
    free(ptr);
}

int64_t lpp_buf_len(void *ptr) {
    if (!ptr) return 0;
    return *(int64_t *)ptr;
}

int64_t lpp_buf_get8(void *ptr, int64_t offset) {
    if (!ptr) return 0;
    int64_t size = *(int64_t *)ptr;
    if (offset < 0 || offset >= size) return 0;
    return ((uint8_t *)ptr)[8 + offset];
}

void lpp_buf_set8(void *ptr, int64_t offset, int64_t value) {
    if (!ptr) return;
    int64_t size = *(int64_t *)ptr;
    if (offset < 0 || offset >= size) return;
    ((uint8_t *)ptr)[8 + offset] = (uint8_t)(value & 0xFF);
}

void lpp_buf_set32le(void *ptr, int64_t offset, int64_t value) {
    if (!ptr) return;
    int64_t size = *(int64_t *)ptr;
    if (offset < 0 || offset + 4 > size) return;
    uint8_t *base = ((uint8_t *)ptr) + 8 + offset;
    uint32_t v = (uint32_t)value;
    base[0] = (uint8_t)(v);
    base[1] = (uint8_t)(v >> 8);
    base[2] = (uint8_t)(v >> 16);
    base[3] = (uint8_t)(v >> 24);
}

int64_t lpp_buf_get32le(void *ptr, int64_t offset) {
    if (!ptr) return 0;
    int64_t size = *(int64_t *)ptr;
    if (offset < 0 || offset + 4 > size) return 0;
    uint8_t *base = ((uint8_t *)ptr) + 8 + offset;
    return (int64_t)((uint32_t)base[0] | ((uint32_t)base[1] << 8) |
                     ((uint32_t)base[2] << 16) | ((uint32_t)base[3] << 24));
}

void lpp_buf_set16le(void *ptr, int64_t offset, int64_t value) {
    if (!ptr) return;
    int64_t size = *(int64_t *)ptr;
    if (offset < 0 || offset + 2 > size) return;
    uint8_t *base = ((uint8_t *)ptr) + 8 + offset;
    uint16_t v = (uint16_t)value;
    base[0] = (uint8_t)(v);
    base[1] = (uint8_t)(v >> 8);
}

int64_t lpp_buf_get16le(void *ptr, int64_t offset) {
    if (!ptr) return 0;
    int64_t size = *(int64_t *)ptr;
    if (offset < 0 || offset + 2 > size) return 0;
    uint8_t *base = ((uint8_t *)ptr) + 8 + offset;
    return (int64_t)((uint16_t)base[0] | ((uint16_t)base[1] << 8));
}

/* ── Buffer copy / append ────────────────────────────────────────────────── */

void lpp_buf_copy(void *dst, int64_t dst_off, void *src, int64_t src_off, int64_t len) {
    if (!dst || !src) return;
    int64_t dst_size = *(int64_t *)dst;
    int64_t src_size = *(int64_t *)src;
    if (dst_off < 0 || dst_off + len > dst_size) return;
    if (src_off < 0 || src_off + len > src_size) return;
    memcpy(((uint8_t *)dst) + 8 + dst_off, ((uint8_t *)src) + 8 + src_off, (size_t)len);
}

/* ── File I/O ────────────────────────────────────────────────────────────── */

void *lpp_buf_read(const char *path) {
    if (!path) return NULL;
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (sz < 0) { fclose(f); return NULL; }
    void *buf = lpp_buf_alloc((int64_t)sz);
    if (!buf) { fclose(f); return NULL; }
    size_t read = fread(((uint8_t *)buf) + 8, 1, (size_t)sz, f);
    fclose(f);
    if (read != (size_t)sz) {
        lpp_buf_free(buf);
        return NULL;
    }
    return buf;
}

int64_t lpp_buf_write(const char *path, void *ptr) {
    if (!path || !ptr) return -1;
    int64_t size = *(int64_t *)ptr;
    FILE *f = fopen(path, "wb");
    if (!f) return -1;
    size_t written = fwrite(((uint8_t *)ptr) + 8, 1, (size_t)size, f);
    fclose(f);
    return (written == (size_t)size) ? 0 : -1;
}

/* ── CRC32 (IEEE 802.3 polynomial) ───────────────────────────────────────── */

static uint32_t crc32_table[256];
static int crc32_table_ready = 0;

static void crc32_init_table(void) {
    if (crc32_table_ready) return;
    for (uint32_t i = 0; i < 256; i++) {
        uint32_t crc = i;
        for (int j = 0; j < 8; j++) {
            crc = (crc >> 1) ^ ((crc & 1) ? 0xEDB88320UL : 0);
        }
        crc32_table[i] = crc;
    }
    crc32_table_ready = 1;
}

int64_t lpp_buf_crc32(void *ptr, int64_t off, int64_t len) {
    if (!ptr) return 0;
    int64_t size = *(int64_t *)ptr;
    if (off < 0 || len < 0 || off + len > size) return 0;
    crc32_init_table();
    uint32_t crc = 0xFFFFFFFFUL;
    uint8_t *data = ((uint8_t *)ptr) + 8 + off;
    for (int64_t i = 0; i < len; i++) {
        crc = crc32_table[(crc ^ data[i]) & 0xFF] ^ (crc >> 8);
    }
    return (int64_t)(crc ^ 0xFFFFFFFFUL);
}

/* ── String from buffer (for debug/tooling) ──────────────────────────────── */

char *lpp_buf_to_str(void *ptr, int64_t off, int64_t len) {
    if (!ptr) return NULL;
    int64_t size = *(int64_t *)ptr;
    if (off < 0 || len < 0 || off + len > size) return NULL;
    char *s = (char *)malloc((size_t)len + 1);
    if (!s) return NULL;
    memcpy(s, ((uint8_t *)ptr) + 8 + off, (size_t)len);
    s[len] = 0;
    return s;
}

/* Write a C string's bytes (without null terminator) into buffer at offset.
   Returns bytes written, or -1 on bounds error. */
int64_t lpp_buf_write_str(void *ptr, int64_t offset, const char *str) {
    if (!ptr || !str) return -1;
    int64_t size = *(int64_t *)ptr;
    int64_t len = (int64_t)strlen(str);
    if (offset < 0 || offset + len > size) return -1;
    memcpy(((uint8_t *)ptr) + 8 + offset, str, (size_t)len);
    return len;
}

/* Read len bytes from buffer as a newly allocated null-terminated string. */
char *lpp_buf_read_str(void *ptr, int64_t offset, int64_t len) {
    return lpp_buf_to_str(ptr, offset, len);
}
