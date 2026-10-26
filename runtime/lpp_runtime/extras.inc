/* Scalar reference plus an optional 256-bit AVX2 implementation for the
 * explicit checksum intrinsic. Dispatch only after a runtime CPU-feature
 * check; never execute AVX2 merely because the compiler can emit it. */
#if defined(__GNUC__) && defined(__x86_64__)
typedef int64_t lpp_i64x4 __attribute__((vector_size(32)));
__attribute__((target("avx2")))
static int64_t lpp_vec_i64_checksum_avx2(int64_t n) {
    int64_t i = 0;
    const lpp_i64x4 three = {3, 3, 3, 3};
    lpp_i64x4 total = {0, 0, 0, 0};
    while (n - i >= 4) {
        lpp_i64x4 v = {i, i + 1, i + 2, i + 3};
        lpp_i64x4 x = (v * three) ^ (v >> 1);
        total += x;
        i += 4;
    }
    int64_t sum = total[0] + total[1] + total[2] + total[3];
    for (; i < n; ++i) sum += (i * 3) ^ (i >> 1);
    return sum;
}
#endif
int64_t lpp_vec_i64_checksum(int64_t n) {
    if (n < 0) return 0;
#if defined(__GNUC__) && defined(__x86_64__)
    if (n >= 64 && __builtin_cpu_supports("avx2")) {
        return lpp_vec_i64_checksum_avx2(n);
    }
#endif
    int64_t total = 0;
    for (int64_t i = 0; i < n; ++i) total += (i * 3) ^ (i >> 1);
    return total;
}

void lpp_prefetch(int64_t addr) {
#if defined(LPP_HAS_X86_SSE2)
    if (addr) _mm_prefetch((const char *)(intptr_t)addr, _MM_HINT_T0);
#elif defined(__GNUC__) || defined(__clang__)
    if (addr) __builtin_prefetch((const void *)(intptr_t)addr, 0, 3);
#endif
}

void lpp_prefetch_write(int64_t addr) {
#if defined(LPP_HAS_X86_SSE2)
    if (addr) _mm_prefetch((const char *)(intptr_t)addr, _MM_HINT_T0);
#elif defined(__GNUC__) || defined(__clang__)
    if (addr) __builtin_prefetch((const void *)(intptr_t)addr, 1, 3);
#endif
}

typedef struct {
    int64_t v[2];
} LppVec128;

LppVec128 lpp_vec_i64x2_and(LppVec128 a, LppVec128 b) {
    LppVec128 r;
    r.v[0] = a.v[0] & b.v[0];
    r.v[1] = a.v[1] & b.v[1];
    return r;
}

LppVec128 lpp_vec_i64x2_or(LppVec128 a, LppVec128 b) {
    LppVec128 r;
    r.v[0] = a.v[0] | b.v[0];
    r.v[1] = a.v[1] | b.v[1];
    return r;
}

LppVec128 lpp_vec_i64x2_not(LppVec128 a) {
    LppVec128 r;
    r.v[0] = ~a.v[0];
    r.v[1] = ~a.v[1];
    return r;
}

LppVec128 lpp_vec_u8x16_splat(int64_t byte_val) {
    uint8_t b = (uint8_t)byte_val;
    uint64_t half = 0;
    for (int i = 0; i < 8; i++) half |= ((uint64_t)b << (i * 8));
    LppVec128 r;
    r.v[0] = (int64_t)half;
    r.v[1] = (int64_t)half;
    return r;
}

LppVec128 lpp_vec_u8x16_eq(LppVec128 a, LppVec128 b) {
#if defined(LPP_HAS_X86_SSE2)
    __m128i va = _mm_loadu_si128((const __m128i *)&a);
    __m128i vb = _mm_loadu_si128((const __m128i *)&b);
    __m128i eq = _mm_cmpeq_epi8(va, vb);
    LppVec128 r;
    _mm_storeu_si128((__m128i *)&r, eq);
    return r;
#else
    uint8_t *pa = (uint8_t *)&a;
    uint8_t *pb = (uint8_t *)&b;
    LppVec128 r;
    uint8_t *pr = (uint8_t *)&r;
    for (int i = 0; i < 16; i++) {
        pr[i] = (pa[i] == pb[i]) ? 0xFF : 0x00;
    }
    return r;
#endif
}

int64_t lpp_vec_u8x16_movemask(LppVec128 a) {
#if defined(LPP_HAS_X86_SSE2)
    __m128i va = _mm_loadu_si128((const __m128i *)&a);
    return (int64_t)_mm_movemask_epi8(va);
#else
    uint8_t *pa = (uint8_t *)&a;
    int64_t mask = 0;
    for (int i = 0; i < 16; i++) {
        if (pa[i] & 0x80) mask |= (1 << i);
    }
    return mask;
#endif
}

int64_t lpp_command_exec(int64_t cmd_ptr) {
    if (!cmd_ptr) return -1;
    const char *cmd = (const char *)(intptr_t)cmd_ptr;
    int res = system(cmd);
    return (int64_t)res;
}

int64_t lpp_command_output(int64_t cmd_ptr) {
    if (!cmd_ptr) {
        char *empty = (char *)lpp_arc_alloc(1);
        empty[0] = '\0';
        return (int64_t)(intptr_t)empty;
    }
    const char *cmd = (const char *)(intptr_t)cmd_ptr;
    
#if defined(_WIN32)
    FILE *fp = _popen(cmd, "r");
#else
    FILE *fp = popen(cmd, "r");
#endif

    if (!fp) {
        char *empty = (char *)lpp_arc_alloc(1);
        empty[0] = '\0';
        return (int64_t)(intptr_t)empty;
    }
    
    size_t cap = 1024;
    size_t len = 0;
    char *buf = (char *)lpp_arc_alloc((int64_t)cap);
    if (!buf) {
#if defined(_WIN32)
        _pclose(fp);
#else
        pclose(fp);
#endif
        return 0;
    }
    
    while (1) {
        if (len + 256 > cap) {
            if (cap > (size_t)INT64_MAX / 2) break;   /* cap *= 2 overflow guard */
            cap *= 2;
            char *new_buf = (char *)lpp_arc_alloc((int64_t)cap);
            if (!new_buf) break;
            memcpy(new_buf, buf, len);
            lpp_arc_release(buf);
            buf = new_buf;
        }
        size_t n = fread(buf + len, 1, 256, fp);
        if (n == 0) break;
        len += n;
    }
    buf[len] = '\0';
    
#if defined(_WIN32)
    _pclose(fp);
#else
    pclose(fp);
#endif
    return (int64_t)(intptr_t)buf;
}

/* C pointer intrinsics & dlfcn compatibility layer */
int64_t lpp_c_malloc(int64_t size) { return (int64_t)(intptr_t)malloc((size_t)size); }
void lpp_c_free(int64_t ptr) { free((void*)(intptr_t)ptr); }
uint8_t lpp_c_load_u8(int64_t ptr) { return *(uint8_t*)(intptr_t)ptr; }
void lpp_c_store_u8(int64_t ptr, uint8_t val) { *(uint8_t*)(intptr_t)ptr = val; }
int32_t lpp_c_load_i32(int64_t ptr) { return *(int32_t*)(intptr_t)ptr; }
void lpp_c_store_i32(int64_t ptr, int32_t val) { *(int32_t*)(intptr_t)ptr = val; }
int64_t lpp_c_load_i64(int64_t ptr) { return *(int64_t*)(intptr_t)ptr; }
void lpp_c_store_i64(int64_t ptr, int64_t val) { *(int64_t*)(intptr_t)ptr = val; }

#if defined(_WIN32)
void *dlopen(const char *filename, int flags) { (void)flags; return (void*)LoadLibraryA(filename); }
void *dlsym(void *handle, const char *symbol) { return (void*)GetProcAddress((HMODULE)handle, symbol); }
int dlclose(void *handle) { return FreeLibrary((HMODULE)handle) ? 0 : -1; }
const char *dlerror(void) { return "dlerror not implemented on Windows"; }
#endif

/* ── Positional Durable File I/O (P0.4) ── */
int64_t lpp_file_open(const char *path, int64_t flags) {
    if (!path) return -1;
#if defined(_WIN32)
    DWORD access = GENERIC_READ | GENERIC_WRITE;
    DWORD creation = OPEN_ALWAYS;
    DWORD attr = FILE_ATTRIBUTE_NORMAL;
    if (flags & 1) attr |= FILE_FLAG_NO_BUFFERING | FILE_FLAG_WRITE_THROUGH;
    HANDLE h = CreateFileA(path, access, FILE_SHARE_READ | FILE_SHARE_WRITE, 0, creation, attr, 0);
    if (h == INVALID_HANDLE_VALUE) return -1;
    return (int64_t)(intptr_t)h;
#else
    int f = O_RDWR | O_CREAT;
    if (flags & 1) f |= O_SYNC;
    int fd = open(path, f, 0644);
    return (int64_t)fd;
#endif
}

void lpp_file_close(int64_t fd) {
    if (fd <= 0) return;
#if defined(_WIN32)
    CloseHandle((HANDLE)(intptr_t)fd);
#else
    close((int)fd);
#endif
}

int64_t lpp_file_pread(int64_t fd, void *buf, int64_t buf_off, int64_t file_off, int64_t len) {
    if (fd <= 0 || !buf || len <= 0 || file_off < 0 || buf_off < 0) return -1;
    int64_t buf_size = *(int64_t*)((char*)buf - 8);
    if (buf_off + len > buf_size) return -1;
#if defined(_WIN32)
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
#else
    ssize_t res = pread((int)fd, (char*)buf + buf_off, (size_t)len, (off_t)file_off);
    return (int64_t)res;
#endif
}

int64_t lpp_file_pwrite(int64_t fd, void *buf, int64_t buf_off, int64_t file_off, int64_t len) {
    if (fd <= 0 || !buf || len <= 0 || file_off < 0 || buf_off < 0) return -1;
    int64_t buf_size = *(int64_t*)((char*)buf - 8);
    if (buf_off + len > buf_size) return -1;
#if defined(_WIN32)
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
#else
    ssize_t res = pwrite((int)fd, (char*)buf + buf_off, (size_t)len, (off_t)file_off);
    return (int64_t)res;
#endif
}

int64_t lpp_file_fsync(int64_t fd) {
    if (fd <= 0) return -1;
#if defined(_WIN32)
    if (!FlushFileBuffers((HANDLE)(intptr_t)fd)) return -1;
    return 0;
#else
    return fsync((int)fd);
#endif
}

int64_t lpp_file_fdatasync(int64_t fd) {
    return lpp_file_fsync(fd);
}

int64_t lpp_file_ftruncate(int64_t fd, int64_t size) {
    if (fd <= 0 || size < 0) return -1;
#if defined(_WIN32)
    HANDLE h = (HANDLE)(intptr_t)fd;
    LARGE_INTEGER li;
    li.QuadPart = size;
    if (!SetFilePointerEx(h, li, 0, FILE_BEGIN)) return -1;
    if (!SetEndOfFile(h)) return -1;
    return 0;
#else
    return ftruncate((int)fd, (off_t)size);
#endif
}

int64_t lpp_fd_size(int64_t fd) {
    if (fd <= 0) return -1;
#if defined(_WIN32)
    HANDLE h = (HANDLE)(intptr_t)fd;
    LARGE_INTEGER li;
    if (!GetFileSizeEx(h, &li)) return -1;
    return (int64_t)li.QuadPart;
#else
    struct stat st;
    if (fstat((int)fd, &st) != 0) return -1;
    return (int64_t)st.st_size;
#endif
}




