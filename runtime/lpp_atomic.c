#ifndef LPP_ATOMIC_C
#define LPP_ATOMIC_C

#include <stdint.h>
#include <stdlib.h>

#if defined(_MSC_VER) || defined(_WIN32)
#include <windows.h>
#include <intrin.h>

extern void lpp_panic(const char *fmt, ...);

int64_t lpp_atomic_alloc(int64_t init_val) {
    int64_t *ptr = (int64_t *)malloc(sizeof(int64_t));
    if (!ptr) lpp_panic("out of memory in atomic_alloc");
    *ptr = init_val;
    return (int64_t)(uintptr_t)ptr;
}

int64_t lpp_atomic_new(int64_t init_val) {
    return lpp_atomic_alloc(init_val);
}

void lpp_atomic_free(int64_t ptr) {
    if (ptr) free((void *)(uintptr_t)ptr);
}

int64_t lpp_atomic_load(int64_t ptr) {
    return _InterlockedOr64((volatile LONG64 *)(uintptr_t)ptr, 0);
}

int64_t lpp_atomic_load_acq(int64_t ptr) {
    return _InterlockedOr64((volatile LONG64 *)(uintptr_t)ptr, 0);
}

int64_t lpp_atomic_load_relaxed(int64_t ptr) {
    return *(volatile int64_t *)(uintptr_t)ptr;
}

void lpp_atomic_store(int64_t ptr, int64_t val) {
    _InterlockedExchange64((volatile LONG64 *)(uintptr_t)ptr, (LONG64)val);
}

void lpp_atomic_store_rel(int64_t ptr, int64_t val) {
    _InterlockedExchange64((volatile LONG64 *)(uintptr_t)ptr, (LONG64)val);
}

void lpp_atomic_store_relaxed(int64_t ptr, int64_t val) {
    *(volatile int64_t *)(uintptr_t)ptr = val;
}

int64_t lpp_atomic_add(int64_t ptr, int64_t val) {
    return (int64_t)_InterlockedExchangeAdd64((volatile LONG64 *)(uintptr_t)ptr, (LONG64)val);
}

int64_t lpp_atomic_sub(int64_t ptr, int64_t val) {
    return (int64_t)_InterlockedExchangeAdd64((volatile LONG64 *)(uintptr_t)ptr, -(LONG64)val);
}

int64_t lpp_atomic_and(int64_t ptr, int64_t val) {
    return (int64_t)_InterlockedAnd64((volatile LONG64 *)(uintptr_t)ptr, (LONG64)val);
}

int64_t lpp_atomic_or(int64_t ptr, int64_t val) {
    return (int64_t)_InterlockedOr64((volatile LONG64 *)(uintptr_t)ptr, (LONG64)val);
}

int64_t lpp_atomic_xor(int64_t ptr, int64_t val) {
    return (int64_t)_InterlockedXor64((volatile LONG64 *)(uintptr_t)ptr, (LONG64)val);
}

int64_t lpp_atomic_swap(int64_t ptr, int64_t val) {
    return (int64_t)_InterlockedExchange64((volatile LONG64 *)(uintptr_t)ptr, (LONG64)val);
}

int64_t lpp_atomic_cas(int64_t ptr, int64_t expected, int64_t desired) {
    return (int64_t)_InterlockedCompareExchange64((volatile LONG64 *)(uintptr_t)ptr, (LONG64)desired, (LONG64)expected);
}

int64_t lpp_atomic_cas_weak(int64_t ptr, int64_t expected, int64_t desired) {
    return (int64_t)_InterlockedCompareExchange64((volatile LONG64 *)(uintptr_t)ptr, (LONG64)desired, (LONG64)expected);
}

int64_t lpp_atomic_load32(int64_t ptr) {
    return (int64_t)_InterlockedOr((volatile LONG *)(uintptr_t)ptr, 0);
}

void lpp_atomic_store32(int64_t ptr, int64_t val) {
    _InterlockedExchange((volatile LONG *)(uintptr_t)ptr, (LONG)val);
}

int64_t lpp_atomic_add32(int64_t ptr, int64_t val) {
    return (int64_t)_InterlockedExchangeAdd((volatile LONG *)(uintptr_t)ptr, (LONG)val);
}

int64_t lpp_atomic_cas32(int64_t ptr, int64_t expected, int64_t desired) {
    return (int64_t)_InterlockedCompareExchange((volatile LONG *)(uintptr_t)ptr, (LONG)desired, (LONG)expected);
}

void lpp_atomic_fence(void) {
    MemoryBarrier();
}

void lpp_atomic_fence_acq(void) {
    MemoryBarrier();
}

void lpp_atomic_fence_rel(void) {
    MemoryBarrier();
}

void lpp_cpu_pause(void) {
    YieldProcessor();
}

#else
#include <stdatomic.h>
#if (defined(__x86_64__) || defined(_M_X64) || defined(__i386__) || defined(_M_IX86)) && !defined(__aarch64__) && !defined(_M_ARM64) && !defined(__arm64__)
#include <immintrin.h>
#endif

extern void lpp_panic(const char *fmt, ...);

int64_t lpp_atomic_alloc(int64_t init_val) {
    int64_t *ptr = (int64_t *)malloc(sizeof(int64_t));
    if (!ptr) lpp_panic("out of memory in atomic_alloc");
    *ptr = init_val;
    return (int64_t)(uintptr_t)ptr;
}

int64_t lpp_atomic_new(int64_t init_val) {
    return lpp_atomic_alloc(init_val);
}

void lpp_atomic_free(int64_t ptr) {
    if (ptr) free((void *)(uintptr_t)ptr);
}

int64_t lpp_atomic_load(int64_t ptr) {
    return __atomic_load_n((const int64_t *)(uintptr_t)ptr, __ATOMIC_SEQ_CST);
}

int64_t lpp_atomic_load_acq(int64_t ptr) {
    return __atomic_load_n((const int64_t *)(uintptr_t)ptr, __ATOMIC_ACQUIRE);
}

int64_t lpp_atomic_load_relaxed(int64_t ptr) {
    return __atomic_load_n((const int64_t *)(uintptr_t)ptr, __ATOMIC_RELAXED);
}

void lpp_atomic_store(int64_t ptr, int64_t val) {
    __atomic_store_n((int64_t *)(uintptr_t)ptr, val, __ATOMIC_SEQ_CST);
}

void lpp_atomic_store_rel(int64_t ptr, int64_t val) {
    __atomic_store_n((int64_t *)(uintptr_t)ptr, val, __ATOMIC_RELEASE);
}

void lpp_atomic_store_relaxed(int64_t ptr, int64_t val) {
    __atomic_store_n((int64_t *)(uintptr_t)ptr, val, __ATOMIC_RELAXED);
}

int64_t lpp_atomic_add(int64_t ptr, int64_t val) {
    return __atomic_fetch_add((int64_t *)(uintptr_t)ptr, val, __ATOMIC_SEQ_CST);
}

int64_t lpp_atomic_sub(int64_t ptr, int64_t val) {
    return __atomic_fetch_sub((int64_t *)(uintptr_t)ptr, val, __ATOMIC_SEQ_CST);
}

int64_t lpp_atomic_and(int64_t ptr, int64_t val) {
    return __atomic_fetch_and((int64_t *)(uintptr_t)ptr, val, __ATOMIC_SEQ_CST);
}

int64_t lpp_atomic_or(int64_t ptr, int64_t val) {
    return __atomic_fetch_or((int64_t *)(uintptr_t)ptr, val, __ATOMIC_SEQ_CST);
}

int64_t lpp_atomic_xor(int64_t ptr, int64_t val) {
    return __atomic_fetch_xor((int64_t *)(uintptr_t)ptr, val, __ATOMIC_SEQ_CST);
}

int64_t lpp_atomic_swap(int64_t ptr, int64_t val) {
    return __atomic_exchange_n((int64_t *)(uintptr_t)ptr, val, __ATOMIC_SEQ_CST);
}

int64_t lpp_atomic_cas(int64_t ptr, int64_t expected, int64_t desired) {
    int64_t exp = expected;
    __atomic_compare_exchange_n((int64_t *)(uintptr_t)ptr, &exp, desired, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    return exp;
}

int64_t lpp_atomic_cas_weak(int64_t ptr, int64_t expected, int64_t desired) {
    int64_t exp = expected;
    __atomic_compare_exchange_n((int64_t *)(uintptr_t)ptr, &exp, desired, 1, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    return exp;
}

int64_t lpp_atomic_load32(int64_t ptr) {
    return (int64_t)__atomic_load_n((const int32_t *)(uintptr_t)ptr, __ATOMIC_SEQ_CST);
}

void lpp_atomic_store32(int64_t ptr, int64_t val) {
    __atomic_store_n((int32_t *)(uintptr_t)ptr, (int32_t)val, __ATOMIC_SEQ_CST);
}

int64_t lpp_atomic_add32(int64_t ptr, int64_t val) {
    return (int64_t)__atomic_fetch_add((int32_t *)(uintptr_t)ptr, (int32_t)val, __ATOMIC_SEQ_CST);
}

int64_t lpp_atomic_cas32(int64_t ptr, int64_t expected, int64_t desired) {
    int32_t exp = (int32_t)expected;
    __atomic_compare_exchange_n((int32_t *)(uintptr_t)ptr, &exp, (int32_t)desired, 0, __ATOMIC_SEQ_CST, __ATOMIC_SEQ_CST);
    return (int64_t)exp;
}

void lpp_atomic_fence(void) {
    __atomic_thread_fence(__ATOMIC_SEQ_CST);
}

void lpp_atomic_fence_acq(void) {
    __atomic_thread_fence(__ATOMIC_ACQUIRE);
}

void lpp_atomic_fence_rel(void) {
    __atomic_thread_fence(__ATOMIC_RELEASE);
}

void lpp_cpu_pause(void) {
#if (defined(__x86_64__) || defined(_M_X64) || defined(__i386__) || defined(_M_IX86)) && !defined(__aarch64__) && !defined(_M_ARM64) && !defined(__arm64__)
    __builtin_ia32_pause();
#elif defined(__aarch64__) || defined(__arm64__) || defined(_M_ARM64)
    __asm__ __volatile__("yield");
#endif
}

#endif

#endif /* LPP_ATOMIC_C */
