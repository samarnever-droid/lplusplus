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
extern char *lpp_empty_str(void);
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

static int lpp_win_rmdir_recursive(const char *path) {
    char pattern[MAX_PATH];
    snprintf(pattern, sizeof(pattern), "%s\\*", path);

    WIN32_FIND_DATAA fd;
    HANDLE h = FindFirstFileA(pattern, &fd);
    if (h != INVALID_HANDLE_VALUE) {
        do {
            if (strcmp(fd.cFileName, ".") == 0 || strcmp(fd.cFileName, "..") == 0)
                continue;

            char subpath[MAX_PATH];
            snprintf(subpath, sizeof(subpath), "%s\\%s", path, fd.cFileName);

            if (fd.dwFileAttributes & FILE_ATTRIBUTE_DIRECTORY) {
                lpp_win_rmdir_recursive(subpath);
            } else {
                DeleteFileA(subpath);
            }
        } while (FindNextFileA(h, &fd));
        FindClose(h);
    }
    return RemoveDirectoryA(path) ? 0 : -1;
}

int64_t lpp_dir_remove(const char *path) {
    if (!path) return -1;
    return lpp_win_rmdir_recursive(path);
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
    if (!out) return lpp_empty_str();
    memcpy(out, base, blen);
    size_t off = blen;
    if (need_sep) out[off++] = '\\';
    memcpy(out + off, child, clen);
    out[off + clen] = 0;
    return out;
}

#else
/* ── Unix (Linux / macOS) implementation ───────────────────────────────── */
#define _XOPEN_SOURCE 500
#include <sys/stat.h>
#include <sys/types.h>
#include <dirent.h>
#include <unistd.h>
#include <ftw.h>

static int lpp_nftw_remove(const char *fpath, const struct stat *sb, int typeflag, struct FTW *ftwbuf) {
    (void)sb;
    (void)typeflag;
    (void)ftwbuf;
    return remove(fpath);
}

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
    /* Recursive removal via nftw */
    return nftw(path, lpp_nftw_remove, 64, FTW_DEPTH | FTW_PHYS) == 0 ? 0 : -1;
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
    if (!out) return lpp_empty_str();
    memcpy(out, base, blen);
    size_t off = blen;
    if (need_sep) out[off++] = '/';
    memcpy(out + off, child, clen);
    out[off + clen] = 0;
    return out;
}

#endif
