#ifndef LPP_CLOCK_RNG_C
#define LPP_CLOCK_RNG_C

#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#if defined(_MSC_VER) || defined(_WIN32)
#include <windows.h>
#else
#include <time.h>
#endif

extern void lpp_panic(const char *fmt, ...);

typedef struct LppRng {
    uint64_t state;
} LppRng;

static uint64_t lpp_splitmix64(uint64_t *s) {
    uint64_t z = (*s += 0x9e3779b97f4a7c15ULL);
    z = (z ^ (z >> 30)) * 0xbf58476d1ce4e5b9ULL;
    z = (z ^ (z >> 27)) * 0x94d049bb133111ebULL;
    return z ^ (z >> 31);
}

int64_t lpp_rng_new(int64_t seed) {
    LppRng *rng = (LppRng *)malloc(sizeof(LppRng));
    if (!rng) lpp_panic("out of memory in rng_new");
    rng->state = seed == 0 ? 0x853c49e6748fea9bULL : (uint64_t)seed;
    return (int64_t)(uintptr_t)rng;
}

int64_t lpp_rng_next(int64_t handle) {
    LppRng *rng = (LppRng *)(uintptr_t)handle;
    if (!rng) return 0;
    return (int64_t)lpp_splitmix64(&rng->state);
}

int64_t lpp_rng_range(int64_t handle, int64_t min_v, int64_t max_v) {
    if (min_v >= max_v) return min_v;
    uint64_t diff = (uint64_t)(max_v - min_v + 1);
    uint64_t val = (uint64_t)lpp_rng_next(handle);
    return min_v + (int64_t)(val % diff);
}

double lpp_rng_float(int64_t handle) {
    uint64_t val = (uint64_t)lpp_rng_next(handle);
    return (double)(val >> 11) * (1.0 / 9007199254740992.0);
}

void lpp_rng_free(int64_t handle) {
    if (handle) free((void *)(uintptr_t)handle);
}

typedef struct LppClock {
    int is_virtual;
    int64_t current_time;
} LppClock;

int64_t lpp_clock_new(int64_t initial_time) {
    LppClock *clk = (LppClock *)malloc(sizeof(LppClock));
    if (!clk) lpp_panic("out of memory in clock_new");
    if (initial_time != 0) {
        clk->is_virtual = 1;
        clk->current_time = initial_time;
    } else {
        clk->is_virtual = 0;
        clk->current_time = 0;
    }
    return (int64_t)(uintptr_t)clk;
}

int64_t lpp_clock_now(int64_t handle) {
    LppClock *clk = (LppClock *)(uintptr_t)handle;
    if (!clk) return 0;
    if (clk->is_virtual) {
        return clk->current_time;
    }
#if defined(_MSC_VER) || defined(_WIN32)
    FILETIME ft;
    GetSystemTimeAsFileTime(&ft);
    ULARGE_INTEGER uli;
    uli.LowPart = ft.dwLowDateTime;
    uli.HighPart = ft.dwHighDateTime;
    return (int64_t)((uli.QuadPart - 116444736000000000ULL) / 10000ULL);
#else
    struct timespec ts;
    clock_gettime(CLOCK_REALTIME, &ts);
    return (int64_t)ts.tv_sec * 1000 + (int64_t)ts.tv_nsec / 1000000;
#endif
}

void lpp_clock_advance(int64_t handle, int64_t delta) {
    LppClock *clk = (LppClock *)(uintptr_t)handle;
    if (!clk) return;
    if (clk->is_virtual) {
        clk->current_time += delta;
    }
}

void lpp_clock_free(int64_t handle) {
    if (handle) free((void *)(uintptr_t)handle);
}

#endif /* LPP_CLOCK_RNG_C */
