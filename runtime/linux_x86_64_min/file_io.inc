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

static long lpp_sys_rename(const char *old_path, const char *new_path) {
    long ret;
    __asm__ volatile (
        "syscall"
        : "=a"(ret)
        : "a"(82), "D"(old_path), "S"(new_path)
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

/* File mutation follows the host runtime ABI: 0 means success, -1 means
 * failure.  The old freestanding implementation returned 1 on success,
 * which made the same L++ program behave differently under --linker direct. */
int64_t lpp_write_file(const char *filename, const char *content) {
    if (!filename || !content) return -1;
    long fd = lpp_sys_open(filename, 0101, 0644); /* O_WRONLY | O_CREAT | O_TRUNC */
    if (fd < 0) return -1;
    long clen = 0;
    while (content[clen]) clen++;
    long written = lpp_sys_write(fd, content, clen);
    lpp_sys_close(fd);
    return written == clen ? 0 : -1;
}

int64_t lpp_append_file(const char *filename, const char *content) {
    if (!filename || !content) return -1;
    long fd = lpp_sys_open(filename, 02001, 0644); /* O_WRONLY | O_CREAT | O_APPEND */
    if (fd < 0) return -1;
    long clen = 0;
    while (content[clen]) clen++;
    long written = lpp_sys_write(fd, content, clen);
    lpp_sys_close(fd);
    return written == clen ? 0 : -1;
}

int64_t lpp_delete_file(const char *filename) {
    if (!filename) return -1;
    return lpp_sys_unlink(filename) == 0 ? 0 : -1;
}

int64_t lpp_file_move(const char *source, const char *destination) {
    if (!source || !destination) return -1;
    return lpp_sys_rename(source, destination) == 0 ? 0 : -1;
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

