/* ── SIMD i64x2 builtins ─────────────────────────────────────────────────
 * The Cranelift and LLVM backends lower these to native SSE2 instructions
 * inline. The host linker still requires the symbols to be resolved when
 * building with cl.exe/cc; these stubs satisfy that requirement.
 * Programs that actually use VectorI64x2 will have the inlined instructions
 * and never reach these stub bodies.
 * --------------------------------------------------------------------- */
typedef struct { int64_t lo; int64_t hi; } LppVecI64x2;
LppVecI64x2 lpp_vec_i64x2(int64_t lo, int64_t hi) { LppVecI64x2 v; v.lo = lo; v.hi = hi; return v; }
LppVecI64x2 lpp_vec_i64x2_splat(int64_t x) { LppVecI64x2 v; v.lo = x; v.hi = x; return v; }
LppVecI64x2 lpp_vec_i64x2_add(LppVecI64x2 a, LppVecI64x2 b) { LppVecI64x2 r; r.lo = a.lo + b.lo; r.hi = a.hi + b.hi; return r; }
LppVecI64x2 lpp_vec_i64x2_sub(LppVecI64x2 a, LppVecI64x2 b) { LppVecI64x2 r; r.lo = a.lo - b.lo; r.hi = a.hi - b.hi; return r; }
LppVecI64x2 lpp_vec_i64x2_mul(LppVecI64x2 a, LppVecI64x2 b) { LppVecI64x2 r; r.lo = a.lo * b.lo; r.hi = a.hi * b.hi; return r; }
LppVecI64x2 lpp_vec_i64x2_xor(LppVecI64x2 a, LppVecI64x2 b) { LppVecI64x2 r; r.lo = a.lo ^ b.lo; r.hi = a.hi ^ b.hi; return r; }
LppVecI64x2 lpp_vec_i64x2_shr(LppVecI64x2 a, int64_t shift) { LppVecI64x2 r; r.lo = a.lo >> shift; r.hi = a.hi >> shift; return r; }
LppVecI64x2 lpp_vec_i64x2_shr_var(LppVecI64x2 a, LppVecI64x2 b) { LppVecI64x2 r; r.lo = a.lo >> b.lo; r.hi = a.hi >> b.hi; return r; }
int64_t     lpp_vec_i64x2_extract(LppVecI64x2 v, int64_t idx) { return idx == 0 ? v.lo : v.hi; }
int64_t     lpp_vec_i64x2_sum(LppVecI64x2 v) { return v.lo + v.hi; }

/* ── Atomics & Concurrency (F3 & F4) ── */
int64_t lpp_atomic_alloc(int64_t v) { int64_t *p=(int64_t*)lpp_arc_alloc(8); if(!p)ExitProcess(101); *p=v; return (int64_t)(uintptr_t)p; }
int64_t lpp_atomic_new(int64_t v) { return lpp_atomic_alloc(v); }
void lpp_atomic_free(int64_t p) { if(p) lpp_arc_release((void*)(uintptr_t)p); }
int64_t lpp_atomic_load(int64_t p) { return _InterlockedOr64((volatile LONG64 *)(uintptr_t)p, 0); }
int64_t lpp_atomic_load_acq(int64_t p) { return _InterlockedOr64((volatile LONG64 *)(uintptr_t)p, 0); }
int64_t lpp_atomic_load_relaxed(int64_t p) { return *(volatile int64_t *)(uintptr_t)p; }
void lpp_atomic_store(int64_t p, int64_t v) { _InterlockedExchange64((volatile LONG64 *)(uintptr_t)p, (LONG64)v); }
void lpp_atomic_store_rel(int64_t p, int64_t v) { _InterlockedExchange64((volatile LONG64 *)(uintptr_t)p, (LONG64)v); }
void lpp_atomic_store_relaxed(int64_t p, int64_t v) { *(volatile int64_t *)(uintptr_t)p = v; }
int64_t lpp_atomic_add(int64_t p, int64_t v) { return (int64_t)_InterlockedExchangeAdd64((volatile LONG64 *)(uintptr_t)p, (LONG64)v); }
int64_t lpp_atomic_sub(int64_t p, int64_t v) { return (int64_t)_InterlockedExchangeAdd64((volatile LONG64 *)(uintptr_t)p, -(LONG64)v); }
int64_t lpp_atomic_and(int64_t p, int64_t v) { return (int64_t)_InterlockedAnd64((volatile LONG64 *)(uintptr_t)p, (LONG64)v); }
int64_t lpp_atomic_or(int64_t p, int64_t v) { return (int64_t)_InterlockedOr64((volatile LONG64 *)(uintptr_t)p, (LONG64)v); }
int64_t lpp_atomic_xor(int64_t p, int64_t v) { return (int64_t)_InterlockedXor64((volatile LONG64 *)(uintptr_t)p, (LONG64)v); }
int64_t lpp_atomic_swap(int64_t p, int64_t v) { return (int64_t)_InterlockedExchange64((volatile LONG64 *)(uintptr_t)p, (LONG64)v); }
int64_t lpp_atomic_cas(int64_t p, int64_t e, int64_t d) { return (int64_t)_InterlockedCompareExchange64((volatile LONG64 *)(uintptr_t)p, (LONG64)d, (LONG64)e); }
int64_t lpp_atomic_cas_weak(int64_t p, int64_t e, int64_t d) { return (int64_t)_InterlockedCompareExchange64((volatile LONG64 *)(uintptr_t)p, (LONG64)d, (LONG64)e); }
int64_t lpp_atomic_load32(int64_t p) { return (int64_t)_InterlockedOr((volatile LONG *)(uintptr_t)p, 0); }
void lpp_atomic_store32(int64_t p, int64_t v) { _InterlockedExchange((volatile LONG *)(uintptr_t)p, (LONG)v); }
int64_t lpp_atomic_add32(int64_t p, int64_t v) { return (int64_t)_InterlockedExchangeAdd((volatile LONG *)(uintptr_t)p, (LONG)v); }
int64_t lpp_atomic_cas32(int64_t p, int64_t e, int64_t d) { return (int64_t)_InterlockedCompareExchange((volatile LONG *)(uintptr_t)p, (LONG)d, (LONG)e); }
void lpp_atomic_fence(void) { MemoryBarrier(); }
void lpp_atomic_fence_acq(void) { MemoryBarrier(); }
void lpp_atomic_fence_rel(void) { MemoryBarrier(); }
void lpp_cpu_pause(void) { YieldProcessor(); }

int64_t lpp_mutex_new(void) { CRITICAL_SECTION *cs=(CRITICAL_SECTION*)lpp_arc_alloc(sizeof(CRITICAL_SECTION)); if(!cs)ExitProcess(101); InitializeCriticalSection(cs); return (int64_t)(uintptr_t)cs; }
void lpp_mutex_lock(int64_t h) { if(h) EnterCriticalSection((CRITICAL_SECTION*)(uintptr_t)h); }
int64_t lpp_mutex_trylock(int64_t h) { if(!h)return 0; return TryEnterCriticalSection((CRITICAL_SECTION*)(uintptr_t)h)?1:0; }
void lpp_mutex_unlock(int64_t h) { if(h) LeaveCriticalSection((CRITICAL_SECTION*)(uintptr_t)h); }
void lpp_mutex_free(int64_t h) { if(h) { DeleteCriticalSection((CRITICAL_SECTION*)(uintptr_t)h); lpp_arc_release((void*)(uintptr_t)h); } }

int64_t lpp_rwlock_new(void) { SRWLOCK *rw=(SRWLOCK*)lpp_arc_alloc(sizeof(SRWLOCK)); if(!rw)ExitProcess(101); InitializeSRWLock(rw); return (int64_t)(uintptr_t)rw; }
void lpp_rwlock_rdlock(int64_t h) { if(h) AcquireSRWLockShared((SRWLOCK*)(uintptr_t)h); }
void lpp_rwlock_wrlock(int64_t h) { if(h) AcquireSRWLockExclusive((SRWLOCK*)(uintptr_t)h); }
void lpp_rwlock_rdunlock(int64_t h) { if(h) ReleaseSRWLockShared((SRWLOCK*)(uintptr_t)h); }
void lpp_rwlock_wrunlock(int64_t h) { if(h) ReleaseSRWLockExclusive((SRWLOCK*)(uintptr_t)h); }
void lpp_rwlock_free(int64_t h) { if(h) lpp_arc_release((void*)(uintptr_t)h); }

int64_t lpp_cpu_count(void) { SYSTEM_INFO si; GetSystemInfo(&si); return (int64_t)si.dwNumberOfProcessors; }
typedef struct LppMinTrampoline { int64_t fn; int64_t arg; } LppMinTrampoline;
static DWORD WINAPI lpp_min_thread_stub(LPVOID p) { LppMinTrampoline *t=(LppMinTrampoline*)p; int64_t f=t->fn, a=t->arg; lpp_arc_release(t); typedef int64_t(*LppFn)(int64_t); return (DWORD)((LppFn)(uintptr_t)f)(a); }
int64_t lpp_thread_spawn(int64_t fn, int64_t arg) { LppMinTrampoline *t=(LppMinTrampoline*)lpp_arc_alloc(sizeof(LppMinTrampoline)); if(!t)ExitProcess(101); t->fn=fn; t->arg=arg; HANDLE h=CreateThread(NULL,0,lpp_min_thread_stub,t,0,NULL); if(!h){lpp_arc_release(t);ExitProcess(101);} return (int64_t)(uintptr_t)h; }
int64_t lpp_thread_join(int64_t h) { if(!h)return 0; HANDLE ht=(HANDLE)(uintptr_t)h; WaitForSingleObject(ht,INFINITE); DWORD ec=0; GetExitCodeThread(ht,&ec); CloseHandle(ht); return (int64_t)ec; }
int64_t lpp_thread_pin(int64_t core) { if(core<0||core>=64)return 0; return SetThreadAffinityMask(GetCurrentThread(),(DWORD_PTR)(1ULL<<core))!=0?1:0; }
int64_t lpp_thread_id(void) { return (int64_t)GetCurrentThreadId(); }

/* ── RNG and Clock (F14) ── */
typedef struct LppMinRng { uint64_t s; } LppMinRng;
int64_t lpp_rng_new(int64_t s) { LppMinRng*r=(LppMinRng*)lpp_arc_alloc(sizeof(LppMinRng)); if(!r)ExitProcess(101); r->s=s==0?0x853c49e6748fea9bULL:(uint64_t)s; return (int64_t)(uintptr_t)r; }
int64_t lpp_rng_next(int64_t h) { LppMinRng*r=(LppMinRng*)(uintptr_t)h; if(!r)return 0; uint64_t z=(r->s+=0x9e3779b97f4a7c15ULL); z=(z^(z>>30))*0xbf58476d1ce4e5b9ULL; z=(z^(z>>27))*0x94d049bb133111ebULL; return (int64_t)(z^(z>>31)); }
int64_t lpp_rng_range(int64_t h,int64_t mi,int64_t ma) { if(mi>=ma)return mi; uint64_t d=(uint64_t)(ma-mi+1); return mi+(int64_t)((uint64_t)lpp_rng_next(h)%d); }
double lpp_rng_float(int64_t h) { return (double)((uint64_t)lpp_rng_next(h)>>11)*(1.0/9007199254740992.0); }
void lpp_rng_free(int64_t h) { if(h) lpp_arc_release((void*)(uintptr_t)h); }

typedef struct LppMinClock { int v; int64_t t; } LppMinClock;
int64_t lpp_clock_new(int64_t i) { LppMinClock*c=(LppMinClock*)lpp_arc_alloc(sizeof(LppMinClock)); if(!c)ExitProcess(101); c->v=(i!=0); c->t=i; return (int64_t)(uintptr_t)c; }
int64_t lpp_clock_now(int64_t h) { LppMinClock*c=(LppMinClock*)(uintptr_t)h; if(!c)return 0; if(c->v)return c->t; FILETIME ft; GetSystemTimeAsFileTime(&ft); ULARGE_INTEGER u; u.LowPart=ft.dwLowDateTime; u.HighPart=ft.dwHighDateTime; return (int64_t)((u.QuadPart-116444736000000000ULL)/10000ULL); }
void lpp_clock_advance(int64_t h,int64_t d) { LppMinClock*c=(LppMinClock*)(uintptr_t)h; if(c&&c->v) c->t+=d; }
void lpp_clock_free(int64_t h) { if(h) lpp_arc_release((void*)(uintptr_t)h); }


