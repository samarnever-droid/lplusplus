/* Phase 5B/5C differential link shim: implements the runtime symbols the
 * 5B/5C slice can import, matching the v1 runtime semantics
 * (runtime/linux_x86_64_min.c).
 *
 * The ARC header is the v1 24-byte layout:
 *   u32 magic @0, i32 refcount @4, i32 generation @8, u32 map_size @12,
 *   destructor @16.  `magic`/`refcount` both holding the 0x41524331
 *   sentinel mark an immortal object (.rodata string literal): retain and
 *   release read it and return without writing.
 *
 * Release at refcount 0: generation++, magic cleared, the destructor runs
 * on the payload, then the allocation is freed.
 *
 * Lists are private-layout ARC objects (header + LppList payload);
 * elements are stored one-per-8-byte-slot and interpreted per the push
 * kind. Out-of-bounds element access fails the link test loudly.
 */
#include <math.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#define LPP_SHIM_IMMORTAL 0x41524331u

typedef void (*LppShimDestructor)(void *payload);
typedef struct {
    uint32_t magic;
    int32_t refcount;
    int32_t generation;
    uint32_t map_size;
    LppShimDestructor destructor;
} LppShimHeader;

static inline int lpp_shim_is_immortal(const LppShimHeader *header) {
    return header->magic == LPP_SHIM_IMMORTAL &&
           (uint32_t)header->refcount == LPP_SHIM_IMMORTAL;
}

/* Monotonic source of object generations, matching the v1 runtime
 * (lpp__generation_counter): every live object carries a unique non-zero
 * generation, and a reused address never reuses one, so a stale weak handle
 * can never match a new occupant. Generation 0 is reserved as the
 * "untracked" sentinel used by the slice family. */
static int lpp_shim_generation_counter = 1;

void *lpp_arc_alloc_with_destructor(int64_t payload_size, LppShimDestructor destructor) {
    if (payload_size < 0) return (void *)0;
    size_t need = (size_t)payload_size + sizeof(LppShimHeader);
    char *memory = (char *)calloc(1, need);
    if (!memory) return (void *)0;
    LppShimHeader *header = (LppShimHeader *)memory;
    header->magic = LPP_SHIM_IMMORTAL;
    header->refcount = 1;
    header->generation = lpp_shim_generation_counter++;
    header->map_size = 0;
    header->destructor = destructor;
    return memory + sizeof(LppShimHeader);
}

void lpp_arc_retain(void *payload) {
    if (!payload) return;
    LppShimHeader *header = (LppShimHeader *)payload - 1;
    if (lpp_shim_is_immortal(header)) return;
    header->refcount++;
}

void lpp_arc_release(void *payload) {
    if (!payload) return;
    LppShimHeader *header = (LppShimHeader *)payload - 1;
    if (lpp_shim_is_immortal(header)) return;
    if (--header->refcount == 0) {
        header->generation++;
        header->magic = 0;
        if (header->destructor) header->destructor(payload);
        free((LppShimHeader *)payload - 1);
    }
}

void lpp_print_str(const char *text) {
    fputs(text, stdout);
    fputc('\n', stdout);
}

/* ── List objects ────────────────────────────────────────────────────────── */

enum {
    LPP_SHIM_ELEM_UNSET = 0,
    LPP_SHIM_ELEM_SCALAR,
    LPP_SHIM_ELEM_FLOAT,
    LPP_SHIM_ELEM_BOOL,
    LPP_SHIM_ELEM_ARC,
};

typedef struct {
    int64_t len;
    int64_t cap;
    int64_t *slots;
    int element_kind;
} LppShimList;

static void lpp_shim_list_destroy(void *raw) {
    LppShimList *list = (LppShimList *)raw;
    if (list->element_kind == LPP_SHIM_ELEM_ARC) {
        for (int64_t i = 0; i < list->len; i++) {
            lpp_arc_release((void *)list->slots[i]);
        }
    }
    free(list->slots);
}

static LppShimList *lpp_shim_list_new(int element_kind) {
    void *payload = lpp_arc_alloc_with_destructor(sizeof(LppShimList), lpp_shim_list_destroy);
    LppShimList *list = (LppShimList *)payload;
    list->len = 0;
    list->cap = 0;
    list->slots = (int64_t *)0;
    list->element_kind = element_kind;
    return list;
}

void *lpp_list_new(void) { return lpp_shim_list_new(LPP_SHIM_ELEM_UNSET); }

void *lpp_list_new_arc(void) { return lpp_shim_list_new(LPP_SHIM_ELEM_UNSET); }

static void lpp_shim_list_reserve(LppShimList *list, int element_kind, int64_t need) {
    if (need <= list->cap) return;
    if (list->element_kind == LPP_SHIM_ELEM_UNSET) list->element_kind = element_kind;
    int64_t cap = list->cap ? list->cap * 2 : 4;
    while (cap < need) cap *= 2;
    int64_t *slots = (int64_t *)realloc(list->slots, (size_t)cap * sizeof(int64_t));
    if (!slots) { fprintf(stderr, "lpp shim: list growth failed\n"); exit(101); }
    list->slots = slots;
    list->cap = cap;
}

static LppShimList *lpp_shim_check(void *raw, const char *op) {
    LppShimList *list = (LppShimList *)raw;
    if (!list) { fprintf(stderr, "lpp shim: %s on null list\n", op); exit(101); }
    return list;
}

static void lpp_shim_push(LppShimList *list, int element_kind, int64_t value) {
    lpp_shim_list_reserve(list, element_kind, list->len + 1);
    list->slots[list->len++] = value;
}

void lpp_list_push(void *raw, int64_t value) {
    lpp_shim_push(lpp_shim_check(raw, "push"), LPP_SHIM_ELEM_SCALAR, value);
}

void lpp_list_push_arc(void *raw, void *value) {
    LppShimList *list = lpp_shim_check(raw, "push_arc");
    lpp_arc_retain(value);
    lpp_shim_push(list, LPP_SHIM_ELEM_ARC, (int64_t)(intptr_t)value);
}

void lpp_list_push_float(void *raw, double value) {
    uint64_t bits;
    memcpy(&bits, &value, sizeof(bits));
    lpp_shim_push(lpp_shim_check(raw, "push_float"), LPP_SHIM_ELEM_FLOAT, (int64_t)bits);
}

void lpp_list_push_bool(void *raw, int8_t value) {
    lpp_shim_push(lpp_shim_check(raw, "push_bool"), LPP_SHIM_ELEM_BOOL, value);
}

static int64_t lpp_shim_index(LppShimList *list, int64_t index, const char *op) {
    if (index < 0 || index >= list->len) {
        fprintf(stderr, "lpp shim: %s out of bounds (%lld >= %lld)\n",
                op, (long long)index, (long long)list->len);
        exit(101);
    }
    return list->slots[index];
}

int64_t lpp_list_get(void *raw, int64_t index) {
    return lpp_shim_index(lpp_shim_check(raw, "get"), index, "get");
}

void *lpp_list_get_arc(void *raw, int64_t index) {
    LppShimList *list = lpp_shim_check(raw, "get_arc");
    return (void *)(intptr_t)lpp_shim_index(list, index, "get_arc");
}

double lpp_list_get_float(void *raw, int64_t index) {
    uint64_t bits = (uint64_t)lpp_shim_index(lpp_shim_check(raw, "get_float"), index, "get_float");
    double value;
    memcpy(&value, &bits, sizeof(value));
    return value;
}

int8_t lpp_list_get_bool(void *raw, int64_t index) {
    return (int8_t)lpp_shim_index(lpp_shim_check(raw, "get_bool"), index, "get_bool");
}

void lpp_list_set(void *raw, int64_t index, int64_t value) {
    LppShimList *list = lpp_shim_check(raw, "set");
    if (index < 0 || index >= list->len) {
        fprintf(stderr, "lpp shim: set out of bounds\n");
        exit(101);
    }
    list->slots[index] = value;
}

void lpp_list_set_arc(void *raw, int64_t index, void *value) {
    LppShimList *list = lpp_shim_check(raw, "set_arc");
    if (index < 0 || index >= list->len) {
        fprintf(stderr, "lpp shim: set_arc out of bounds\n");
        exit(101);
    }
    /* Retain the new owner first, then release the replaced element —
     * the interpreter's store_place order (a self-store survives). */
    lpp_arc_retain(value);
    int64_t old = list->slots[index];
    list->slots[index] = (int64_t)(intptr_t)value;
    lpp_arc_release((void *)(intptr_t)old);
}

void lpp_list_set_float(void *raw, int64_t index, double value) {
    uint64_t bits;
    memcpy(&bits, &value, sizeof(bits));
    lpp_list_set(raw, index, (int64_t)bits);
}

void lpp_list_set_bool(void *raw, int64_t index, int8_t value) {
    lpp_list_set(raw, index, value);
}

int64_t lpp_list_len(void *raw) {
    return lpp_shim_check(raw, "len")->len;
}

/* ── 5C2: closures, tuples, tasks ─────────────────────────────────────────── */

/* The closure capsule payload is [code @0, env @8]; the capsule's
 * destructor releases the env reference (NULL for zero-capture
 * closures is a no-op). */
void lpp_closure_destroy(void *payload) {
    int64_t env;
    memcpy(&env, (char *)payload + 8, sizeof(env));
    lpp_arc_release((void *)(intptr_t)env);
}

/* The task env tuple: 16-byte prefix (managed mask + packed 16-bit
 * absolute offsets), value slots from offset 16. No slot-count limit
 * beyond the 64 mask bits. */
static void lpp_shim_tuple_destroy(void *raw) {
    int64_t mask, offsets;
    memcpy(&mask, raw, sizeof(mask));
    memcpy(&offsets, (char *)raw + 8, sizeof(offsets));
    for (int i = 0; i < 64; i++) {
        if (!((uint64_t)mask >> i & 1)) continue;
        int64_t offset = (offsets >> (16 * i)) & 0xFFFF;
        if (!offset) continue;
        int64_t slot;
        memcpy(&slot, (char *)raw + offset, sizeof(slot));
        lpp_arc_release((void *)(intptr_t)slot);
    }
}

void *lpp_tuple_alloc(int64_t size, int64_t mask, int64_t packed_offsets) {
    if (size < 16) size = 16;
    if (size > 16 + 64 * 8) { fprintf(stderr, "lpp shim: tuple too large\n"); exit(101); }
    void *payload = lpp_arc_alloc_with_destructor(size, lpp_shim_tuple_destroy);
    if (!payload) return (void *)0;
    memcpy(payload, &mask, sizeof(mask));
    memcpy((char *)payload + 8, &packed_offsets, sizeof(packed_offsets));
    return payload;
}

/* The task node: { code, environment, result, state, result_managed }.
 * The destructor releases the environment and, once resolved and
 * managed, the result. */
typedef struct {
    int64_t code;
    int64_t environment;
    int64_t result;
    int32_t state;
    int32_t result_managed;
} LppShimTask;

static void lpp_shim_task_destroy(void *raw) {
    LppShimTask *task = (LppShimTask *)raw;
    lpp_arc_release((void *)(intptr_t)task->environment);
    if (task->state == 2 && task->result_managed) {
        lpp_arc_release((void *)(intptr_t)task->result);
    }
}

void *lpp_task_new(int64_t code, int64_t environment, int64_t managed) {
    if (!code || !environment) {
        fprintf(stderr, "lpp shim: task with null code or environment\n");
        exit(101);
    }
    void *payload = lpp_arc_alloc_with_destructor(sizeof(LppShimTask), lpp_shim_task_destroy);
    if (!payload) return (void *)0;
    LppShimTask *task = (LppShimTask *)payload;
    task->code = code;
    task->environment = environment;
    task->result = 0;
    task->state = 0;
    task->result_managed = (int32_t)managed;
    return payload;
}

int64_t lpp_task_poll(int64_t task_raw) {
    LppShimTask *task = (LppShimTask *)(intptr_t)task_raw;
    if (!task || task->state != 0) {
        fprintf(stderr, "lpp shim: poll of a non-fresh task\n");
        exit(101);
    }
    task->state = 1;
    int64_t result = ((int64_t (*)(void *))task->code)((void *)(intptr_t)task->environment);
    task->result = result;
    task->state = 2;
    return result;
}

int64_t lpp_task_await(int64_t task_raw) {
    LppShimTask *task = (LppShimTask *)(intptr_t)task_raw;
    if (!task || !task->environment) {
        fprintf(stderr, "lpp shim: await of a null task\n");
        exit(101);
    }
    if (task->state == 0) lpp_task_poll(task_raw);
    /* The task keeps its share; the awaiter gains one (the oracle's
     * repeated-await semantics). */
    if (task->result_managed) lpp_arc_retain((void *)(intptr_t)task->result);
    return task->result;
}

void lpp_task_destroy(int64_t task_raw) {
    lpp_arc_release((void *)(intptr_t)task_raw);
}

/* ── 5C2: the v1 builtin surface (Family A) ──────────────────────────────── */

/* Dynamic strings are ARC nodes (header + NUL-terminated payload). */
static char *lpp_shim_string(const char *text) {
    size_t length = strlen(text);
    char *payload = (char *)lpp_arc_alloc_with_destructor(length + 1, (LppShimDestructor)0);
    if (!payload) { fprintf(stderr, "lpp shim: string alloc failed\n"); exit(101); }
    memcpy(payload, text, length + 1);
    return payload;
}

static char *lpp_shim_string_capacity(size_t length) {
    char *payload = (char *)lpp_arc_alloc_with_destructor(length + 1, (LppShimDestructor)0);
    if (!payload) { fprintf(stderr, "lpp shim: string alloc failed\n"); exit(101); }
    payload[length] = 0;
    return payload;
}

void lpp_print_int(int64_t value) {
    char buffer[32];
    snprintf(buffer, sizeof(buffer), "%lld\n", (long long)value);
    fputs(buffer, stdout);
}

/* The oracle prints floats with Rust `{:.6}`: `NaN`, `inf`, `-inf`,
 * a sign for negative zero, and no exponent form. */
void lpp_print_float(double value) {
    char buffer[64];
    if (isnan(value)) {
        fputs("NaN\n", stdout);
        return;
    }
    if (isinf(value)) {
        fputs(value < 0.0 ? "-inf\n" : "inf\n", stdout);
        return;
    }
    snprintf(buffer, sizeof(buffer), "%.6f\n", value);
    fputs(buffer, stdout);
}

void lpp_print_bool(int8_t value) {
    lpp_print_int(value ? 1 : 0);
}

void lpp_eprint_str(const char *text) {
    if (!text) return;
    fputs(text, stderr);
    fputc('\n', stderr);
}

void lpp_write_str(const char *text) {
    if (!text) return;
    fputs(text, stdout);
}

int64_t lpp_str_len(const char *s) {
    return (int64_t)strlen(s);
}

int64_t lpp_str_eq(const char *a, const char *b) {
    return strcmp(a, b) == 0;
}

char *lpp_str_concat(const char *a, const char *b) {
    size_t alen = strlen(a);
    size_t blen = strlen(b);
    char *out = lpp_shim_string_capacity(alen + blen);
    memcpy(out, a, alen);
    memcpy(out + alen, b, blen);
    return out;
}

int64_t lpp_str_contains(const char *haystack, const char *needle) {
    return strstr(haystack, needle) != (char *)0;
}

int64_t lpp_str_starts_with(const char *text, const char *prefix) {
    size_t plen = strlen(prefix);
    return strncmp(text, prefix, plen) == 0;
}

int64_t lpp_str_ends_with(const char *text, const char *suffix) {
    size_t tlen = strlen(text);
    size_t slen = strlen(suffix);
    if (slen > tlen) return 0;
    return strcmp(text + (tlen - slen), suffix) == 0;
}

int64_t lpp_str_find(const char *haystack, const char *needle) {
    if (!*needle) return 0;
    const char *hit = memmem(haystack, strlen(haystack), needle, strlen(needle));
    return hit ? (int64_t)(hit - haystack) : -1;
}

/* Rust `str::replace`: all non-overlapping occurrences, left to right;
 * an empty `old` leaves the content unchanged (but still allocates). */
char *lpp_str_replace(const char *text, const char *old, const char *replacement) {
    size_t old_len = strlen(old);
    if (old_len == 0) return lpp_shim_string(text);
    size_t rep_len = strlen(replacement);
    size_t count = 0;
    {
        const char *cursor = text;
        for (;;) {
            const char *hit = memmem(cursor, strlen(cursor), old, old_len);
            if (!hit) break;
            count++;
            cursor = hit + old_len;
        }
    }
    size_t out_len = strlen(text) + count * rep_len;
    char *out = lpp_shim_string_capacity(out_len);
    size_t pos = 0;
    const char *cursor = text;
    for (;;) {
        const char *hit = memmem(cursor, strlen(cursor), old, old_len);
        if (!hit) {
            size_t tail = strlen(cursor);
            memcpy(out + pos, cursor, tail);
            pos += tail;
            break;
        }
        size_t before = (size_t)(hit - cursor);
        memcpy(out + pos, cursor, before);
        pos += before;
        memcpy(out + pos, replacement, rep_len);
        pos += rep_len;
        cursor = hit + old_len;
    }
    out[pos] = 0;
    return out;
}

/* v1 trims spaces, tabs, newlines, and carriage returns only. */
char *lpp_str_trim(const char *text) {
    const char *begin = text;
    const char *end = text + strlen(text);
    while (begin < end && (*begin == ' ' || *begin == '\t' || *begin == '\n' || *begin == '\r')) begin++;
    while (end > begin && (*(end - 1) == ' ' || *(end - 1) == '\t' || *(end - 1) == '\n' || *(end - 1) == '\r')) end--;
    char *out = lpp_shim_string_capacity((size_t)(end - begin));
    memcpy(out, begin, (size_t)(end - begin));
    return out;
}

char *lpp_str_lower(const char *text) {
    char *out = lpp_shim_string(text);
    for (char *c = out; *c; c++) {
        if (*c >= 'A' && *c <= 'Z') *c = (char)(*c + 32);
    }
    return out;
}

char *lpp_str_upper(const char *text) {
    char *out = lpp_shim_string(text);
    for (char *c = out; *c; c++) {
        if (*c >= 'a' && *c <= 'z') *c = (char)(*c - 32);
    }
    return out;
}

char *lpp_int_to_str(int64_t value) {
    char buffer[32];
    snprintf(buffer, sizeof(buffer), "%lld", (long long)value);
    return lpp_shim_string(buffer);
}

char *lpp_bool_to_str(int8_t value) {
    return lpp_shim_string(value ? "true" : "false");
}

char *lpp_u64_to_str(int64_t value) {
    char buffer[32];
    snprintf(buffer, sizeof(buffer), "%llu", (unsigned long long)(uint64_t)value);
    return lpp_shim_string(buffer);
}

char *lpp_u64_to_hex(int64_t value) {
    char buffer[32];
    snprintf(buffer, sizeof(buffer), "%llx", (unsigned long long)(uint64_t)value);
    return lpp_shim_string(buffer);
}

/* `strtoll` semantics: whitespace (incl. \v \f), optional sign, decimal
 * digits, saturating, 0 when no conversion is possible. */
int64_t lpp_str_to_int(const char *text) {
    while (*text == ' ' || *text == '\t' || *text == '\n' || *text == '\v' || *text == '\f' || *text == '\r') text++;
    int sign = 1;
    if (*text == '+') text++;
    else if (*text == '-') { sign = -1; text++; }
    int64_t value = 0;
    int converted = 0;
    while (*text >= '0' && *text <= '9') {
        int64_t digit = *text++ - '0';
        if (value > (INT64_MAX - digit) / 10) {
            value = sign > 0 ? INT64_MAX : INT64_MIN;
            while (*text >= '0' && *text <= '9') text++;
            return value;
        }
        value = value * 10 + digit;
        converted = 1;
    }
    if (!converted) return 0;
    return sign > 0 ? value : -value;
}

/* `lpp_str_to_u64`: whitespace (no \v \f), optional `0x` prefix, hex or
 * decimal collected with wrapping shifts. */
int64_t lpp_str_to_u64(const char *text) {
    while (*text == ' ' || *text == '\t' || *text == '\n' || *text == '\r') text++;
    int hexadecimal = 0;
    if (*text == '0' && (text[1] == 'x' || text[1] == 'X')) {
        text += 2;
        hexadecimal = 1;
    }
    uint64_t value = 0;
    while (*text) {
        if (hexadecimal) {
            int digit;
            char c = *text;
            if (c >= '0' && c <= '9') digit = c - '0';
            else if (c >= 'a' && c <= 'f') digit = c - 'a' + 10;
            else if (c >= 'A' && c <= 'F') digit = c - 'A' + 10;
            else break;
            value = (value << 4) | (uint64_t)digit;
            text++;
        } else {
            if (*text < '0' || *text > '9') break;
            value = value * 10 + (uint64_t)(*text - '0');
            text++;
        }
    }
    return (int64_t)value;
}

/* The oracle `format_percent_g`: C `%g` with the Rust 3-digit exponent
 * (`1e-005`, not `1e-05`). */
char *lpp_float_to_str(double value) {
    char buffer[64];
    if (isnan(value)) return lpp_shim_string("nan");
    if (isinf(value)) return lpp_shim_string(value < 0.0 ? "-inf" : "inf");
    int negative = signbit(value);
    double magnitude = negative ? -value : value;
    if (magnitude == 0.0) return lpp_shim_string(negative ? "-0" : "0");
    char scientific[32];
    snprintf(scientific, sizeof(scientific), "%.5e", magnitude);
    /* mantissa like `d.dddd`, exponent like `e+NN` / `e-NN`. */
    char *at_e = strchr(scientific, 'e');
    char *end_ptr = (char *)0;
    int exponent = (int)strtol(at_e + 1, &end_ptr, 10);
    char digits[8];
    int dlen = 0;
    for (char *c = scientific; c < at_e; c++) {
        if (*c == '.') continue;
        digits[dlen++] = *c;
    }
    digits[dlen] = 0;
    while (dlen > 0 && digits[dlen - 1] == '0') digits[--dlen] = 0;
    if (dlen == 0) { digits[0] = '0'; dlen = 1; }
    if (exponent < -4 || exponent >= 6) {
        /* The Rust 3-digit exponent form: `d[.ddd]e{:+03}`. */
        if (dlen == 1) {
            snprintf(buffer, sizeof(buffer), "%s%ce%+03d", negative ? "-" : "", digits[0], exponent);
        } else {
            size_t pos = 0;
            buffer[pos++] = negative ? '-' : digits[0];
            buffer[pos++] = '.';
            memcpy(buffer + pos, digits + 1, (size_t)(dlen - 1));
            pos += (size_t)(dlen - 1);
            buffer[pos++] = 'e';
            char expbuf[8];
            snprintf(expbuf, sizeof(expbuf), "%+03d", exponent);
            memcpy(buffer + pos, expbuf, sizeof(expbuf));
        }
        return lpp_shim_string(buffer);
    }
    if (exponent >= dlen - 1) {
        int zeros = exponent - (dlen - 1);
        size_t pos = 0;
        if (negative) buffer[pos++] = '-';
        memcpy(buffer + pos, digits, (size_t)dlen);
        pos += (size_t)dlen;
        memset(buffer + pos, '0', (size_t)zeros);
        pos += (size_t)zeros;
        buffer[pos] = 0;
        return lpp_shim_string(buffer);
    }
    if (exponent >= 0) {
        size_t pos = 0;
        if (negative) buffer[pos++] = '-';
        int split_at = exponent + 1;
        memcpy(buffer + pos, digits, (size_t)split_at);
        pos += (size_t)split_at;
        buffer[pos++] = '.';
        memcpy(buffer + pos, digits + split_at, (size_t)(dlen - split_at));
        pos += (size_t)(dlen - split_at);
        buffer[pos] = 0;
        return lpp_shim_string(buffer);
    }
    {
        int zeros = -exponent - 1;
        size_t pos = 0;
        if (negative) buffer[pos++] = '-';
        buffer[pos++] = '0';
        buffer[pos++] = '.';
        memset(buffer + pos, '0', (size_t)zeros);
        pos += (size_t)zeros;
        memcpy(buffer + pos, digits, (size_t)dlen);
        pos += (size_t)dlen;
        buffer[pos] = 0;
        return lpp_shim_string(buffer);
    }
}

int64_t lpp_abs(int64_t x) {
    return x < 0 ? (int64_t)(0 - (uint64_t)x) : x;
}

int64_t lpp_min(int64_t a, int64_t b) {
    return a < b ? a : b;
}

int64_t lpp_max(int64_t a, int64_t b) {
    return a > b ? a : b;
}

int64_t lpp_min_u(int64_t a, int64_t b) {
    return (uint64_t)a < (uint64_t)b ? a : b;
}

int64_t lpp_max_u(int64_t a, int64_t b) {
    return (uint64_t)a > (uint64_t)b ? a : b;
}

int64_t lpp_lt_u(int64_t a, int64_t b) {
    return (uint64_t)a < (uint64_t)b;
}

int64_t lpp_le_u(int64_t a, int64_t b) {
    return (uint64_t)a <= (uint64_t)b;
}

int64_t lpp_gt_u(int64_t a, int64_t b) {
    return (uint64_t)a > (uint64_t)b;
}

int64_t lpp_ge_u(int64_t a, int64_t b) {
    return (uint64_t)a >= (uint64_t)b;
}

int64_t lpp_shr_u(int64_t left, int64_t shift) {
    if (shift < 0 || shift >= 64) return 0;
    return (int64_t)((uint64_t)left >> shift);
}

int64_t lpp_shl_u(int64_t left, int64_t shift) {
    if (shift < 0 || shift >= 64) return 0;
    return (int64_t)((uint64_t)left << shift);
}

int64_t lpp_div_u(int64_t left, int64_t right) {
    if (right == 0) { fprintf(stderr, "lpp shim: unsigned division by zero\n"); exit(101); }
    return (int64_t)((uint64_t)left / (uint64_t)right);
}

int64_t lpp_rem_u(int64_t left, int64_t right) {
    if (right == 0) { fprintf(stderr, "lpp shim: unsigned remainder by zero\n"); exit(101); }
    return (int64_t)((uint64_t)left % (uint64_t)right);
}

int64_t lpp_popcount64(int64_t value) {
    return (int64_t)__builtin_popcountll((uint64_t)value);
}

int64_t lpp_clz64(int64_t value) {
    return value ? (int64_t)__builtin_clzll((uint64_t)value) : 64;
}

int64_t lpp_ctz64(int64_t value) {
    return value ? (int64_t)__builtin_ctzll((uint64_t)value) : 64;
}

int64_t lpp_bswap16(int64_t value) {
    return (int64_t)__builtin_bswap16((uint16_t)(uint64_t)value);
}

int64_t lpp_bswap32(int64_t value) {
    return (int64_t)__builtin_bswap32((uint32_t)(uint64_t)value);
}

int64_t lpp_bswap64(int64_t value) {
    return (int64_t)__builtin_bswap64((uint64_t)value);
}

int64_t lpp_rotl64(int64_t value, int64_t shift) {
    uint64_t x = (uint64_t)value;
    int n = (int)(shift & 63);
    if (n == 0) return value;
    return (int64_t)((x << n) | (x >> (64 - n)));
}

int64_t lpp_rotr64(int64_t value, int64_t shift) {
    uint64_t x = (uint64_t)value;
    int n = (int)(shift & 63);
    if (n == 0) return value;
    return (int64_t)((x >> n) | (x << (64 - n)));
}

int64_t lpp_rotl32(int64_t value, int64_t shift) {
    uint32_t x = (uint32_t)(uint64_t)value;
    int n = (int)(shift & 31);
    if (n == 0) return (int64_t)x;
    return (int64_t)((x << n) | (x >> (32 - n)));
}

int64_t lpp_rotr32(int64_t value, int64_t shift) {
    uint32_t x = (uint32_t)(uint64_t)value;
    int n = (int)(shift & 31);
    if (n == 0) return (int64_t)x;
    return (int64_t)((x >> n) | (x << (32 - n)));
}

int64_t lpp_trunc_u8(int64_t value) {
    return (int64_t)(uint8_t)(uint64_t)value;
}

int64_t lpp_trunc_u16(int64_t value) {
    return (int64_t)(uint16_t)(uint64_t)value;
}

int64_t lpp_trunc_u32(int64_t value) {
    return (int64_t)(uint32_t)(uint64_t)value;
}

int64_t lpp_trunc_i8(int64_t value) {
    return (int64_t)(int8_t)(uint64_t)value;
}

int64_t lpp_trunc_i16(int64_t value) {
    return (int64_t)(int16_t)(uint64_t)value;
}

int64_t lpp_trunc_i32(int64_t value) {
    return (int64_t)(int32_t)(uint64_t)value;
}

static int64_t lpp_shim_checked(int64_t left, int64_t right, int op) {
    // Signed overflow: the v1 checked family traps on *signed*
    // overflow. (The same-width unsigned builtins report the borrow /
    // carry instead, which would trap on any left < right.)
    int64_t result;
    int overflow;
    if (op == 0) overflow = __builtin_add_overflow(left, right, &result);
    else if (op == 1) overflow = __builtin_sub_overflow(left, right, &result);
    else overflow = __builtin_mul_overflow(left, right, &result);
    if (overflow) {
        fprintf(stderr, "lpp shim: integer overflow in checked operation\n");
        exit(101);
    }
    return result;
}

int64_t lpp_add_checked(int64_t left, int64_t right) {
    return lpp_shim_checked(left, right, 0);
}

int64_t lpp_sub_checked(int64_t left, int64_t right) {
    return lpp_shim_checked(left, right, 1);
}

int64_t lpp_mul_checked(int64_t left, int64_t right) {
    return lpp_shim_checked(left, right, 2);
}

int64_t lpp_add_wrap(int64_t left, int64_t right) {
    return (int64_t)((uint64_t)left + (uint64_t)right);
}

int64_t lpp_sub_wrap(int64_t left, int64_t right) {
    return (int64_t)((uint64_t)left - (uint64_t)right);
}

int64_t lpp_mul_wrap(int64_t left, int64_t right) {
    return (int64_t)((uint64_t)left * (uint64_t)right);
}

double lpp_floor(double value) {
    return floor(value);
}

double lpp_ceil(double value) {
    return ceil(value);
}

double lpp_pow(double base, double exponent) {
    return pow(base, exponent);
}

double lpp_sqrt(double value) {
    return sqrt(value);
}

/* The oracle `fmod`: IEEE remainder, same sign as the dividend. */
double fmod(double x, double y) {
    if (y == 0.0) return 0.0;
    return x - floor(x / y) * y;
}

/* ── 5C2: slice handles (Family C) ───────────────────────────────────────── */

typedef struct {
    void *base;
    int64_t start;
    int64_t length;
    int64_t generation;
    int64_t kind;
} LppShimSlice;

static int64_t lpp_shim_weak_generation(void *payload) {
    if (!payload) return 0;
    LppShimHeader *header = (LppShimHeader *)payload - 1;
    if (lpp_shim_is_immortal(header)) return (int64_t)LPP_SHIM_IMMORTAL;
    return header->generation;
}

static void *lpp_shim_weak_get(void *payload, int64_t expected_generation) {
    if (!payload || expected_generation == 0) return (void *)0;
    LppShimHeader *header = (LppShimHeader *)payload - 1;
    if (lpp_shim_is_immortal(header)) {
        return expected_generation == (int64_t)LPP_SHIM_IMMORTAL ? payload : (void *)0;
    }
    if (header->magic != LPP_SHIM_IMMORTAL || header->generation != (int32_t)expected_generation) {
        return (void *)0;
    }
    return payload;
}

static void *lpp_shim_slice_checked_base(LppShimSlice *view) {
    if (!view || !view->base || !view->generation) {
        fprintf(stderr, "lpp shim: slice of a dead object\n");
        exit(101);
    }
    void *base = lpp_shim_weak_get(view->base, view->generation);
    if (!base) {
        fprintf(stderr, "lpp shim: slice of a released object\n");
        exit(101);
    }
    return base;
}

void *lpp_slice_init(void *storage, void *base, int64_t start, int64_t length, int64_t kind) {
    if (!storage || !base || start < 0 || length < 0 ||
        start > INT64_MAX - length) {
        fprintf(stderr, "lpp shim: invalid slice range\n");
        exit(101);
    }
    int64_t source_length;
    if (kind == 0) source_length = (int64_t)strlen((const char *)base);
    else source_length = lpp_list_len(base);
    if (start > source_length || length > source_length - start) {
        fprintf(stderr, "lpp shim: slice out of range\n");
        exit(101);
    }
    LppShimSlice *view = (LppShimSlice *)storage;
    view->base = base;
    view->start = start;
    view->length = length;
    view->generation = lpp_shim_weak_generation(base);
    view->kind = kind;
    if (!view->generation) {
        fprintf(stderr, "lpp shim: slice of an untracked object\n");
        exit(101);
    }
    return view;
}

int64_t lpp_slice_len(void *raw) {
    LppShimSlice *view = (LppShimSlice *)raw;
    (void)lpp_shim_slice_checked_base(view);
    return view->length;
}

int64_t lpp_slice_get(void *raw, int64_t index) {
    LppShimSlice *view = (LppShimSlice *)raw;
    void *base = lpp_shim_slice_checked_base(view);
    if (view->kind != 1 || index < 0 || index >= view->length) {
        fprintf(stderr, "lpp shim: slice index out of range\n");
        exit(101);
    }
    return lpp_list_get(base, view->start + index);
}

double lpp_slice_get_float(void *raw, int64_t index) {
    int64_t bits = lpp_slice_get(raw, index);
    double value;
    memcpy(&value, &bits, sizeof(value));
    return value;
}

int8_t lpp_slice_get_bool(void *raw, int64_t index) {
    return lpp_slice_get(raw, index) != 0;
}

char *lpp_str_slice_get(void *raw, int64_t index) {
    LppShimSlice *view = (LppShimSlice *)raw;
    const char *base = (const char *)lpp_shim_slice_checked_base(view);
    if (view->kind != 0 || index < 0 || index >= view->length) {
        fprintf(stderr, "lpp shim: str slice index out of range\n");
        exit(101);
    }
    char *result = lpp_shim_string_capacity(1);
    result[0] = base[view->start + index];
    return result;
}

char *lpp_str_slice_to_str(void *raw) {
    LppShimSlice *view = (LppShimSlice *)raw;
    const char *base = (const char *)lpp_shim_slice_checked_base(view);
    if (view->kind != 0) {
        fprintf(stderr, "lpp shim: str_slice_to_str on a list slice\n");
        exit(101);
    }
    char *result = lpp_shim_string_capacity((size_t)view->length);
    memcpy(result, base + view->start, (size_t)view->length);
    return result;
}

/* ── 5C2: the scalar SIMD checksum (the 17 vector operators are native) ──── */

int64_t lpp_vec_i64_checksum(int64_t n) {
    if (n < 0) return 0;
    int64_t total = 0;
    for (int64_t i = 0; i < n; i++) total += (i * 3) ^ (i >> 1);
    return total;
}
