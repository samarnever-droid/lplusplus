/* ── I/O ──────────────────────────────────────────────────────────────────── */

void lpp_print_int(int64_t value) {
    printf("%lld\n", (long long)value);
    fflush(stdout);
}

void lpp_print_float(double value) {
    printf("%f\n", value);
    fflush(stdout);
}

void lpp_print_bool(int8_t value) {
    printf("%d\n", value ? 1 : 0);
    fflush(stdout);
}

void lpp_print_str(const char *ptr) {
    if (!ptr) return;
#ifdef LPP_ANDROID
    /* Android build (no console): route to logcat so output is visible. */
    __android_log_print(ANDROID_LOG_INFO, "L++", "%s", ptr);
#else
    /* Host / Termux (console + full libc): normal stdout with trailing newline. */
    puts(ptr);
    fflush(stdout);
#endif
}

void lpp_write_str(const char *ptr) {
    if (!ptr) return;
#ifdef LPP_ANDROID
    __android_log_print(ANDROID_LOG_INFO, "L++", "%s", ptr);
#else
    /* Writes raw string without appending newline. */
    fputs(ptr, stdout);
    fflush(stdout);
#endif
}

/* Read one line from stdin (strips trailing newline).
   Returns a heap-allocated string; caller frees with lpp_free_str. */
char *lpp_input(void) {
    char buf[4096];
    if (!fgets(buf, sizeof(buf), stdin)) return NULL;
    size_t len = strlen(buf);
    if (len > 0 && buf[len - 1] == '\n') buf[--len] = '\0';
    char *result = (char *)lpp_arc_alloc((int64_t)(len + 1));
    if (!result) return NULL;
    memcpy(result, buf, len + 1);
    return result;
}

/* `Str` values are ARC-managed: every owned string is allocated via
 * lpp_arc_alloc (see lpp_input, lpp_net_strdup_impl, lpp_json_get_str) and
 * reclaimed through lpp_arc_release. The old `free(ptr)` here passed the
 * payload address directly to libc, even though the 16-byte ARC header is
 * immediately before it, so real owned strings were not safely reclaimed.
 * lpp_arc_release validates
 * the header first, so it is a safe no-op on NULL and on non-ARC pointers and
 * correctly reclaims owned strings. */
void lpp_free_str(char *ptr) {
    lpp_arc_release(ptr);
}

int64_t lpp_parse_int(const char *str) {
    if (!str || *str == '\0') {
        lpp_panic("Invalid integer format: empty string");
    }

    // Skip leading whitespace
    const char *p = str;
    while (*p == ' ' || *p == '\t' || *p == '\r' || *p == '\n') {
        p++;
    }

    if (*p == '\0') {
        lpp_panic("Invalid integer format: \"%s\"", str);
    }

    char *endptr;
    errno = 0;
    long long val = strtoll(p, &endptr, 10);

    // Check for overflow/underflow
    if (errno == ERANGE) {
        lpp_panic("Integer overflow/underflow: \"%s\" exceeds 64-bit limits", str);
    }

    // Check for trailing garbage (invalid chars)
    while (*endptr == ' ' || *endptr == '\t' || *endptr == '\r' || *endptr == '\n') {
        endptr++;
    }
    if (*endptr != '\0') {
        lpp_panic("Invalid integer format: \"%s\"", str);
    }

    return (int64_t)val;
}

/* ── File I/O ─────────────────────────────────────────────────────────────── */

#include <sys/stat.h>

#if defined(_WIN32)
#  define LPP_STAT_STRUCT struct _stat64
#  define LPP_FSTAT _fstat64
#  define LPP_FILENO _fileno
#  define LPP_S_ISREG(m) (((m) & _S_IFREG) == _S_IFREG)
#else
#  define LPP_STAT_STRUCT struct stat
#  define LPP_FSTAT fstat
#  define LPP_FILENO fileno
#  define LPP_S_ISREG(m) S_ISREG(m)
#endif

/* Read entire file contents. Returns heap-allocated string or NULL on error. */
char *lpp_read_file(const char *path) {
    if (!path) return NULL;
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;

    LPP_STAT_STRUCT st;
    int64_t size = -1;
    int is_reg = 0;

    if (LPP_FSTAT(LPP_FILENO(f), &st) == 0) {
        if (LPP_S_ISREG(st.st_mode)) {
            size = (int64_t)st.st_size;
            is_reg = 1;
        }
    }

    if (is_reg && size >= 0) {
        char *buf = (char *)lpp_arc_alloc(size + 1);
        if (!buf) { fclose(f); return NULL; }
        size_t read = fread(buf, 1, (size_t)size, f);
        buf[read] = '\0';
        fclose(f);
        return buf;
    } else {
        size_t cap = 4096;
        size_t len = 0;
        char *buf = (char *)lpp_arc_alloc((int64_t)cap);
        if (!buf) { fclose(f); return NULL; }

        for (;;) {
            if (len + 1024 >= cap) {
                if (cap > (size_t)INT64_MAX / 2) {   /* cap *= 2 overflow guard */
                    lpp_arc_release(buf);
                    fclose(f);
                    return NULL;
                }
                size_t new_cap = cap * 2;
                char *new_buf = (char *)lpp_arc_alloc((int64_t)new_cap);
                if (!new_buf) {
                    lpp_arc_release(buf);
                    fclose(f);
                    return NULL;
                }
                memcpy(new_buf, buf, len);
                lpp_arc_release(buf);
                buf = new_buf;
                cap = new_cap;
            }
            size_t n = fread(buf + len, 1, 1024, f);
            if (n == 0) break;
            len += n;
        }
        buf[len] = '\0';
        fclose(f);
        return buf;
    }
}

static void make_parent_dirs(const char *path) {
    if (!path) return;
    char temp[1024];
    strncpy(temp, path, sizeof(temp) - 1);
    temp[sizeof(temp) - 1] = '\0';
    for (char *p = temp + 1; *p; p++) {
        if (*p == '/' || *p == '\\') {
            char orig = *p;
            *p = '\0';
#if defined(_WIN32)
            _mkdir(temp);
#else
            mkdir(temp, 0755);
#endif
            *p = orig;
        }
    }
}

/* Write data to file. Returns 0 on success, -1 on error. */
int64_t lpp_write_file(const char *path, const char *data) {
    if (!path || !data) return -1;
    make_parent_dirs(path);
    FILE *f = fopen(path, "wb");
    if (!f) return -1;
    size_t len = strlen(data);
    size_t written = fwrite(data, 1, len, f);
    int close_status = fclose(f);
    return written == len && close_status == 0 ? 0 : -1;
}

void lpp_exit(int64_t code) {
#if defined(_WIN32)
    ExitProcess((UINT)code);
#else
    exit((int)code);
#endif
}

/* Append data to file. Returns 0 on success, -1 on error. */
int64_t lpp_append_file(const char *path, const char *data) {
    if (!path || !data) return -1;
    FILE *f = fopen(path, "ab");
    if (!f) return -1;
    size_t len = strlen(data);
    size_t written = fwrite(data, 1, len, f);
    int close_status = fclose(f);
    return written == len && close_status == 0 ? 0 : -1;
}

/* ── Math Builtins ── */
double lpp_sqrt(double x) { return sqrt(x); }
double lpp_sin(double x) { return sin(x); }
double lpp_cos(double x) { return cos(x); }
double lpp_tan(double x) { return tan(x); }

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

    LPP_STAT_STRUCT st;
    int64_t size = -1;
    if (LPP_FSTAT(LPP_FILENO(f), &st) == 0) {
        if (LPP_S_ISREG(st.st_mode)) {
            size = (int64_t)st.st_size;
        }
    }
    fclose(f);
    return size;
}

/* ── System Metrics ──────────────────────────────────────────────────────── */
#if defined(_WIN32)
int64_t lpp_sys_mem_total(void) {
    MEMORYSTATUSEX status;
    memset(&status, 0, sizeof(status));
    status.dwLength = sizeof(status);
    if (GlobalMemoryStatusEx(&status)) {
        return (int64_t)(status.ullTotalPhys / (1024 * 1024));
    }
    return 16384;
}

int64_t lpp_sys_mem_free(void) {
    MEMORYSTATUSEX status;
    memset(&status, 0, sizeof(status));
    status.dwLength = sizeof(status);
    if (GlobalMemoryStatusEx(&status)) {
        return (int64_t)(status.ullAvailPhys / (1024 * 1024));
    }
    return 8192;
}

static uint64_t lpp__filetime_to_u64(const FILETIME *ft) {
    return ((uint64_t)ft->dwHighDateTime << 32) | ft->dwLowDateTime;
}

int64_t lpp_sys_cpu_usage(void) {
    static uint64_t last_idle = 0;
    static uint64_t last_kernel = 0;
    static uint64_t last_user = 0;

    FILETIME idle, kernel, user;
    if (!GetSystemTimes(&idle, &kernel, &user)) return 5;

    uint64_t now_idle = lpp__filetime_to_u64(&idle);
    uint64_t now_kernel = lpp__filetime_to_u64(&kernel);
    uint64_t now_user = lpp__filetime_to_u64(&user);

    if (last_idle == 0) {
        Sleep(15);
        if (!GetSystemTimes(&idle, &kernel, &user)) return 5;
        now_idle = lpp__filetime_to_u64(&idle);
        now_kernel = lpp__filetime_to_u64(&kernel);
        now_user = lpp__filetime_to_u64(&user);
    }

    uint64_t idle_diff = now_idle - last_idle;
    uint64_t kernel_diff = now_kernel - last_kernel;
    uint64_t user_diff = now_user - last_user;

    last_idle = now_idle;
    last_kernel = now_kernel;
    last_user = now_user;

    uint64_t total_diff = kernel_diff + user_diff;
    if (total_diff == 0) return 0;
    if (total_diff >= idle_diff) {
        return (int64_t)((total_diff - idle_diff) * 100 / total_diff);
    }
    return 0;
}

int64_t lpp_sys_uptime(void) {
    return (int64_t)(GetTickCount64() / 1000);
}
#elif defined(__APPLE__) || defined(__MACH__)
#include <sys/types.h>
#include <sys/sysctl.h>
int64_t lpp_sys_mem_total(void) {
    int64_t mem = 0;
    size_t len = sizeof(mem);
    if (sysctlbyname("hw.memsize", &mem, &len, NULL, 0) == 0) {
        return mem / (1024 * 1024);
    }
    return 16384;
}

int64_t lpp_sys_mem_free(void) {
    return 8192;
}

int64_t lpp_sys_cpu_usage(void) {
    return 5;
}

int64_t lpp_sys_uptime(void) {
    struct timeval boottime;
    size_t len = sizeof(boottime);
    int mib[2] = {CTL_KERN, KERN_BOOTTIME};
    if (sysctl(mib, 2, &boottime, &len, NULL, 0) == 0) {
        time_t now = time(NULL);
        return (int64_t)(now - boottime.tv_sec);
    }
    return 3600;
}
#else
#include <sys/sysinfo.h>
int64_t lpp_sys_mem_total(void) {
    struct sysinfo info;
    if (sysinfo(&info) == 0) return (int64_t)((info.totalram * info.mem_unit) / (1024 * 1024));
    return 16384;
}

int64_t lpp_sys_mem_free(void) {
    struct sysinfo info;
    if (sysinfo(&info) == 0) return (int64_t)((info.freeram * info.mem_unit) / (1024 * 1024));
    return 8192;
}

int64_t lpp_sys_cpu_usage(void) {
    struct sysinfo info;
    if (sysinfo(&info) == 0) return (int64_t)info.loads[0];
    return 5;
}

int64_t lpp_sys_uptime(void) {
    struct sysinfo info;
    if (sysinfo(&info) == 0) return (int64_t)info.uptime;
    return 3600;
}
#endif

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


