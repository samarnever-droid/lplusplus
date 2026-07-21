/*
 * lpp_dir.c  —  L++ directory / filesystem builtins (cross-platform)
 *
 * Provides: dir_create, dir_list, dir_remove, path_exists, path_join
 *
 * Build: cc -O2 -c runtime/lpp_dir.c -o lpp_dir.o
 *        cl /nologo /O2 /c runtime/lpp_dir.c /Fo:lpp_dir.obj
 */

#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdio.h>

extern void *lpp_arc_alloc(int64_t size);
extern void  lpp_arc_release(void *ptr);
extern void *lpp_list_new_arc(void);
extern void  lpp_list_push_arc(void *list, void *value);
extern void  lpp_list_free(void *list);

#if defined(_WIN32)
/* ── Windows implementation ───────────────────────────────────────────── */
#include <windows.h>

int64_t lpp_dir_create(const char *path) {
    if (!path) return -1;
    return CreateDirectoryA(path, NULL) ? 0 : -1;
}

void *lpp_dir_list(const char *path) {
    void *list = lpp_list_new_arc();
    if (!list) return 0;
    if (!path) return list;

    char pattern[MAX_PATH];
    snprintf(pattern, sizeof(pattern), "%s\\*", path);
    WIN32_FIND_DATAA fd;
    HANDLE h = FindFirstFileA(pattern, &fd);
    if (h == INVALID_HANDLE_VALUE) return list;

    do {
        if (strcmp(fd.cFileName, ".") == 0 || strcmp(fd.cFileName, "..") == 0)
            continue;
        size_t len = strlen(fd.cFileName);
        char *copy = (char *)lpp_arc_alloc((int64_t)(len + 1));
        if (copy) { memcpy(copy, fd.cFileName, len); copy[len] = 0;
                    lpp_list_push_arc(list, copy); lpp_arc_release(copy); }
    } while (FindNextFileA(h, &fd));
    FindClose(h);
    return list;
}

int64_t lpp_dir_remove(const char *path) {
    if (!path) return -1;
    /* RemoveDirectoryA only works on empty directories.
       For a PM we need recursive removal, so shell out. */
    char cmd[MAX_PATH + 32];
    snprintf(cmd, sizeof(cmd), "rmdir /s /q \"%s\"", path);
    return system(cmd) == 0 ? 0 : -1;
}

int64_t lpp_path_exists(const char *path) {
    if (!path) return 0;
    DWORD attr = GetFileAttributesA(path);
    return (attr != INVALID_FILE_ATTRIBUTES) ? 1 : 0;
}

char *lpp_path_join(const char *base, const char *child) {
    if (!base) base = "";
    if (!child) child = "";
    size_t blen = strlen(base), clen = strlen(child);
    int need_sep = (blen > 0 && base[blen - 1] != '\\' && base[blen - 1] != '/');
    int64_t total = (int64_t)(blen + (need_sep ? 1 : 0) + clen + 1);
    char *out = (char *)lpp_arc_alloc(total);
    if (!out) return (char *)"";
    memcpy(out, base, blen);
    size_t off = blen;
    if (need_sep) out[off++] = '\\';
    memcpy(out + off, child, clen);
    out[off + clen] = 0;
    return out;
}

#else
/* ── Unix (Linux / macOS) implementation ───────────────────────────────── */
#include <sys/stat.h>
#include <sys/types.h>
#include <dirent.h>
#include <unistd.h>

int64_t lpp_dir_create(const char *path) {
    if (!path) return -1;
    return mkdir(path, 0755) == 0 ? 0 : -1;
}

void *lpp_dir_list(const char *path) {
    void *list = lpp_list_new_arc();
    if (!list) return 0;
    if (!path) return list;

    DIR *d = opendir(path);
    if (!d) return list;

    struct dirent *entry;
    while ((entry = readdir(d)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0)
            continue;
        size_t len = strlen(entry->d_name);
        char *copy = (char *)lpp_arc_alloc((int64_t)(len + 1));
        if (copy) { memcpy(copy, entry->d_name, len); copy[len] = 0;
                    lpp_list_push_arc(list, copy); lpp_arc_release(copy); }
    }
    closedir(d);
    return list;
}

int64_t lpp_dir_remove(const char *path) {
    if (!path) return -1;
    /* Recursive removal via system rm -rf for full directories */
    char cmd[4096];
    snprintf(cmd, sizeof(cmd), "rm -rf \"%s\"", path);
    return system(cmd) == 0 ? 0 : -1;
}

int64_t lpp_path_exists(const char *path) {
    if (!path) return 0;
    struct stat st;
    return stat(path, &st) == 0 ? 1 : 0;
}

char *lpp_path_join(const char *base, const char *child) {
    if (!base) base = "";
    if (!child) child = "";
    size_t blen = strlen(base), clen = strlen(child);
    int need_sep = (blen > 0 && base[blen - 1] != '/');
    int64_t total = (int64_t)(blen + (need_sep ? 1 : 0) + clen + 1);
    char *out = (char *)lpp_arc_alloc(total);
    if (!out) return (char *)"";
    memcpy(out, base, blen);
    size_t off = blen;
    if (need_sep) out[off++] = '/';
    memcpy(out + off, child, clen);
    out[off + clen] = 0;
    return out;
}

#endif
