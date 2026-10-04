#ifndef LPP_INT_C
#define LPP_INT_C

#include <stdint.h>
#include <stddef.h>
#include <string.h>
#include <stdio.h>
#include <stdlib.h>

#if defined(_MSC_VER)
#include <intrin.h>
#include <stdlib.h>
#endif

/* Unsigned bit shifts and arithmetic */
int64_t lpp_shr_u(int64_t a, int64_t b) {
    if (b < 0 || b >= 64) return 0;
    return (int64_t)((uint64_t)a >> b);
}

int64_t lpp_shl_u(int64_t a, int64_t b) {
    if (b < 0 || b >= 64) return 0;
    return (int64_t)((uint64_t)a << b);
}

int64_t lpp_div_u(int64_t a, int64_t b) {
    if (b == 0) {
        lpp_panic("unsigned integer division by zero");
    }
    return (int64_t)((uint64_t)a / (uint64_t)b);
}

int64_t lpp_rem_u(int64_t a, int64_t b) {
    if (b == 0) {
        lpp_panic("unsigned integer modulo by zero");
    }
    return (int64_t)((uint64_t)a % (uint64_t)b);
}

int64_t lpp_lt_u(int64_t a, int64_t b) {
    return ((uint64_t)a < (uint64_t)b) ? 1 : 0;
}

int64_t lpp_le_u(int64_t a, int64_t b) {
    return ((uint64_t)a <= (uint64_t)b) ? 1 : 0;
}

int64_t lpp_gt_u(int64_t a, int64_t b) {
    return ((uint64_t)a > (uint64_t)b) ? 1 : 0;
}

int64_t lpp_ge_u(int64_t a, int64_t b) {
    return ((uint64_t)a >= (uint64_t)b) ? 1 : 0;
}

int64_t lpp_min_u(int64_t a, int64_t b) {
    return ((uint64_t)a < (uint64_t)b) ? a : b;
}

int64_t lpp_max_u(int64_t a, int64_t b) {
    return ((uint64_t)a > (uint64_t)b) ? a : b;
}

char *lpp_u64_to_str(int64_t a) {
    char buf[32];
    snprintf(buf, sizeof(buf), "%llu", (unsigned long long)(uint64_t)a);
    size_t len = strlen(buf);
    char *out = (char*)lpp_arc_alloc((int64_t)(len + 1));
    if (!out) return lpp_empty_str();
    memcpy(out, buf, len + 1);
    return out;
}

char *lpp_u64_to_hex(int64_t a) {
    char buf[32];
    snprintf(buf, sizeof(buf), "%llx", (unsigned long long)(uint64_t)a);
    size_t len = strlen(buf);
    char *out = (char*)lpp_arc_alloc((int64_t)(len + 1));
    if (!out) return lpp_empty_str();
    memcpy(out, buf, len + 1);
    return out;
}

int64_t lpp_str_to_u64(const char *s) {
    if (!s) return 0;
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r') s++;
    uint64_t val = 0;
    if (s[0] == '0' && (s[1] == 'x' || s[1] == 'X')) {
        s += 2;
        while (*s) {
            char c = *s++;
            if (c >= '0' && c <= '9') val = (val << 4) | (uint64_t)(c - '0');
            else if (c >= 'a' && c <= 'f') val = (val << 4) | (uint64_t)(c - 'a' + 10);
            else if (c >= 'A' && c <= 'F') val = (val << 4) | (uint64_t)(c - 'A' + 10);
            else break;
        }
    } else {
        while (*s >= '0' && *s <= '9') {
            val = val * 10 + (uint64_t)(*s++ - '0');
        }
    }
    return (int64_t)val;
}

int64_t lpp_rotl64(int64_t a, int64_t b) {
    uint64_t v = (uint64_t)a;
    unsigned int shift = (unsigned int)(b & 63);
    if (shift == 0) return a;
    return (int64_t)((v << shift) | (v >> (64 - shift)));
}

int64_t lpp_rotr64(int64_t a, int64_t b) {
    uint64_t v = (uint64_t)a;
    unsigned int shift = (unsigned int)(b & 63);
    if (shift == 0) return a;
    return (int64_t)((v >> shift) | (v << (64 - shift)));
}

int64_t lpp_rotl32(int64_t a, int64_t b) {
    uint32_t v = (uint32_t)a;
    unsigned int shift = (unsigned int)(b & 31);
    if (shift == 0) return (int64_t)v;
    return (int64_t)(uint32_t)((v << shift) | (v >> (32 - shift)));
}

int64_t lpp_rotr32(int64_t a, int64_t b) {
    uint32_t v = (uint32_t)a;
    unsigned int shift = (unsigned int)(b & 31);
    if (shift == 0) return (int64_t)v;
    return (int64_t)(uint32_t)((v >> shift) | (v << (32 - shift)));
}

int64_t lpp_clz64(int64_t a) {
    uint64_t v = (uint64_t)a;
    if (v == 0) return 64;
#if defined(_MSC_VER)
    unsigned long index;
    if (_BitScanReverse64(&index, v)) {
        return (int64_t)(63 - index);
    }
    return 64;
#elif defined(__GNUC__) || defined(__clang__)
    return (int64_t)__builtin_clzll(v);
#else
    int count = 0;
    while ((v & 0x8000000000000000ULL) == 0) {
        count++;
        v <<= 1;
    }
    return count;
#endif
}

int64_t lpp_ctz64(int64_t a) {
    uint64_t v = (uint64_t)a;
    if (v == 0) return 64;
#if defined(_MSC_VER)
    unsigned long index;
    if (_BitScanForward64(&index, v)) {
        return (int64_t)index;
    }
    return 64;
#elif defined(__GNUC__) || defined(__clang__)
    return (int64_t)__builtin_ctzll(v);
#else
    int count = 0;
    while ((v & 1) == 0) {
        count++;
        v >>= 1;
    }
    return count;
#endif
}

int64_t lpp_popcount64(int64_t a) {
    uint64_t v = (uint64_t)a;
#if defined(_MSC_VER)
    return (int64_t)__popcnt64(v);
#elif defined(__GNUC__) || defined(__clang__)
    return (int64_t)__builtin_popcountll(v);
#else
    v = v - ((v >> 1) & 0x5555555555555555ULL);
    v = (v & 0x3333333333333333ULL) + ((v >> 2) & 0x3333333333333333ULL);
    v = (v + (v >> 4)) & 0x0F0F0F0F0F0F0F0FULL;
    return (int64_t)((v * 0x0101010101010101ULL) >> 56);
#endif
}

int64_t lpp_bswap16(int64_t a) {
    uint16_t v = (uint16_t)a;
    return (int64_t)(uint16_t)((v >> 8) | (v << 8));
}

int64_t lpp_bswap32(int64_t a) {
    uint32_t v = (uint32_t)a;
#if defined(_MSC_VER)
    return (int64_t)_byteswap_ulong(v);
#elif defined(__GNUC__) || defined(__clang__)
    return (int64_t)__builtin_bswap32(v);
#else
    return (int64_t)(((v >> 24) & 0xFF) | ((v >> 8) & 0xFF00) | ((v << 8) & 0xFF0000) | ((v << 24) & 0xFF000000));
#endif
}

int64_t lpp_bswap64(int64_t a) {
    uint64_t v = (uint64_t)a;
#if defined(_MSC_VER)
    return (int64_t)_byteswap_uint64(v);
#elif defined(__GNUC__) || defined(__clang__)
    return (int64_t)__builtin_bswap64(v);
#else
    return (int64_t)(
        ((v & 0x00000000000000FFULL) << 56) |
        ((v & 0x000000000000FF00ULL) << 40) |
        ((v & 0x0000000000FF0000ULL) << 24) |
        ((v & 0x00000000FF000000ULL) << 8)  |
        ((v & 0x000000FF00000000ULL) >> 8)  |
        ((v & 0x0000FF0000000000ULL) >> 24) |
        ((v & 0x00FF000000000000ULL) >> 40) |
        ((v & 0xFF00000000000000ULL) >> 56)
    );
#endif
}

int64_t lpp_trunc_u8(int64_t a) {
    return (int64_t)((uint8_t)a);
}

int64_t lpp_trunc_u16(int64_t a) {
    return (int64_t)((uint16_t)a);
}

int64_t lpp_trunc_u32(int64_t a) {
    return (int64_t)((uint32_t)a);
}

int64_t lpp_trunc_i8(int64_t a) {
    return (int64_t)((int8_t)a);
}

int64_t lpp_trunc_i16(int64_t a) {
    return (int64_t)((int16_t)a);
}

int64_t lpp_trunc_i32(int64_t a) {
    return (int64_t)((int32_t)a);
}

int64_t lpp_add_checked(int64_t a, int64_t b) {
    int64_t res;
#if defined(__GNUC__) || defined(__clang__)
    if (__builtin_add_overflow(a, b, &res)) {
        lpp_panic("integer addition overflow: %lld + %lld", (long long)a, (long long)b);
    }
#else
    if ((b > 0 && a > (INT64_MAX - b)) || (b < 0 && a < (INT64_MIN - b))) {
        lpp_panic("integer addition overflow: %lld + %lld", (long long)a, (long long)b);
    }
    res = a + b;
#endif
    return res;
}

int64_t lpp_sub_checked(int64_t a, int64_t b) {
    int64_t res;
#if defined(__GNUC__) || defined(__clang__)
    if (__builtin_sub_overflow(a, b, &res)) {
        lpp_panic("integer subtraction overflow: %lld - %lld", (long long)a, (long long)b);
    }
#else
    if ((b < 0 && a > (INT64_MAX + b)) || (b > 0 && a < (INT64_MIN + b))) {
        lpp_panic("integer subtraction overflow: %lld - %lld", (long long)a, (long long)b);
    }
    res = a - b;
#endif
    return res;
}

int64_t lpp_mul_checked(int64_t a, int64_t b) {
    int64_t res;
#if defined(__GNUC__) || defined(__clang__)
    if (__builtin_mul_overflow(a, b, &res)) {
        lpp_panic("integer multiplication overflow: %lld * %lld", (long long)a, (long long)b);
    }
#else
    if (a > 0) {
        if (b > 0) {
            if (a > (INT64_MAX / b)) lpp_panic("integer multiplication overflow: %lld * %lld", (long long)a, (long long)b);
        } else {
            if (b < (INT64_MIN / a)) lpp_panic("integer multiplication overflow: %lld * %lld", (long long)a, (long long)b);
        }
    } else {
        if (b > 0) {
            if (a < (INT64_MIN / b)) lpp_panic("integer multiplication overflow: %lld * %lld", (long long)a, (long long)b);
        } else {
            if (a != 0 && b < (INT64_MAX / a)) lpp_panic("integer multiplication overflow: %lld * %lld", (long long)a, (long long)b);
        }
    }
    res = a * b;
#endif
    return res;
}

int64_t lpp_add_wrap(int64_t a, int64_t b) {
    return (int64_t)((uint64_t)a + (uint64_t)b);
}

int64_t lpp_sub_wrap(int64_t a, int64_t b) {
    return (int64_t)((uint64_t)a - (uint64_t)b);
}

int64_t lpp_mul_wrap(int64_t a, int64_t b) {
    return (int64_t)((uint64_t)a * (uint64_t)b);
}

#endif /* LPP_INT_C */
