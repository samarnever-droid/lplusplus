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
    if (slen == 0) { char *e = (char *)lpp_alloc(1); e[0] = 0; return e; }
    if (n > 0 && slen > 0x7FFFFFFFFFFFFFFFLL / n) {
        lpp_exit(101);
    }
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
    int64_t delta = nlen - olen;
    if (count > 0 && delta > 0 && slen > 0x7FFFFFFFFFFFFFFFLL - count * delta) {
        lpp_exit(101);
    }
    int64_t rlen = slen + count * delta;
    if (rlen < 0) rlen = 0;
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

/* float_to_str: the host runtime uses snprintf("%g"), which this freestanding
 * build has no libc for. Formatting is done by hand with the same six-decimal
 * fixed form lpp_print_float already uses, then trailing zeros are trimmed so
 * 2.5 prints as "2.5" rather than "2.500000".
 *
 * Without this symbol the internal lpp-link fails with
 * "unresolved GOT symbol 'lpp_float_to_str'" for any program that calls
 * float_to_str — the function existed only in runtime/lpp_str.c (host path). */
char *lpp_float_to_str(double val) {
    char buffer[64];
    int64_t w = 0;
    int negative = (val < 0.0);
    if (negative) val = -val;
    int64_t ipart = (int64_t)val;
    double fpart = val - (double)ipart;
    int64_t frac = (int64_t)(fpart * 1000000.0 + 0.5);
    if (frac >= 1000000) { frac -= 1000000; ipart += 1; }

    char tmp[32];
    int64_t t = 0;
    uint64_t magnitude = (uint64_t)ipart;
    do { tmp[t++] = (char)('0' + (magnitude % 10)); magnitude /= 10; } while (magnitude != 0);
    if (negative) buffer[w++] = '-';
    while (t > 0) buffer[w++] = tmp[--t];

    char fbuf[8];
    for (int i = 5; i >= 0; i--) { fbuf[i] = (char)('0' + (frac % 10)); frac /= 10; }
    int64_t flen = 6;
    while (flen > 0 && fbuf[flen - 1] == '0') flen--;
    if (flen > 0) {
        buffer[w++] = '.';
        for (int64_t i = 0; i < flen; i++) buffer[w++] = fbuf[i];
    }
    buffer[w] = 0;

    char *out = (char *)lpp_alloc(w + 1);
    for (int64_t i = 0; i <= w; i++) out[i] = buffer[i];
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
int64_t lpp_parse_int(const char *s) { return lpp_str_to_int(s); }

typedef struct LppEnvOverride {
    const char *name;
    const char *value;
    struct LppEnvOverride *next;
} LppEnvOverride;

static LppEnvOverride *lpp_env_overrides = 0;

int64_t lpp_env_set(const char *name, const char *value) {
    if (!name || !*name) return -1;
    LppEnvOverride *curr = lpp_env_overrides;
    while (curr) {
        if (lpp_str_eq(curr->name, name)) {
            if (curr->value) {
                lpp_free((void *)curr->value, 0);
            }
            if (value) {
                int len = 0;
                while (value[len]) len++;
                char *new_val = (char *)lpp_alloc(len + 1);
                for (int i = 0; i <= len; i++) new_val[i] = value[i];
                curr->value = new_val;
            } else {
                curr->value = 0;
            }
            return 0;
        }
        curr = curr->next;
    }
    LppEnvOverride *node = (LppEnvOverride *)lpp_alloc(sizeof(LppEnvOverride));
    int nlen = 0;
    while (name[nlen]) nlen++;
    char *new_name = (char *)lpp_alloc(nlen + 1);
    for (int i = 0; i <= nlen; i++) new_name[i] = name[i];
    node->name = new_name;
    if (value) {
        int vlen = 0;
        while (value[vlen]) vlen++;
        char *new_val = (char *)lpp_alloc(vlen + 1);
        for (int i = 0; i <= vlen; i++) new_val[i] = value[i];
        node->value = new_val;
    } else {
        node->value = 0;
    }
    node->next = lpp_env_overrides;
    lpp_env_overrides = node;
    return 0;
}

char *lpp_env_get(const char *name) {
    if (!name || !*name) {
        char *empty = (char *)lpp_alloc(1);
        empty[0] = 0;
        return empty;
    }
    LppEnvOverride *curr = lpp_env_overrides;
    while (curr) {
        if (lpp_str_eq(curr->name, name)) {
            if (!curr->value) {
                char *empty = (char *)lpp_alloc(1);
                empty[0] = 0;
                return empty;
            }
            int len = 0;
            while (curr->value[len]) len++;
            char *out = (char *)lpp_alloc(len + 1);
            for (int i = 0; i <= len; i++) out[i] = curr->value[i];
            return out;
        }
        curr = curr->next;
    }
    long fd = lpp_sys_open("/proc/self/environ", 0, 0);
    if (fd < 0) {
        char *empty = (char *)lpp_alloc(1);
        empty[0] = 0;
        return empty;
    }
    long cap = 32768;
    char *buf = (char *)lpp_arc_alloc(cap);
    if (!buf) {
        lpp_sys_close(fd);
        char *empty = (char *)lpp_alloc(1);
        empty[0] = 0;
        return empty;
    }
    long total = 0;
    while (total < cap) {
        long rd = lpp_sys_read(fd, buf + total, cap - total);
        if (rd <= 0) break;
        total += rd;
    }
    lpp_sys_close(fd);
    long name_len = 0;
    while (name[name_len]) name_len++;
    long i = 0;
    while (i < total) {
        int match = 1;
        for (long j = 0; j < name_len; j++) {
            if (i + j >= total || buf[i + j] != name[j]) {
                match = 0;
                break;
            }
        }
        if (match && i + name_len < total && buf[i + name_len] == '=') {
            long val_start = i + name_len + 1;
            long val_len = 0;
            while (val_start + val_len < total && buf[val_start + val_len] != '\0') {
                val_len++;
            }
            char *out = (char *)lpp_alloc(val_len + 1);
            for (long k = 0; k < val_len; k++) {
                out[k] = buf[val_start + k];
            }
            out[val_len] = 0;
            lpp_arc_release(buf);
            return out;
        }
        while (i < total && buf[i] != '\0') {
            i++;
        }
        i++;
    }
    lpp_arc_release(buf);
    char *empty = (char *)lpp_alloc(1);
    empty[0] = 0;
    return empty;
}

static long lpp_sys_fork(void) {
    long result;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(57)
        : "rcx", "r11", "memory"
    );
    return result;
}

static long lpp_sys_execve(const char *pathname, char *const argv[], char *const envp[]) {
    long result;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(59), "D"(pathname), "S"(argv), "d"(envp)
        : "rcx", "r11", "memory"
    );
    return result;
}

static long lpp_sys_wait4(long pid, int *wstatus, int options, void *rusage) {
    long result;
    register long r10 __asm__("r10") = (long)rusage;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(61), "D"(pid), "S"(wstatus), "d"((long)options), "r"(r10)
        : "rcx", "r11", "memory"
    );
    return result;
}

static long lpp_sys_pipe(int pipefd[2]) {
    long result;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(22), "D"(pipefd)
        : "rcx", "r11", "memory"
    );
    return result;
}

static long lpp_sys_dup2(int oldfd, int newfd) {
    long result;
    __asm__ volatile (
        "syscall"
        : "=a"(result)
        : "a"(33), "D"(oldfd), "S"(newfd)
        : "rcx", "r11", "memory"
    );
    return result;
}

int64_t lpp_command_exec(const char *cmdline) {
    if (!cmdline) return -1;
    long pid = lpp_sys_fork();
    if (pid < 0) return -1;
    if (pid == 0) {
        const char *argv[] = { "/bin/sh", "-c", cmdline, 0 };
        lpp_sys_execve("/bin/sh", (char *const *)argv, 0);
        lpp_exit(127);
    }
    int status = 0;
    long ret = lpp_sys_wait4(pid, &status, 0, 0);
    if (ret < 0) return -1;
    if ((status & 0x7f) == 0) {
        return (int64_t)((status >> 8) & 0xff);
    }
    return -1;
}

char *lpp_command_output(const char *cmdline) {
    if (!cmdline) {
        char *empty = (char *)lpp_alloc(1);
        empty[0] = 0;
        return empty;
    }
    int pipefd[2];
    if (lpp_sys_pipe(pipefd) < 0) {
        char *empty = (char *)lpp_alloc(1);
        empty[0] = 0;
        return empty;
    }
    long pid = lpp_sys_fork();
    if (pid < 0) {
        lpp_sys_close(pipefd[0]);
        lpp_sys_close(pipefd[1]);
        char *empty = (char *)lpp_alloc(1);
        empty[0] = 0;
        return empty;
    }
    if (pid == 0) {
        lpp_sys_dup2(pipefd[1], 1);
        lpp_sys_dup2(pipefd[1], 2);
        lpp_sys_close(pipefd[0]);
        lpp_sys_close(pipefd[1]);
        const char *argv[] = { "/bin/sh", "-c", cmdline, 0 };
        lpp_sys_execve("/bin/sh", (char *const *)argv, 0);
        lpp_exit(127);
    }
    lpp_sys_close(pipefd[1]);
    long cap = 1024;
    long length = 0;
    char *buf = (char *)lpp_alloc(cap);
    while (1) {
        if (length + 256 >= cap) {
            long new_cap = cap * 2;
            char *new_buf = (char *)lpp_alloc(new_cap);
            for (long i = 0; i < length; i++) new_buf[i] = buf[i];
            lpp_arc_release(buf);
            buf = new_buf;
            cap = new_cap;
        }
        long rd = lpp_sys_read(pipefd[0], buf + length, 256);
        if (rd <= 0) break;
        length += rd;
    }
    buf[length] = 0;
    lpp_sys_close(pipefd[0]);
    int status = 0;
    lpp_sys_wait4(pid, &status, 0, 0);
    return buf;
}

const char *lpp_input(void) {
    char buf[4096];
    long rd = lpp_sys_read(0, buf, sizeof(buf) - 1);
    if (rd < 0) rd = 0;
    while (rd > 0 && (buf[rd - 1] == '\n' || buf[rd - 1] == '\r')) {
        rd--;
    }
    char *out = (char *)lpp_alloc(rd + 1);
    for (long i = 0; i < rd; i++) {
        out[i] = buf[i];
    }
    out[rd] = 0;
    return out;
}

