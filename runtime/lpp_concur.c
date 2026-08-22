#ifndef LPP_CONCUR_C
#define LPP_CONCUR_C

#include <stdint.h>
#include <stdlib.h>

#if defined(_MSC_VER) || defined(_WIN32)
#include <windows.h>

extern void lpp_panic(const char *fmt, ...);

int64_t lpp_mutex_new(void) {
    CRITICAL_SECTION *cs = (CRITICAL_SECTION *)malloc(sizeof(CRITICAL_SECTION));
    if (!cs) lpp_panic("out of memory in mutex_new");
    InitializeCriticalSection(cs);
    return (int64_t)(uintptr_t)cs;
}

void lpp_mutex_lock(int64_t handle) {
    if (handle) EnterCriticalSection((CRITICAL_SECTION *)(uintptr_t)handle);
}

int64_t lpp_mutex_trylock(int64_t handle) {
    if (!handle) return 0;
    return TryEnterCriticalSection((CRITICAL_SECTION *)(uintptr_t)handle) ? 1 : 0;
}

void lpp_mutex_unlock(int64_t handle) {
    if (handle) LeaveCriticalSection((CRITICAL_SECTION *)(uintptr_t)handle);
}

void lpp_mutex_free(int64_t handle) {
    if (handle) {
        DeleteCriticalSection((CRITICAL_SECTION *)(uintptr_t)handle);
        free((void *)(uintptr_t)handle);
    }
}

int64_t lpp_rwlock_new(void) {
    SRWLOCK *rw = (SRWLOCK *)malloc(sizeof(SRWLOCK));
    if (!rw) lpp_panic("out of memory in rwlock_new");
    InitializeSRWLock(rw);
    return (int64_t)(uintptr_t)rw;
}

void lpp_rwlock_rdlock(int64_t handle) {
    if (handle) AcquireSRWLockShared((SRWLOCK *)(uintptr_t)handle);
}

void lpp_rwlock_wrlock(int64_t handle) {
    if (handle) AcquireSRWLockExclusive((SRWLOCK *)(uintptr_t)handle);
}

void lpp_rwlock_rdunlock(int64_t handle) {
    if (handle) ReleaseSRWLockShared((SRWLOCK *)(uintptr_t)handle);
}

void lpp_rwlock_wrunlock(int64_t handle) {
    if (handle) ReleaseSRWLockExclusive((SRWLOCK *)(uintptr_t)handle);
}

void lpp_rwlock_free(int64_t handle) {
    if (handle) free((void *)(uintptr_t)handle);
}

int64_t lpp_cpu_count(void) {
    SYSTEM_INFO si;
    GetSystemInfo(&si);
    return (int64_t)si.dwNumberOfProcessors;
}

typedef struct LppThreadTrampoline {
    int64_t fn_ptr;
    int64_t arg;
} LppThreadTrampoline;

static DWORD WINAPI lpp_thread_stub(LPVOID param) {
    LppThreadTrampoline *t = (LppThreadTrampoline *)param;
    int64_t fn_ptr = t->fn_ptr;
    int64_t arg = t->arg;
    free(t);
    typedef int64_t (*LppThreadFn)(int64_t);
    LppThreadFn fn = (LppThreadFn)(uintptr_t)fn_ptr;
    return (DWORD)fn(arg);
}

int64_t lpp_thread_spawn(int64_t fn_ptr, int64_t arg) {
    LppThreadTrampoline *t = (LppThreadTrampoline *)malloc(sizeof(LppThreadTrampoline));
    if (!t) lpp_panic("out of memory in thread_spawn");
    t->fn_ptr = fn_ptr;
    t->arg = arg;
    HANDLE hThread = CreateThread(NULL, 0, lpp_thread_stub, t, 0, NULL);
    if (!hThread) {
        free(t);
        lpp_panic("failed to create OS thread");
    }
    return (int64_t)(uintptr_t)hThread;
}

int64_t lpp_thread_join(int64_t handle) {
    if (!handle) return 0;
    HANDLE hThread = (HANDLE)(uintptr_t)handle;
    WaitForSingleObject(hThread, INFINITE);
    DWORD exit_code = 0;
    GetExitCodeThread(hThread, &exit_code);
    CloseHandle(hThread);
    return (int64_t)exit_code;
}

int64_t lpp_thread_pin(int64_t core_id) {
    if (core_id < 0 || core_id >= 64) return 0;
    DWORD_PTR mask = (DWORD_PTR)(1ULL << core_id);
    return SetThreadAffinityMask(GetCurrentThread(), mask) != 0 ? 1 : 0;
}

int64_t lpp_thread_id(void) {
    return (int64_t)GetCurrentThreadId();
}

#else
#include <pthread.h>
#include <unistd.h>

extern void lpp_panic(const char *fmt, ...);

int64_t lpp_mutex_new(void) {
    pthread_mutex_t *m = (pthread_mutex_t *)malloc(sizeof(pthread_mutex_t));
    if (!m) lpp_panic("out of memory in mutex_new");
    pthread_mutex_init(m, NULL);
    return (int64_t)(uintptr_t)m;
}

void lpp_mutex_lock(int64_t handle) {
    if (handle) pthread_mutex_lock((pthread_mutex_t *)(uintptr_t)handle);
}

int64_t lpp_mutex_trylock(int64_t handle) {
    if (!handle) return 0;
    return pthread_mutex_trylock((pthread_mutex_t *)(uintptr_t)handle) == 0 ? 1 : 0;
}

void lpp_mutex_unlock(int64_t handle) {
    if (handle) pthread_mutex_unlock((pthread_mutex_t *)(uintptr_t)handle);
}

void lpp_mutex_free(int64_t handle) {
    if (handle) {
        pthread_mutex_destroy((pthread_mutex_t *)(uintptr_t)handle);
        free((void *)(uintptr_t)handle);
    }
}

int64_t lpp_rwlock_new(void) {
    pthread_rwlock_t *rw = (pthread_rwlock_t *)malloc(sizeof(pthread_rwlock_t));
    if (!rw) lpp_panic("out of memory in rwlock_new");
    pthread_rwlock_init(rw, NULL);
    return (int64_t)(uintptr_t)rw;
}

void lpp_rwlock_rdlock(int64_t handle) {
    if (handle) pthread_rwlock_rdlock((pthread_rwlock_t *)(uintptr_t)handle);
}

void lpp_rwlock_wrlock(int64_t handle) {
    if (handle) pthread_rwlock_wrlock((pthread_rwlock_t *)(uintptr_t)handle);
}

void lpp_rwlock_rdunlock(int64_t handle) {
    if (handle) pthread_rwlock_unlock((pthread_rwlock_t *)(uintptr_t)handle);
}

void lpp_rwlock_wrunlock(int64_t handle) {
    if (handle) pthread_rwlock_unlock((pthread_rwlock_t *)(uintptr_t)handle);
}

void lpp_rwlock_free(int64_t handle) {
    if (handle) {
        pthread_rwlock_destroy((pthread_rwlock_t *)(uintptr_t)handle);
        free((void *)(uintptr_t)handle);
    }
}

int64_t lpp_cpu_count(void) {
    long count = sysconf(_SC_NPROCESSORS_ONLN);
    return count > 0 ? (int64_t)count : 1;
}

typedef struct LppThreadTrampoline {
    int64_t fn_ptr;
    int64_t arg;
} LppThreadTrampoline;

static void *lpp_thread_stub(void *param) {
    LppThreadTrampoline *t = (LppThreadTrampoline *)param;
    int64_t fn_ptr = t->fn_ptr;
    int64_t arg = t->arg;
    free(t);
    typedef int64_t (*LppThreadFn)(int64_t);
    LppThreadFn fn = (LppThreadFn)(uintptr_t)fn_ptr;
    int64_t res = fn(arg);
    return (void *)(intptr_t)res;
}

int64_t lpp_thread_spawn(int64_t fn_ptr, int64_t arg) {
    LppThreadTrampoline *t = (LppThreadTrampoline *)malloc(sizeof(LppThreadTrampoline));
    if (!t) lpp_panic("out of memory in thread_spawn");
    t->fn_ptr = fn_ptr;
    t->arg = arg;
    pthread_t *thread = (pthread_t *)malloc(sizeof(pthread_t));
    if (!thread) { free(t); lpp_panic("out of memory"); }
    if (pthread_create(thread, NULL, lpp_thread_stub, t) != 0) {
        free(t); free(thread);
        lpp_panic("failed to create thread");
    }
    return (int64_t)(uintptr_t)thread;
}

int64_t lpp_thread_join(int64_t handle) {
    if (!handle) return 0;
    pthread_t *thread = (pthread_t *)(uintptr_t)handle;
    void *res = NULL;
    pthread_join(*thread, &res);
    free(thread);
    return (int64_t)(intptr_t)res;
}

int64_t lpp_thread_pin(int64_t core_id) {
    (void)core_id;
    return 1;
}

int64_t lpp_thread_id(void) {
    return (int64_t)(uintptr_t)pthread_self();
}

#endif

#endif /* LPP_CONCUR_C */
