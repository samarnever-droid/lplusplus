/*
 * lpp_str.c  —  L++ string builtins (cross-platform, libc-backed)
 *
 * Provides: str_concat, str_split, str_find, str_replace, str_substr, str_trim
 *
 * Build: cc -O2 -c runtime/lpp_str.c -o lpp_str.o
 *        cl /nologo /O2 /c runtime/lpp_str.c /Fo:lpp_str.obj
 */

#include <stdlib.h>
#include <string.h>
#include <stdint.h>

/* ── ARC helpers (declared in lpp_runtime.c) ──────────────────────────── */
extern void *lpp_arc_alloc(int64_t size);
extern void  lpp_arc_release(void *ptr);
extern void *lpp_list_new_arc(void);
extern void  lpp_list_push_arc(void *list, void *value);
extern int64_t lpp_list_len(void *list);
extern void *lpp_list_get_arc(void *list, int64_t index);
extern void  lpp_list_free(void *list);

/* ── str_concat(a, b) → ARC-allocated concatenation ───────────────────── */
char *lpp_str_concat(const char *a, const char *b) {
    if (!a) a = "";
    if (!b) b = "";
    size_t la = strlen(a), lb = strlen(b);
    char *out = (char *)lpp_arc_alloc((int64_t)(la + lb + 1));
    if (!out) return (char *)"";
    memcpy(out, a, la);
    memcpy(out + la, b, lb);
    out[la + lb] = 0;
    return out;
}

/* ── str_split(s, delim_char) → List[String] (ARC-managed) ────────────── */
void *lpp_str_split(const char *s, int64_t delim) {
    void *list = lpp_list_new_arc();
    if (!list) return 0;
    if (!s || !*s) return list;

    char ch = (char)delim;
    const char *start = s;

    for (;;) {
        if (*s == ch || *s == 0) {
            int64_t len = (int64_t)(s - start);
            char *piece = (char *)lpp_arc_alloc(len + 1);
            if (piece) {
                memcpy(piece, start, (size_t)len);
                piece[len] = 0;
                lpp_list_push_arc(list, piece);
                lpp_arc_release(piece); /* list now owns the reference */
            }
            if (*s == 0) break;
            start = s + 1;
        }
        s++;
    }
    return list;
}

/* ── str_find(haystack, needle) → first index or -1 ───────────────────── */
int64_t lpp_str_find(const char *haystack, const char *needle) {
    if (!haystack || !needle) return -1;
    const char *found = strstr(haystack, needle);
    if (!found) return -1;
    return (int64_t)(found - haystack);
}

/* ── str_replace(s, old, new) → ARC-allocated with replacements ───────── */
char *lpp_str_replace(const char *s, const char *old, const char *new_) {
    if (!s) s = "";
    if (!old || !*old) return (char *)s; /* no pattern → no change */
    if (!new_) new_ = "";

    size_t slen = strlen(s), olen = strlen(old), nlen = strlen(new_);
    /* Count occurrences */
    int64_t count = 0;
    const char *scan = s;
    while ((scan = strstr(scan, old))) { count++; scan += olen; }

    size_t outlen = slen + (size_t)count * (nlen - olen) + 1;
    char *out = (char *)lpp_arc_alloc((int64_t)outlen);
    if (!out) return (char *)"";

    char *dst = out;
    const char *src = s;
    while (*src) {
        const char *next = strstr(src, old);
        if (!next) { strcpy(dst, src); break; }
        size_t prefix = (size_t)(next - src);
        memcpy(dst, src, prefix); dst += prefix;
        memcpy(dst, new_, nlen);   dst += nlen;
        src = next + olen;
    }
    return out;
}

/* ── str_substr(s, start, length) → ARC-allocated slice ───────────────── */
char *lpp_str_substr(const char *s, int64_t start, int64_t length) {
    if (!s) s = "";
    size_t slen = strlen(s);
    if (start < 0) start = 0;
    if (start > (int64_t)slen) return (char *)"";

    size_t remain = slen - (size_t)start;
    size_t copy = (length < 0 || (size_t)length > remain) ? remain : (size_t)length;

    char *out = (char *)lpp_arc_alloc((int64_t)(copy + 1));
    if (!out) return (char *)"";
    memcpy(out, s + start, copy);
    out[copy] = 0;
    return out;
}

/* ── str_trim(s) → ARC-allocated with leading/trailing whitespace removed */
char *lpp_str_trim(const char *s) {
    if (!s) return (char *)"";
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r') s++;
    const char *end = s + strlen(s);
    while (end > s && (end[-1] == ' ' || end[-1] == '\t' || end[-1] == '\n' || end[-1] == '\r'))
        end--;

    int64_t len = (int64_t)(end - s);
    char *out = (char *)lpp_arc_alloc(len + 1);
    if (!out) return (char *)"";
    memcpy(out, s, (size_t)len);
    out[len] = 0;
    return out;
}
