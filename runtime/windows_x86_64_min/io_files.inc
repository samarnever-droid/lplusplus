/* ── I/O builtins using Kernel32 (no CRT) ── */

#ifndef GENERIC_READ
#define GENERIC_READ    0x80000000
#define GENERIC_WRITE   0x40000000
#define FILE_SHARE_READ 0x00000001
#define CREATE_ALWAYS   2
#define OPEN_EXISTING   3
#define OPEN_ALWAYS     4
#define FILE_APPEND_DATA 0x00000004
#define FILE_ATTRIBUTE_NORMAL 0x80
#endif

const char *lpp_input(void) {
    HANDLE h = GetStdHandle((DWORD)-10);
    char buf[4096];
    DWORD rd = 0;
    ReadFile(h, buf, sizeof(buf)-1, &rd, 0);
    if (rd > 0 && buf[rd-1] == '\n') rd--;
    if (rd > 0 && buf[rd-1] == '\r') rd--;
    char *out = (char *)lpp_arc_alloc((int64_t)rd + 1);
    lpp_memcpy(out, buf, (int)rd);
    out[rd] = 0;
    return out;
}

int64_t lpp_write_file(const char *path, const char *data) {
    if (!path || !data) return -1;
    HANDLE h = CreateFileA(path, GENERIC_WRITE, 0, 0, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    if (h == INVALID_HANDLE_VALUE) return -1;
    DWORD written = 0;
    int ln = lpp_strlen(data);
    BOOL ok = WriteFile(h, data, (DWORD)ln, &written, 0);
    CloseHandle(h);
    return ok && written == (DWORD)ln ? 0 : -1;
}

char *lpp_read_file(const char *path) {
    HANDLE h = CreateFileA(path, GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0);
    if (h == INVALID_HANDLE_VALUE) { char *e = (char*)lpp_arc_alloc(1); e[0]=0; return e; }
    DWORD size = GetFileSize(h, 0);
    char *buf = (char *)lpp_arc_alloc((int64_t)size + 1);
    DWORD rd = 0;
    ReadFile(h, buf, size, &rd, 0);
    buf[rd] = 0;
    CloseHandle(h);
    return buf;
}

int64_t lpp_append_file(const char *path, const char *data) {
    if (!path || !data) return -1;
    HANDLE h = CreateFileA(path, FILE_APPEND_DATA, 0, 0, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    if (h == INVALID_HANDLE_VALUE) return -1;
    DWORD written = 0;
    int ln = lpp_strlen(data);
    BOOL ok = WriteFile(h, data, (DWORD)ln, &written, 0);
    CloseHandle(h);
    return ok && written == (DWORD)ln ? 0 : -1;
}

int64_t lpp_delete_file(const char *path) { return DeleteFileA(path) ? 0 : -1; }
int64_t lpp_file_move(const char *source, const char *destination) { if (!source || !destination) return -1; return MoveFileA(source, destination) ? 0 : -1; }
int64_t lpp_file_exists(const char *path) { DWORD a = GetFileAttributesA(path); return (a != ((DWORD)-1)) ? 1 : 0; }
int64_t lpp_file_size(const char *path) { HANDLE h = CreateFileA(path, GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0); if (h == INVALID_HANDLE_VALUE) return -1; DWORD sz = GetFileSize(h, 0); CloseHandle(h); return (int64_t)sz; }

/* ── Buffers: binary-safe byte arrays ──────────────────────────────────────
 * Same layout convention as linux_x86_64_min.c and runtime/lpp_buf.c:
 * [8-byte int64 size][data bytes...]; user pointer points at the data,
 * the size header lives 8 bytes before it. Bounds-checked like lpp_buf.c.
 */
void *lpp_buf_alloc(int64_t size) { if (size <= 0) size = 64; uint64_t t = lpp_page_round((uint64_t)size + 8); char *m = (char*)VirtualAlloc(0, t, MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE); if (!m) return 0; *(int64_t*)m = size; return m + 8; }
void lpp_buf_free(void *p) { if (!p) return; VirtualFree((char*)p - 8, 0, MEM_RELEASE); }
/* The OS reclaims stale-context tombstones when this freestanding process exits. */
void lpp_buf_free_at_exit(int64_t p) { (void)p; }
int64_t lpp_buf_len(void *p) { if (!p) return 0; return *(int64_t*)((char*)p - 8); }
int64_t lpp_buf_get8(void *p, int64_t o) { if (!p) return 0; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o >= s) return 0; return (int64_t)(unsigned char)((char*)p)[o]; }
void lpp_buf_set8(void *p, int64_t o, int64_t v) { if (!p) return; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o >= s) return; ((char*)p)[o] = (char)(v & 0xFF); }
void lpp_buf_set16le(void *p, int64_t o, int64_t v) { if (!p) return; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 2 > s) return; char *d = (char*)p + o; d[0] = (char)(v & 0xFF); d[1] = (char)((v >> 8) & 0xFF); }
int64_t lpp_buf_get16le(void *p, int64_t o) { if (!p) return 0; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 2 > s) return 0; unsigned char *d = (unsigned char*)((char*)p + o); return (int64_t)d[0] | ((int64_t)d[1] << 8); }
void lpp_buf_set16be(void *p, int64_t o, int64_t v) { if (!p) return; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 2 > s) return; char *d = (char*)p + o; d[0] = (char)((v >> 8) & 0xFF); d[1] = (char)(v & 0xFF); }
int64_t lpp_buf_get16be(void *p, int64_t o) { if (!p) return 0; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 2 > s) return 0; unsigned char *d = (unsigned char*)((char*)p + o); return ((int64_t)d[0] << 8) | (int64_t)d[1]; }
void lpp_buf_set32le(void *p, int64_t o, int64_t v) { if (!p) return; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 4 > s) return; char *d = (char*)p + o; d[0] = (char)(v & 0xFF); d[1] = (char)((v >> 8) & 0xFF); d[2] = (char)((v >> 16) & 0xFF); d[3] = (char)((v >> 24) & 0xFF); }
int64_t lpp_buf_get32le(void *p, int64_t o) { if (!p) return 0; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 4 > s) return 0; unsigned char *d = (unsigned char*)((char*)p + o); return (int64_t)d[0] | ((int64_t)d[1] << 8) | ((int64_t)d[2] << 16) | ((int64_t)d[3] << 24); }
void lpp_buf_set32be(void *p, int64_t o, int64_t v) { if (!p) return; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 4 > s) return; char *d = (char*)p + o; d[0] = (char)((v >> 24) & 0xFF); d[1] = (char)((v >> 16) & 0xFF); d[2] = (char)((v >> 8) & 0xFF); d[3] = (char)(v & 0xFF); }
int64_t lpp_buf_get32be(void *p, int64_t o) { if (!p) return 0; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 4 > s) return 0; unsigned char *d = (unsigned char*)((char*)p + o); return ((int64_t)d[0] << 24) | ((int64_t)d[1] << 16) | ((int64_t)d[2] << 8) | (int64_t)d[3]; }
void lpp_buf_set64le(void *p, int64_t o, int64_t v) { if (!p) return; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 8 > s) return; char *d = (char*)p + o; for(int i=0; i<8; i++) d[i] = (char)((((uint64_t)v) >> (i * 8)) & 0xFF); }
int64_t lpp_buf_get64le(void *p, int64_t o) { if (!p) return 0; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 8 > s) return 0; unsigned char *d = (unsigned char*)((char*)p + o); uint64_t r = 0; for(int i=0; i<8; i++) r |= ((uint64_t)d[i]) << (i * 8); return (int64_t)r; }
void lpp_buf_set64be(void *p, int64_t o, int64_t v) { if (!p) return; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 8 > s) return; char *d = (char*)p + o; for(int i=0; i<8; i++) d[7 - i] = (char)((((uint64_t)v) >> (i * 8)) & 0xFF); }
int64_t lpp_buf_get64be(void *p, int64_t o) { if (!p) return 0; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 8 > s) return 0; unsigned char *d = (unsigned char*)((char*)p + o); uint64_t r = 0; for(int i=0; i<8; i++) r |= ((uint64_t)d[7 - i]) << (i * 8); return (int64_t)r; }
void lpp_buf_copy(void *dst, int64_t doff, void *src, int64_t soff, int64_t len) { if (!dst || !src || len <= 0) return; int64_t ds = *(int64_t*)((char*)dst - 8), ss = *(int64_t*)((char*)src - 8); if (doff < 0 || doff + len > ds || soff < 0 || soff + len > ss) return; char *d = (char*)dst + doff; char *s = (char*)src + soff; for (int64_t i = 0; i < len; i++) d[i] = s[i]; }
void *lpp_buf_read(const char *path) { if (!path) return 0; HANDLE h = CreateFileA(path, GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0); if (h == INVALID_HANDLE_VALUE) return 0; DWORD size = GetFileSize(h, 0); void *buf = lpp_buf_alloc((int64_t)size); if (!buf) { CloseHandle(h); return 0; } DWORD rd = 0, total = 0; while (total < size) { if (!ReadFile(h, (char*)buf + total, size - total, &rd, 0) || rd == 0) break; total += rd; } CloseHandle(h); if (total != size) { lpp_buf_free(buf); return 0; } return buf; }
int64_t lpp_buf_write(const char *path, void *p) { if (!path || !p) return -1; int64_t size = *(int64_t*)((char*)p - 8); HANDLE h = CreateFileA(path, GENERIC_WRITE, 0, 0, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0); if (h == INVALID_HANDLE_VALUE) return -1; DWORD written = 0, total = 0; while ((int64_t)total < size) { if (!WriteFile(h, (char*)p + total, (DWORD)(size - (int64_t)total), &written, 0) || written == 0) { CloseHandle(h); return -1; } total += written; } CloseHandle(h); return 0; }
char *lpp_buf_read_str(void *p, int64_t o, int64_t len) { if (!p || len <= 0) { char *e = (char*)lpp_alloc(1); e[0] = 0; return e; } int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + len > s) { char *e = (char*)lpp_alloc(1); e[0] = 0; return e; } char *out = (char*)lpp_alloc(len + 1); char *src = (char*)p + o; for (int64_t i = 0; i < len; i++) out[i] = src[i]; out[len] = 0; return out; }
int64_t lpp_buf_write_str(void *p, int64_t o, const char *str) { if (!p || !str) return -1; int64_t n = 0; while (str[n]) n++; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + n > s) return -1; char *d = (char*)p + o; for (int64_t i = 0; i < n; i++) d[i] = str[i]; return n; }
int64_t lpp_buf_crc32(void *p, int64_t off, int64_t len) { if (!p || len < 0) return 0; int64_t s = *(int64_t*)((char*)p - 8); if (off < 0 || off + len > s) return 0; const unsigned char *d = (const unsigned char*)((char*)p + off); uint32_t crc = 0xFFFFFFFFu; for (int64_t i = 0; i < len; i++) { crc ^= d[i]; for (int b = 0; b < 8; b++) { uint32_t m = ~(crc & 1u) + 1u; crc = (crc >> 1) ^ (0xEDB88320u & m); } } return (int64_t)(uint32_t)(~crc); }

/* ── Positional Durable File I/O (P0.4) ── */
int64_t lpp_file_open(const char *path, int64_t flags) {
    if (!path) return -1;
    DWORD access = GENERIC_READ | GENERIC_WRITE;
    DWORD creation = OPEN_ALWAYS;
    DWORD attr = FILE_ATTRIBUTE_NORMAL;
    if (flags & 1) attr |= FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH; // direct/sync
    HANDLE h = CreateFileA(path, access, FILE_SHARE_READ | FILE_SHARE_WRITE, 0, creation, attr, 0);
    if (h == INVALID_HANDLE_VALUE) return -1;
    return (int64_t)(intptr_t)h;
}

void lpp_file_close(int64_t fd) {
    if (fd <= 0 || (HANDLE)(intptr_t)fd == INVALID_HANDLE_VALUE) return;
    CloseHandle((HANDLE)(intptr_t)fd);
}

int64_t lpp_file_pread(int64_t fd, void *buf, int64_t buf_off, int64_t file_off, int64_t len) {
    if (fd <= 0 || !buf || len <= 0 || file_off < 0 || buf_off < 0) return -1;
    int64_t buf_size = *(int64_t*)((char*)buf - 8);
    if (buf_off + len > buf_size) return -1;
    HANDLE h = (HANDLE)(intptr_t)fd;
    OVERLAPPED ov;
    memset(&ov, 0, sizeof(ov));
    ov.Offset = (DWORD)(file_off & 0xFFFFFFFF);
    ov.OffsetHigh = (DWORD)((file_off >> 32) & 0xFFFFFFFF);
    DWORD bytes_read = 0;
    if (!ReadFile(h, (char*)buf + buf_off, (DWORD)len, &bytes_read, &ov)) {
        DWORD err = GetLastError();
        if (err == ERROR_HANDLE_EOF) return (int64_t)bytes_read;
        return -1;
    }
    return (int64_t)bytes_read;
}

int64_t lpp_file_pwrite(int64_t fd, void *buf, int64_t buf_off, int64_t file_off, int64_t len) {
    if (fd <= 0 || !buf || len <= 0 || file_off < 0 || buf_off < 0) return -1;
    int64_t buf_size = *(int64_t*)((char*)buf - 8);
    if (buf_off + len > buf_size) return -1;
    HANDLE h = (HANDLE)(intptr_t)fd;
    OVERLAPPED ov;
    memset(&ov, 0, sizeof(ov));
    ov.Offset = (DWORD)(file_off & 0xFFFFFFFF);
    ov.OffsetHigh = (DWORD)((file_off >> 32) & 0xFFFFFFFF);
    DWORD written = 0;
    if (!WriteFile(h, (char*)buf + buf_off, (DWORD)len, &written, &ov)) {
        return -1;
    }
    return (int64_t)written;
}

int64_t lpp_file_fsync(int64_t fd) {
    if (fd <= 0 || (HANDLE)(intptr_t)fd == INVALID_HANDLE_VALUE) return -1;
    if (!FlushFileBuffers((HANDLE)(intptr_t)fd)) return -1;
    return 0;
}

int64_t lpp_file_fdatasync(int64_t fd) {
    return lpp_file_fsync(fd);
}

int64_t lpp_file_ftruncate(int64_t fd, int64_t size) {
    if (fd <= 0 || size < 0) return -1;
    HANDLE h = (HANDLE)(intptr_t)fd;
    LARGE_INTEGER li;
    li.QuadPart = size;
    if (!SetFilePointerEx(h, li, 0, FILE_BEGIN)) return -1;
    if (!SetEndOfFile(h)) return -1;
    return 0;
}

int64_t lpp_fd_size(int64_t fd) {
    if (fd <= 0) return -1;
    HANDLE h = (HANDLE)(intptr_t)fd;
    LARGE_INTEGER li;
    if (!GetFileSizeEx(h, &li)) return -1;
    return (int64_t)li.QuadPart;
}

