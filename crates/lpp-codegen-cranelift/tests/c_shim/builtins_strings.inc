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

static const uint32_t lpp__shim_empty_str_blob[8] __attribute__((aligned(16))) = {
    LPP_SHIM_IMMORTAL, LPP_SHIM_IMMORTAL, 0, 0, 0, 0, 0, 0
};

char *lpp_empty_str(void) {
    return (char *)&lpp__shim_empty_str_blob[4];
}

const char *lpp_test_external_string(void) {
    static const char value[] = "foreign-owned";
    return value;
}

int64_t lpp_test_external_string_length(const char *value) {
    return value ? (int64_t)strlen(value) : -1;
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

static const char *lpp_memmem(const void *hay, size_t hay_len, const void *needle, size_t needle_len) {
    if (needle_len == 0) return (const char *)hay;
    if (hay_len < needle_len) return NULL;
    const unsigned char *h = (const unsigned char *)hay;
    const unsigned char *n = (const unsigned char *)needle;
    size_t limit = hay_len - needle_len;
    for (size_t i = 0; i <= limit; i++) {
        if (h[i] == n[0] && memcmp(h + i, n, needle_len) == 0) {
            return (const char *)(h + i);
        }
    }
    return NULL;
}

int64_t lpp_str_find(const char *haystack, const char *needle) {
    if (!*needle) return 0;
    const char *hit = lpp_memmem(haystack, strlen(haystack), needle, strlen(needle));
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
            const char *hit = lpp_memmem(cursor, strlen(cursor), old, old_len);
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
        const char *hit = lpp_memmem(cursor, strlen(cursor), old, old_len);
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

