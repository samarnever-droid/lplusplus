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

/* ── str_repeat(s, n) → ARC-allocated string repeated n times ──────────── */
/* O(1) allocation instead of O(n) str_concat calls.                        */
char *lpp_str_repeat(const char *s, int64_t n) {
    if (!s || n <= 0) return (char *)"";
    size_t slen = strlen(s);
    if (slen == 0) return (char *)"";
    size_t total = slen * (size_t)n;
    char *out = (char *)lpp_arc_alloc((int64_t)(total + 1));
    if (!out) return (char *)"";
    for (int64_t i = 0; i < n; i++) {
        memcpy(out + i * slen, s, slen);
    }
    out[total] = 0;
    return out;
}

/* ── char_at: return single character at index as a 1-char string ── */
char *lpp_char_at(const char *s, int64_t idx) {
    if (!s) return NULL;
    int64_t len = (int64_t)strlen(s);
    if (idx < 0 || idx >= len) return NULL;
    char *out = (char *)malloc(2);
    out[0] = s[idx];
    out[1] = 0;
    return out;
}

/* ── ord: return ASCII/Unicode codepoint of first character ── */
int64_t lpp_ord(const char *s) {
    if (!s || !s[0]) return 0;
    return (int64_t)(unsigned char)s[0];
}

/* ── chr: return 1-char string from codepoint ── */
char *lpp_chr(int64_t code) {
    char *out = (char *)malloc(2);
    out[0] = (char)(code & 0xFF);
    out[1] = 0;
    return out;
}

/* ── str_contains: check if needle is in haystack ── */
int64_t lpp_str_contains(const char *haystack, const char *needle) {
    if (!haystack || !needle) return 0;
    return strstr(haystack, needle) != NULL ? 1 : 0;
}

/* ── str_starts_with ── */
int64_t lpp_str_starts_with(const char *s, const char *prefix) {
    if (!s || !prefix) return 0;
    size_t plen = strlen(prefix);
    return strncmp(s, prefix, plen) == 0 ? 1 : 0;
}

/* ── str_ends_with ── */
int64_t lpp_str_ends_with(const char *s, const char *suffix) {
    if (!s || !suffix) return 0;
    size_t slen = strlen(s);
    size_t xlen = strlen(suffix);
    if (xlen > slen) return 0;
    return strcmp(s + slen - xlen, suffix) == 0 ? 1 : 0;
}

/* ── str_upper: uppercase copy ── */
char *lpp_str_upper(const char *s) {
    if (!s) return NULL;
    size_t len = strlen(s);
    char *out = (char *)malloc(len + 1);
    for (size_t i = 0; i < len; i++)
        out[i] = (s[i] >= 'a' && s[i] <= 'z') ? s[i] - 32 : s[i];
    out[len] = 0;
    return out;
}

/* ── str_lower: lowercase copy ── */
char *lpp_str_lower(const char *s) {
    if (!s) return NULL;
    size_t len = strlen(s);
    char *out = (char *)malloc(len + 1);
    for (size_t i = 0; i < len; i++)
        out[i] = (s[i] >= 'A' && s[i] <= 'Z') ? s[i] + 32 : s[i];
    out[len] = 0;
    return out;
}

/* ── int_to_str: convert integer to string ── */
char *lpp_int_to_str(int64_t val) {
    char buf[32];
    snprintf(buf, sizeof(buf), "%lld", (long long)val);
    char *out = (char *)malloc(strlen(buf) + 1);
    strcpy(out, buf);
    return out;
}

/* ── str_to_int: parse integer from string ── */
int64_t lpp_str_to_int(const char *s) {
    if (!s) return 0;
    return (int64_t)strtoll(s, NULL, 10);
}
