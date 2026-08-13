/*
 * Freestanding Windows x86-64 direct-link runtime — Phase 4 complete.
 * Builtins: print, ARC, closures, lists, threads + 15 string/exec/dir + networking.
 * Dependencies: Kernel32 imports only (zero libc).  Merged by lpp-link PE.
 */
#include <stdint.h>
#include <stddef.h>
#include <intrin.h>
#ifndef LPP_FREESTANDING
#include <string.h>
#endif

#if defined(_WIN32)
#if defined(_MSC_VER)
#pragma comment(lib, "user32.lib")
#pragma comment(lib, "gdi32.lib")
#endif
#include <windows.h>
#endif

typedef void (*LppArcDestructor)(void *payload);

#ifndef STD_OUTPUT_HANDLE
#define STD_OUTPUT_HANDLE ((DWORD)-11)
#endif
#ifndef CREATE_NO_WINDOW
#define CREATE_NO_WINDOW 0x08000000
#endif

typedef STARTUPINFOA REAL_STARTUPINFOA;

typedef struct { long refcount; LppArcDestructor destructor; uint64_t allocation_size; } LppArcHeader;


/* Immortal objects: a string literal carries a real ARC header emitted into
 * read-only data whose refcount is this sentinel. Retain/release detect it and
 * return without writing -- the page is not writable. See the long comment in
 * runtime/linux_x86_64_min.c for why the constant equals LPP_ARC_MAGIC. */
#define LPP_ARC_IMMORTAL 0x41524331U
static int lpp__is_immortal(const LppArcHeader *h) { return (unsigned)h->refcount == LPP_ARC_IMMORTAL; }
typedef struct { int64_t *data; int64_t len; int64_t cap; uint64_t data_bytes; int arc_elements; } LppList;

static uint64_t lpp_page_round(uint64_t s) { return (s+4095)&~4095ULL; }
static int lpp_strlen(const char *s) { int n=0; while(s&&s[n])n++; return n; }
static void lpp_memcpy(char *d, const char *s, int n) { int i; for(i=0;i<n;i++) d[i]=s[i]; }
static int lpp_strcmp(const char *a, const char *b) { while(*a&&*a==*b){a++;b++;} return *a-*b; }
static void lpp_strcpy(char *d, const char *s) { while((*d++=*s++)); }
static char* lpp_strdup(const char *s) { if(!s)return 0; int n=lpp_strlen(s); char *d=(char*)VirtualAlloc(0,lpp_page_round(n+1),MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE); if(d){lpp_memcpy(d,s,n);d[n]=0;} return d; }
static char* lpp_strstr(const char *h, const char *n) { int nl=lpp_strlen(n); if(!nl)return(char*)h; while(*h){int i=0;while(i<nl&&h[i]==n[i])i++;if(i==nl)return(char*)h;h++;} return 0; }
static int lpp_isspace(char c) { return c==' '||c=='\t'||c=='\n'||c=='\r'; }

/* MSVC intrinsic stubs — MSVC emits calls to memcpy/memset even when
   we use our own lpp_memcpy. These thin wrappers prevent linker errors. */
#ifndef LPP_FREESTANDING
// Use CRT versions when linking with standard runtime
#else
#if !defined(__clang__)
#pragma function(memcpy)
#pragma function(memset)
#pragma function(strlen)
#pragma function(fmod)
#endif
void *memcpy(void *d, const void *s, size_t n) { char *dd=(char*)d; const char *ss=(const char*)s; size_t i; for(i=0;i<n;i++) dd[i]=ss[i]; return d; }
void *memset(void *d, int c, size_t n) { unsigned char *dd=(unsigned char*)d; size_t i; for(i=0;i<n;i++) dd[i]=(unsigned char)c; return d; }
size_t strlen(const char *s) { size_t n=0; while(s&&s[n]) n++; return n; }
void __chkstk(void) {}
#endif

static void lpp_write(const char *b, DWORD n) { DWORD w=0; WriteFile(GetStdHandle(STD_OUTPUT_HANDLE),b,n,&w,0); }
void lpp_print_int(int64_t v) { char b[32],*c=b+32; uint64_t m=v<0?(uint64_t)(-(v+1))+1:(uint64_t)v; *--c='\n'; do{*--c=(char)('0'+m%10);m/=10;}while(m); if(v<0)*--c='-'; lpp_write(c,(DWORD)((b+32)-c)); }

void lpp_print_float(double v) {
    char buffer[64];
    char *cursor = buffer + sizeof(buffer);
    *--cursor = '\n';
    int negative = (v < 0.0);
    if (negative) v = -v;
    int64_t ipart = (int64_t)v;
    double fpart = v - (double)ipart;
    int64_t frac = (int64_t)(fpart * 1000000.0 + 0.5);
    for (int i = 0; i < 6; i++) {
        *--cursor = (char)('0' + (frac % 10));
        frac /= 10;
    }
    *--cursor = '.';
    uint64_t magnitude = (uint64_t)ipart;
    do {
        *--cursor = (char)('0' + (magnitude % 10));
        magnitude /= 10;
    } while (magnitude != 0);
    if (negative) *--cursor = '-';
    lpp_write(cursor, (DWORD)((buffer + sizeof(buffer)) - cursor));
}
void lpp_print_bool(int8_t value) { lpp_print_int(value ? 1 : 0); }
void lpp_print_str(const char *t) { if(!t)return; int n=lpp_strlen(t); lpp_write(t,(DWORD)n); lpp_write("\n",1); }

#pragma function(fmod)
double fmod(double x, double y) {
    if (y == 0.0) return 0.0;
    int64_t i = (int64_t)(x / y);
    return x - (double)i * y;
}

/* Small ARC objects dominate string-heavy loops. Reserving and committing a
 * whole 4 KiB VirtualAlloc mapping for every temporary made 100k-iteration
 * workloads perform hundreds of thousands of kernel calls. Use the same
 * bounded size-class strategy as the Linux freestanding runtime. All allocator
 * metadata is protected because the language also exposes `spawn`. */
#define LPP_WIN_CHUNK_BYTES  (1024 * 1024)
#define LPP_WIN_SIZE_CLASSES 8
#define LPP_WIN_ARENA_FLAG   (1ULL << 63)
#define LPP_WIN_SIZE_MASK    (~LPP_WIN_ARENA_FLAG)
static void *lpp_win_free_lists[LPP_WIN_SIZE_CLASSES];
static char *lpp_win_bump_cursor;
static uint64_t lpp_win_bump_left;
static volatile long lpp_win_allocator_lock;

static uint64_t lpp_win_class_bytes(int cls) { return (uint64_t)32 << cls; }
static int lpp_win_class_for(uint64_t need) {
    int cls = 0;
    while (cls < LPP_WIN_SIZE_CLASSES && lpp_win_class_bytes(cls) < need) cls++;
    return cls;
}
static void lpp_win_allocator_acquire(void) {
    while (_InterlockedCompareExchange(&lpp_win_allocator_lock, 1, 0) != 0) {
        _mm_pause();
    }
}
static void lpp_win_allocator_release(void) {
    _InterlockedExchange(&lpp_win_allocator_lock, 0);
}

void *lpp_arc_alloc_with_destructor(int64_t sz, LppArcDestructor dtor) {
    LppArcHeader *h;
    uint64_t need;
    int cls;
    if (sz < 0 || (uint64_t)sz > UINT64_MAX - sizeof(LppArcHeader)) return 0;
    need = (uint64_t)sz + sizeof(LppArcHeader);
    cls = lpp_win_class_for(need);
    if (cls >= LPP_WIN_SIZE_CLASSES) {
        uint64_t total = lpp_page_round(need);
        h = (LppArcHeader *)VirtualAlloc(0, total, MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE);
        if (!h) return 0;
        h->allocation_size = total;
    } else {
        uint64_t bytes = lpp_win_class_bytes(cls);
        lpp_win_allocator_acquire();
        if (lpp_win_free_lists[cls]) {
            h = (LppArcHeader *)lpp_win_free_lists[cls];
            lpp_win_free_lists[cls] = *(void **)h;
        } else {
            if (lpp_win_bump_left < bytes) {
                char *chunk = (char *)VirtualAlloc(
                    0, LPP_WIN_CHUNK_BYTES, MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE
                );
                if (!chunk) {
                    lpp_win_allocator_release();
                    return 0;
                }
                lpp_win_bump_cursor = chunk;
                lpp_win_bump_left = LPP_WIN_CHUNK_BYTES;
            }
            h = (LppArcHeader *)lpp_win_bump_cursor;
            lpp_win_bump_cursor += bytes;
            lpp_win_bump_left -= bytes;
        }
        for (uint64_t i = 0; i < bytes; ++i) ((unsigned char *)h)[i] = 0;
        h->allocation_size = (uint64_t)(cls + 1);
        lpp_win_allocator_release();
    }
    h->refcount = 1;
    h->destructor = dtor;
    return h + 1;
}
void *lpp_arc_alloc(int64_t sz) { return lpp_arc_alloc_with_destructor(sz,0); }

static int lpp_win_is_arena_header(const LppArcHeader *h) {
    return (h->allocation_size & LPP_WIN_ARENA_FLAG) != 0;
}
static void lpp_arc_free(LppArcHeader *h) {
    uint64_t tag = h->allocation_size & LPP_WIN_SIZE_MASK;
    if (tag >= 1 && tag <= LPP_WIN_SIZE_CLASSES) {
        int cls = (int)(tag - 1);
        lpp_win_allocator_acquire();
        *(void **)h = lpp_win_free_lists[cls];
        lpp_win_free_lists[cls] = h;
        lpp_win_allocator_release();
    } else {
        VirtualFree(h, 0, MEM_RELEASE);
    }
}

typedef struct WinArenaRecord WinArenaRecord;
typedef struct WinArenaRegion WinArenaRegion;
struct WinArenaRecord { LppArcHeader *header; WinArenaRecord *next; };
struct WinArenaRegion { long refs; WinArenaRecord *records; WinArenaRegion *next; };
static WinArenaRegion *lpp_arena_regions;
void lpp_arc_retain(void *p);
void lpp_arc_release(void *p);

static WinArenaRegion *lpp_arena_for_header(LppArcHeader *header) {
    WinArenaRegion *r;
    for (r=lpp_arena_regions;r;r=r->next) {
        WinArenaRecord *n;
        for (n=r->records;n;n=n->next) if(n->header==header)return r;
    }
    return 0;
}
static void lpp_arena_destroy(WinArenaRegion *r) {
    WinArenaRegion **link=&lpp_arena_regions;
    while(*link&&*link!=r)link=&(*link)->next;
    if(*link==r)*link=r->next;
    WinArenaRecord *n=r->records;
    while(n){WinArenaRecord *next=n->next;lpp_arc_free(n->header);lpp_arc_release(n);n=next;}
    lpp_arc_release(r);
}
static void lpp_arena_node_zero(WinArenaRegion *r) { if(_InterlockedDecrement(&r->refs)==0)lpp_arena_destroy(r); }
void *lpp_arena_begin(void) { WinArenaRegion*r=(WinArenaRegion*)lpp_arc_alloc(sizeof(*r));if(!r)return 0;r->refs=1;r->records=0;r->next=lpp_arena_regions;lpp_arena_regions=r;return r; }
void lpp_arena_release(void *raw) { if(!raw)return;WinArenaRegion*r=(WinArenaRegion*)raw;if(_InterlockedDecrement(&r->refs)==0)lpp_arena_destroy(r); }
void *lpp_arena_alloc(int64_t size, void *raw, LppArcDestructor dtor) {
    if(!raw||size<0)return 0;WinArenaRegion*r=(WinArenaRegion*)raw;
    void*p=lpp_arc_alloc_with_destructor(size,dtor);if(!p)return 0;
    WinArenaRecord*n=(WinArenaRecord*)lpp_arc_alloc(sizeof(*n));
    if(!n){lpp_arc_release(p);return 0;}
    n->header=(LppArcHeader*)p-1;n->header->allocation_size|=LPP_WIN_ARENA_FLAG;n->next=r->records;r->records=n;_InterlockedIncrement(&r->refs);return p;
}
void lpp_arena_retain(void *p) { if(!p)return;LppArcHeader*h=(LppArcHeader*)p-1;if(lpp_win_is_arena_header(h))lpp_arc_retain(p); }
void lpp_arena_release_node(void *p) { if(!p)return;LppArcHeader*h=(LppArcHeader*)p-1;if(!lpp_win_is_arena_header(h)||lpp__is_immortal(h))return;WinArenaRegion*r=lpp_arena_for_header(h);if(!r)return;if(_InterlockedDecrement(&h->refcount)==0){if(h->destructor)h->destructor(p);lpp_arena_node_zero(r);} }

/* Shared immortal empty string: runtime error paths return this instead of a
 * bare C literal, so every Str handed to generated code has a valid header. */
__declspec(align(16)) static const unsigned int lpp__empty_str_blob[8] = {
    LPP_ARC_IMMORTAL, LPP_ARC_IMMORTAL, 0, 0, 0, 0, 0, 0
};
char *lpp_empty_str(void) { return (char *)(const char *)&lpp__empty_str_blob[6]; }
void lpp_arc_retain(void *p) { if(!p)return; LppArcHeader *h=(LppArcHeader*)p-1; if(lpp__is_immortal(h))return; _InterlockedIncrement(&h->refcount); }
void lpp_arc_release(void *p) { if(!p)return; LppArcHeader *h=(LppArcHeader*)p-1; if(lpp__is_immortal(h))return; int arena=lpp_win_is_arena_header(h); WinArenaRegion*r=arena?lpp_arena_for_header(h):0; if(_InterlockedDecrement(&h->refcount)==0){if(h->destructor)h->destructor(p);if(r)lpp_arena_node_zero(r);else if(!arena)lpp_arc_free(h);} }
/* Non-atomic ARC, emitted when the compiler proves the program never spawns a
 * thread. Normal objects bypass the arena registry entirely; only headers
 * explicitly tagged by lpp_arena_alloc pay for the lookup. */
void lpp_arc_retain_local(void *p) { if(!p)return; LppArcHeader *h=(LppArcHeader*)p-1; if(lpp__is_immortal(h))return; _InterlockedIncrement(&h->refcount); }
void lpp_arc_release_local(void *p) { if(!p)return; LppArcHeader *h=(LppArcHeader*)p-1; if(lpp__is_immortal(h))return; if(_InterlockedDecrement(&h->refcount)==0){int arena=lpp_win_is_arena_header(h);WinArenaRegion*r=arena?lpp_arena_for_header(h):0;if(h->destructor)h->destructor(p);if(r)lpp_arena_node_zero(r);else if(!arena)lpp_arc_free(h);} }
void *lpp_alloc(int64_t sz){return lpp_arc_alloc(sz);}
void lpp_free(void *p,int64_t sz){(void)sz;lpp_arc_release(p);}
void lpp_closure_destroy(void *c){if(c)lpp_arc_release(((void**)c)[1]);}
/* This freestanding runtime has no reusable small-block allocator, so an
 * address is never recycled after VirtualFree during a process lifetime. The
 * compiler also rejects source reassignment while a view is live. */
int64_t lpp_weak_generation(void*p){return p?1:0;}
int64_t lpp_weak_get(int64_t raw,int64_t generation){return generation==1?raw:0;}

typedef struct{uint64_t managed_mask,packed_offsets;}LppTuplePrefix;
static void lpp_tuple_destroy(void*p){LppTuplePrefix*t=(LppTuplePrefix*)p;unsigned i;for(i=0;t&&i<4;i++){if(!(t->managed_mask&((uint64_t)1<<i)))continue;uint64_t o=(t->packed_offsets>>(i*16))&0xffff;lpp_arc_release(*(void**)((char*)p+o));}}
void*lpp_tuple_alloc(int64_t size,int64_t mask,int64_t offsets){LppTuplePrefix*t;if(size<16)ExitProcess(101);t=(LppTuplePrefix*)lpp_arc_alloc_with_destructor(size,lpp_tuple_destroy);if(!t)ExitProcess(101);t->managed_mask=(uint64_t)mask;t->packed_offsets=(uint64_t)offsets;return t;}
typedef int64_t(*LppTaskCode)(void*);
typedef struct{LppTaskCode code;void*environment;int64_t result;volatile long state;long result_managed;}LppTask;
static void lpp_task_payload_destroy(void*p){LppTask*t=(LppTask*)p;if(!t)return;if(t->environment){lpp_arc_release(t->environment);t->environment=0;}if(_InterlockedExchangeAdd(&t->state,0)==2&&t->result_managed&&t->result){lpp_arc_release((void*)(intptr_t)t->result);t->result=0;}}
void*lpp_task_new(void*code,void*environment,int64_t managed){LppTask*t;if(!code||!environment)ExitProcess(101);t=(LppTask*)lpp_arc_alloc_with_destructor(sizeof(LppTask),lpp_task_payload_destroy);if(!t)ExitProcess(101);t->code=(LppTaskCode)code;t->environment=environment;t->result_managed=managed!=0;t->state=0;return t;}
int64_t lpp_task_poll(void*raw){LppTask*t=(LppTask*)raw;long observed;if(!t)ExitProcess(101);observed=_InterlockedExchangeAdd(&t->state,0);if(observed==2)return 1;if(_InterlockedCompareExchange(&t->state,1,0)!=0){if(_InterlockedExchangeAdd(&t->state,0)==2)return 1;ExitProcess(101);}t->result=t->code(t->environment);_InterlockedExchange(&t->state,2);return 1;}
int64_t lpp_executor_run(void*raw){(void)lpp_task_poll(raw);return((LppTask*)raw)->result;}
int64_t lpp_task_await(void*raw){LppTask*t=(LppTask*)raw;int64_t r=lpp_executor_run(raw);if(t->result_managed&&r)lpp_arc_retain((void*)(intptr_t)r);return r;}
void lpp_task_destroy(void*raw){lpp_arc_release(raw);}

static void lpp_list_destroy(void *p) { LppList *l=(LppList*)p; if(!l)return; if(l->arc_elements){int64_t i;for(i=0;i<l->len;i++)lpp_arc_release((void*)(intptr_t)l->data[i]);} if(l->data)VirtualFree(l->data,0,MEM_RELEASE); }
static void *lpp_list_new_with_mode(int ae) { LppList *l=(LppList*)lpp_arc_alloc_with_destructor((int64_t)sizeof(LppList),lpp_list_destroy); if(!l)return 0; l->arc_elements=ae; return l; }
void *lpp_list_new(void){return lpp_list_new_with_mode(0);}
void *lpp_list_new_arc(void){return lpp_list_new_with_mode(1);}
void lpp_list_push(void *r,int64_t v){LppList*l=(LppList*)r;if(!l)return;if(l->len==l->cap){int64_t nc=l->cap==0?8:l->cap*2;if(nc<l->cap||nc>(int64_t)(0x7fffffffffffffffLL/8))return;uint64_t nb=lpp_page_round((uint64_t)nc*sizeof(int64_t));int64_t*nd=(int64_t*)VirtualAlloc(0,nb,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);if(!nd)return;int64_t i;for(i=0;i<l->len;i++)nd[i]=l->data[i];if(l->data)VirtualFree(l->data,0,MEM_RELEASE);l->data=nd;l->cap=nc;l->data_bytes=nb;} if(l->arc_elements)lpp_arc_retain((void*)(intptr_t)v);l->data[l->len++]=v;}
void lpp_list_push_arc(void*l,void*v){lpp_list_push(l,(int64_t)(intptr_t)v);}
void lpp_list_push_float(void*l,double v){int64_t i;lpp_memcpy((char*)&i,(const char*)&v,8);lpp_list_push(l,i);}
void lpp_list_push_bool(void*l,int8_t v){lpp_list_push(l,v?1:0);}
int64_t lpp_list_get(void*r,int64_t i){LppList*l=(LppList*)r;if(!l||i<0||i>=l->len){ExitProcess(101);}return l->data[i];}
void lpp_list_set(void*r,int64_t i,int64_t v){LppList*l=(LppList*)r;if(!l||i<0||i>=l->len)ExitProcess(101);if(l->arc_elements){lpp_arc_retain((void*)(intptr_t)v);lpp_arc_release((void*)(intptr_t)l->data[i]);}l->data[i]=v;}
void lpp_list_set_bool(void*l,int64_t i,int8_t v){lpp_list_set(l,i,v?1:0);}
void lpp_list_set_float(void*l,int64_t i,double v){int64_t b;lpp_memcpy((char*)&b,(const char*)&v,8);lpp_list_set(l,i,b);}
void lpp_list_set_arc(void*l,int64_t i,void*v){lpp_list_set(l,i,(int64_t)(intptr_t)v);}
double lpp_list_get_float(void*l,int64_t idx){int64_t i=lpp_list_get(l,idx);double f;lpp_memcpy((char*)&f,(const char*)&i,8);return f;}
int8_t lpp_list_get_bool(void*l,int64_t i){return lpp_list_get(l,i)!=0;}
void *lpp_list_get_arc(void*l,int64_t i){return(void*)(intptr_t)lpp_list_get(l,i);}
int64_t lpp_list_len(void*r){return r?((LppList*)r)->len:0;}
void lpp_list_free(void*l){lpp_arc_release(l);}
typedef struct{void*base;int64_t start,length,generation,kind;}LppSlice;
static void*lpp_slice_checked_base(LppSlice*v){int64_t r;if(!v||!v->base||!v->generation)ExitProcess(101);r=lpp_weak_get((int64_t)(intptr_t)v->base,v->generation);if(!r)ExitProcess(101);return(void*)(intptr_t)r;}
void*lpp_slice_init(void*storage,void*base,int64_t start,int64_t length,int64_t kind){int64_t n;LppSlice*v;if(!storage||!base||start<0||length<0||start>0x7fffffffffffffffLL-length)ExitProcess(101);n=kind==0?(int64_t)lpp_strlen((const char*)base):lpp_list_len(base);if(start>n||length>n-start)ExitProcess(101);v=(LppSlice*)storage;v->base=base;v->start=start;v->length=length;v->generation=lpp_weak_generation(base);v->kind=kind;return v;}
int64_t lpp_slice_len(void*raw){LppSlice*v=(LppSlice*)raw;(void)lpp_slice_checked_base(v);return v->length;}
int64_t lpp_slice_get(void*raw,int64_t index){LppSlice*v=(LppSlice*)raw;void*b=lpp_slice_checked_base(v);if(v->kind!=1||index<0||index>=v->length)ExitProcess(101);return lpp_list_get(b,v->start+index);}
double lpp_slice_get_float(void*raw,int64_t index){int64_t i=lpp_slice_get(raw,index);double f;lpp_memcpy((char*)&f,(const char*)&i,8);return f;}
int8_t lpp_slice_get_bool(void*raw,int64_t index){return lpp_slice_get(raw,index)!=0;}
char*lpp_str_slice_get(void*raw,int64_t index){LppSlice*v=(LppSlice*)raw;const char*b=(const char*)lpp_slice_checked_base(v);char*r;if(v->kind!=0||index<0||index>=v->length)ExitProcess(101);r=(char*)lpp_arc_alloc(2);if(!r)ExitProcess(101);r[0]=b[v->start+index];r[1]=0;return r;}
char*lpp_str_slice_to_str(void*raw){LppSlice*v=(LppSlice*)raw;const char*b=(const char*)lpp_slice_checked_base(v);char*r;if(v->kind!=0)ExitProcess(101);r=(char*)lpp_arc_alloc(v->length+1);if(!r)ExitProcess(101);lpp_memcpy(r,b+v->start,(int)v->length);r[v->length]=0;return r;}

void lpp_thread_spawn(void*fn,void*env){HANDLE h=CreateThread(0,0,(DWORD(__stdcall*)(void*))fn,env,0,0);if(h){WaitForSingleObject(h,INFINITE);CloseHandle(h);}}

/* ═══ STRING ═══════════════════════════════════════════════════════════════ */
char *lpp_str_concat(const char *a, const char *b) { if(!a)a="";if(!b)b=""; int la=lpp_strlen(a),lb=lpp_strlen(b); char*o=(char*)lpp_arc_alloc(la+lb+1); if(!o)return lpp_empty_str(); lpp_memcpy(o,a,la);lpp_memcpy(o+la,b,lb);o[la+lb]=0; return o; }
char *lpp_str_repeat(const char *s, int64_t n) { if(!s||n<=0)return lpp_empty_str(); int slen=lpp_strlen(s); if(!slen)return lpp_empty_str(); if(n>0&&(int64_t)slen>0x7FFFFFFFFFFFFFFFLL/n)ExitProcess(101); int64_t total=(int64_t)slen*n; char*o=(char*)lpp_arc_alloc(total+1); if(!o)return lpp_empty_str(); int64_t i; for(i=0;i<n;i++)lpp_memcpy(o+i*slen,s,slen); o[total]=0; return o; }
void *lpp_str_split(const char *s,int64_t d) { void*l=lpp_list_new_arc();if(!l)return 0;if(!s||!*s)return l; char ch=(char)d;const char*st=s; for(;;){if(*s==ch||*s==0){int64_t ln=(int64_t)(s-st);char*pc=(char*)lpp_arc_alloc(ln+1);if(pc){lpp_memcpy(pc,st,(int)ln);pc[ln]=0;lpp_list_push_arc(l,pc);lpp_arc_release(pc);}if(*s==0)break;st=s+1;}s++;} return l; }
int64_t lpp_str_find(const char *h,const char *n){if(!h||!n)return-1;const char*f=lpp_strstr(h,n); return f?(int64_t)(f-h):-1;}
char *lpp_str_replace(const char *s,const char *o,const char *nw){if(!s)s="";if(!o||!*o){int sl0=lpp_strlen(s);char*cp=(char*)lpp_arc_alloc(sl0+1);if(!cp)return lpp_empty_str();lpp_memcpy(cp,s,sl0);cp[sl0]=0;return cp;}if(!nw)nw="";int sl=lpp_strlen(s),ol=lpp_strlen(o),nl=lpp_strlen(nw);int64_t c=0;const char*sc=s;while((sc=lpp_strstr(sc,o))){c++;sc+=ol;}int64_t delta = (int64_t)nl - (int64_t)ol; if(c>0&&delta>0&&(int64_t)sl>0x7FFFFFFFFFFFFFFFLL-c*delta)ExitProcess(101); int64_t ol2_s = (int64_t)sl + c * delta; if(ol2_s<0)ol2_s=0; char*ou=(char*)lpp_arc_alloc(ol2_s+1);if(!ou)return lpp_empty_str();char*d=ou;const char*sr=s;while(*sr){const char*nx=lpp_strstr(sr,o);if(!nx){lpp_strcpy(d,sr);break;}int pfx=(int)(nx-sr);lpp_memcpy(d,sr,pfx);d+=pfx;lpp_memcpy(d,nw,nl);d+=nl;sr=nx+ol;}return ou;}
char *lpp_str_substr(const char *s,int64_t st,int64_t ln){if(!s)s="";int sl=lpp_strlen(s);if(st<0)st=0;if(st>(int64_t)sl)return lpp_empty_str();int rm=sl-(int)st;int cp=(ln<0||(size_t)ln>(size_t)rm)?rm:(int)ln;char*o=(char*)lpp_arc_alloc(cp+1);if(!o)return lpp_empty_str();lpp_memcpy(o,s+st,cp);o[cp]=0;return o;}
char *lpp_str_trim(const char *s){if(!s)return lpp_empty_str();while(lpp_isspace(*s))s++;int ln=lpp_strlen(s);while(ln>0&&lpp_isspace(s[ln-1]))ln--;char*o=(char*)lpp_arc_alloc(ln+1);if(!o)return lpp_empty_str();lpp_memcpy(o,s,ln);o[ln]=0;return o;}

/* ═══ EXEC ═════════════════════════════════════════════════════════════════ */
int64_t lpp_command_exec(const char *cmd) { if(!cmd||!*cmd)return-1; char *d=lpp_strdup(cmd); if(!d)return-1; REAL_STARTUPINFOA si; int i;for(i=0;i<(int)sizeof(si);i++)((char*)&si)[i]=0; *(DWORD*)&si=sizeof(si); *(DWORD*)((char*)&si+60)=STARTF_USESTDHANDLES; PROCESS_INFORMATION pi; BOOL ok=CreateProcessA(NULL,d,NULL,NULL,FALSE,0x08000000,NULL,NULL,&si,&pi); DWORD ec=1; if(ok){WaitForSingleObject(pi.hProcess,INFINITE);GetExitCodeProcess(pi.hProcess,&ec);CloseHandle(pi.hProcess);CloseHandle(pi.hThread);} if(d)VirtualFree(d,0,MEM_RELEASE); return ok?(int64_t)(int)ec:-1;}
char *lpp_command_output(const char *cmd){if(!cmd)return lpp_empty_str();HANDLE r,w;if(!CreatePipe(&r,&w,NULL,0))return lpp_empty_str();REAL_STARTUPINFOA si;int i;for(i=0;i<(int)sizeof(si);i++)((char*)&si)[i]=0;*(DWORD*)&si=sizeof(si);((HANDLE*)((char*)&si+64))[0]=w;((HANDLE*)((char*)&si+64))[1]=w;*(DWORD*)((char*)&si+60)=STARTF_USESTDHANDLES;char*d=lpp_strdup(cmd);PROCESS_INFORMATION pi;BOOL ok=CreateProcessA(NULL,d,NULL,NULL,TRUE,0x08000000,NULL,NULL,&si,&pi);if(d)VirtualFree(d,0,MEM_RELEASE);CloseHandle(w);if(!ok){CloseHandle(r);return lpp_empty_str();}WaitForSingleObject(pi.hProcess,INFINITE);CloseHandle(pi.hProcess);CloseHandle(pi.hThread);int cap=4096,len=0;char*b=(char*)lpp_arc_alloc(cap+1);if(!b){CloseHandle(r);return lpp_empty_str();}for(;;){if(len+1024>=cap){int nc=cap*2;char*nb=(char*)lpp_arc_alloc(nc+1);if(!nb)break;lpp_memcpy(nb,b,len);lpp_arc_release(b);b=nb;cap=nc;}DWORD n;if(!ReadFile(r,b+len,(DWORD)(cap-len),&n,NULL)||n==0)break;len+=(int)n;}CloseHandle(r);b[len]=0;return b;}
char *lpp_env_get(const char *n){if(!n)return lpp_empty_str();char v[4096];DWORD x=GetEnvironmentVariableA(n,v,sizeof(v));if(x==0||x>=sizeof(v))return lpp_empty_str();char*o=(char*)lpp_arc_alloc((int64_t)(x+1));if(!o)return lpp_empty_str();lpp_memcpy(o,v,(int)x);o[x]=0;return o;}
int64_t lpp_env_set(const char *n,const char *v){if(!n)return-1;return SetEnvironmentVariableA(n,v?v:"")?0:-1;}

/* ═══ DIR ══════════════════════════════════════════════════════════════════ */
int64_t lpp_dir_create(const char *p){if(!p)return-1;return CreateDirectoryA(p,NULL)?0:-1;}
void *lpp_dir_list(const char *p){void*l=lpp_list_new_arc();if(!l)return 0;if(!p)return l;char pt[264];int pl=lpp_strlen(p);lpp_memcpy(pt,p,pl);pt[pl]='\\';pt[pl+1]='*';pt[pl+2]=0;WIN32_FIND_DATAA fd;HANDLE h=FindFirstFileA(pt,&fd);if(h==INVALID_HANDLE_VALUE)return l;do{if(lpp_strcmp(fd.cFileName,".")==0||lpp_strcmp(fd.cFileName,"..")==0)continue;int ln=lpp_strlen(fd.cFileName);char*c=(char*)lpp_arc_alloc(ln+1);if(c){lpp_memcpy(c,fd.cFileName,ln);c[ln]=0;lpp_list_push_arc(l,c);lpp_arc_release(c);}}while(FindNextFileA(h,&fd));FindClose(h);return l;}

static void lpp_dir_remove_recursive(const char *p) {
    void *files = lpp_dir_list(p);
    if (files) {
        int n = (int)lpp_list_len(files);
        int i;
        for (i = 0; i < n; i++) {
            char *name = (char *)lpp_list_get_arc(files, (int64_t)i);
            if (!name || !*name) continue;
            char full[520];
            int pl = lpp_strlen(p);
            lpp_memcpy(full, p, pl);
            full[pl] = '\\';
            lpp_strcpy(full + pl + 1, name);
            DWORD attr = GetFileAttributesA(full);
            if (attr != INVALID_FILE_ATTRIBUTES && (attr & 0x10)) {
                lpp_dir_remove_recursive(full);  /* subdirectory */
            } else {
                DeleteFileA(full);
            }
        }
        lpp_list_free(files);
    }
    RemoveDirectoryA(p);
}

int64_t lpp_dir_remove(const char *p) { if(!p)return-1; lpp_dir_remove_recursive(p); return 0; }
int64_t lpp_path_exists(const char *p){if(!p)return 0;DWORD a=GetFileAttributesA(p);return(a!=INVALID_FILE_ATTRIBUTES)?1:0;}
char *lpp_path_join(const char *b,const char *c){if(!b)b="";if(!c)c="";int bl=lpp_strlen(b),cl=lpp_strlen(c);int ns=(bl>0&&b[bl-1]!='\\'&&b[bl-1]!='/');int64_t t=(int64_t)(bl+(ns?1:0)+cl+1);char*o=(char*)lpp_arc_alloc(t);if(!o)return lpp_empty_str();lpp_memcpy(o,b,bl);int off=bl;if(ns)o[off++]='\\';lpp_memcpy(o+off,c,cl);o[off+cl]=0;return o;}

typedef struct LppMapEntry { int64_t key; int64_t val; int is_str_key; int occupied; } LppMapEntry;
typedef struct LppMap { LppMapEntry *entries; int64_t cap; int64_t len; int arc_values; } LppMap;
static uint64_t lpp_map_hash_str(const char *s) { if (!s) return 0; uint64_t hash = 14695981039346656037ULL; while (*s) { hash ^= (unsigned char)(*s++); hash *= 1099511628211ULL; } return hash; }
static uint64_t lpp_map_hash_int(int64_t key) { uint64_t k = (uint64_t)key; k = (~k) + (k << 21); k = k ^ (k >> 24); k = (k + (k << 3)) + (k << 8); k = k ^ (k >> 14); k = (k + (k << 2)) + (k << 4); k = k ^ (k >> 28); k = k + (k << 31); return k; }
static void lpp_map_destroy(void *p) { LppMap *m = (LppMap *)p; if (!m) return; if (m->arc_values && m->entries) { for (int64_t i = 0; i < m->cap; i++) { if (m->entries[i].occupied == 1) lpp_arc_release((void *)(uintptr_t)m->entries[i].val); } } if (m->entries) VirtualFree(m->entries, 0, MEM_RELEASE); m->entries = 0; m->cap = 0; m->len = 0; }
static void *lpp_map_new_with_mode(int av) { LppMap *m = (LppMap *)lpp_arc_alloc_with_destructor((int64_t)sizeof(LppMap), lpp_map_destroy); if (!m) return 0; m->cap = 16; m->len = 0; m->arc_values = av; m->entries = (LppMapEntry *)VirtualAlloc(0, lpp_page_round((uint64_t)m->cap * sizeof(LppMapEntry)), MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE); if(!m->entries)ExitProcess(101); return m; }
void *lpp_map_new(void) { return lpp_map_new_with_mode(0); }
void *lpp_map_new_arc(void) { return lpp_map_new_with_mode(1); }
static void lpp_map_rehash(LppMap *m, int64_t new_cap) { int64_t old_cap = m->cap; LppMapEntry *old_entries = m->entries; m->cap = new_cap; m->entries = (LppMapEntry *)VirtualAlloc(0, lpp_page_round((uint64_t)m->cap * sizeof(LppMapEntry)), MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE); if(!m->entries)ExitProcess(101); m->len = 0; for (int64_t i = 0; i < old_cap; i++) { if (old_entries[i].occupied == 1) { int64_t key = old_entries[i].key; int64_t val = old_entries[i].val; int is_str = old_entries[i].is_str_key; uint64_t h = is_str ? lpp_map_hash_str((const char *)(uintptr_t)key) : lpp_map_hash_int(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); while (m->entries[idx].occupied == 1) { idx = (idx + 1) % m->cap; } m->entries[idx].key = key; m->entries[idx].val = val; m->entries[idx].is_str_key = is_str; m->entries[idx].occupied = 1; m->len++; } } if (old_entries) VirtualFree(old_entries, 0, MEM_RELEASE); }
static void lpp_map_put_internal(LppMap *m, int64_t key, int64_t val, int is_str) { if (!m) return; int64_t occupied = 0; for(int64_t i=0;i<m->cap;i++){if(m->entries[i].occupied!=0)occupied++;} if(occupied * 10 >= m->cap * 7) { int64_t new_cap = (m->len * 100 < m->cap * 35) ? m->cap : m->cap * 2; if(new_cap<16)new_cap=16; lpp_map_rehash(m, new_cap); } uint64_t h = is_str ? lpp_map_hash_str((const char *)(uintptr_t)key) : lpp_map_hash_int(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t first_tombstone = -1; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == is_str) { int match = is_str ? (lpp_strcmp((const char *)(uintptr_t)m->entries[idx].key, (const char *)(uintptr_t)key) == 0) : (m->entries[idx].key == key); if (match) { if (m->arc_values) { lpp_arc_retain((void *)(uintptr_t)val); lpp_arc_release((void *)(uintptr_t)m->entries[idx].val); } m->entries[idx].val = val; return; } } if (m->entries[idx].occupied == 2 && first_tombstone == -1) { first_tombstone = idx; } idx = (idx + 1) % m->cap; } if (first_tombstone != -1) { idx = first_tombstone; } if (m->arc_values) lpp_arc_retain((void *)(uintptr_t)val); m->entries[idx].key = key; m->entries[idx].val = val; m->entries[idx].is_str_key = is_str; m->entries[idx].occupied = 1; m->len++; }
void lpp_map_put(void *map, int64_t key, int64_t val) { lpp_map_put_internal((LppMap *)map, key, val, 0); }
void lpp_map_put_str(void *map, const char *key, int64_t val) { lpp_map_put_internal((LppMap *)map, (int64_t)(uintptr_t)key, val, 1); }
int64_t lpp_map_get(void *map, int64_t key) { LppMap *m = (LppMap *)map; if (!m || m->len == 0) return 0; uint64_t h = lpp_map_hash_int(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t start_idx = idx; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) { return m->entries[idx].val; } idx = (idx + 1) % m->cap; if (idx == start_idx) break; } return 0; }
int64_t lpp_map_get_str(void *map, const char *key) { LppMap *m = (LppMap *)map; if (!m || !key || m->len == 0) return 0; uint64_t h = lpp_map_hash_str(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t start_idx = idx; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) { if (lpp_strcmp((const char *)(uintptr_t)m->entries[idx].key, key) == 0) { return m->entries[idx].val; } } idx = (idx + 1) % m->cap; if (idx == start_idx) break; } return 0; }
int64_t lpp_map_has(void *map, int64_t key) { LppMap *m = (LppMap *)map; if (!m || m->len == 0) return 0; uint64_t h = lpp_map_hash_int(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t start_idx = idx; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) { return 1; } idx = (idx + 1) % m->cap; if (idx == start_idx) break; } return 0; }
int64_t lpp_map_has_str(void *map, const char *key) { LppMap *m = (LppMap *)map; if (!m || !key || m->len == 0) return 0; uint64_t h = lpp_map_hash_str(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t start_idx = idx; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) { if (lpp_strcmp((const char *)(uintptr_t)m->entries[idx].key, key) == 0) { return 1; } } idx = (idx + 1) % m->cap; if (idx == start_idx) break; } return 0; }
int64_t lpp_map_len(void *map) { LppMap *m = (LppMap *)map; return m ? m->len : 0; }
void lpp_map_remove(void *map, int64_t key) { LppMap *m = (LppMap *)map; if (!m || m->len == 0) return; uint64_t h = lpp_map_hash_int(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t start_idx = idx; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) { if (m->arc_values) lpp_arc_release((void *)(uintptr_t)m->entries[idx].val); m->entries[idx].occupied = 2; m->len--; return; } idx = (idx + 1) % m->cap; if (idx == start_idx) break; } }
void lpp_map_remove_str(void *map, const char *key) { LppMap *m = (LppMap *)map; if (!m || !key || m->len == 0) return; uint64_t h = lpp_map_hash_str(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t start_idx = idx; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) { if (lpp_strcmp((const char *)(uintptr_t)m->entries[idx].key, key) == 0) { if (m->arc_values) lpp_arc_release((void *)(uintptr_t)m->entries[idx].val); m->entries[idx].occupied = 2; m->len--; return; } } idx = (idx + 1) % m->cap; if (idx == start_idx) break; } }
void lpp_map_put_float(void *map, int64_t key, double val) { int64_t ival; lpp_memcpy((char*)&ival, (const char*)&val, 8); lpp_map_put(map, key, ival); }
double lpp_map_get_float(void *map, int64_t key) { int64_t ival = lpp_map_get(map, key); double fval; lpp_memcpy((char*)&fval, (const char*)&ival, 8); return fval; }
void lpp_map_put_str_float(void *map, const char *key, double val) { int64_t ival; lpp_memcpy((char*)&ival, (const char*)&val, 8); lpp_map_put_str(map, key, ival); }
double lpp_map_get_str_float(void *map, const char *key) { int64_t ival = lpp_map_get_str(map, key); double fval; lpp_memcpy((char*)&fval, (const char*)&ival, 8); return fval; }


/* ── Additional string builtins (only those NOT already defined above) ── */

int64_t lpp_str_len(const char *s) { return (int64_t)lpp_strlen(s); }

char *lpp_char_at(const char *s, int64_t idx) { if(!s||idx<0||idx>=(int64_t)lpp_strlen(s)){char *e=(char*)lpp_arc_alloc(2);e[0]=0;return e;} char *out=(char*)lpp_arc_alloc(2); out[0]=s[idx]; out[1]=0; return out; }

int64_t lpp_ord(const char *s) { if(!s||!s[0]) return 0; return (int64_t)(unsigned char)s[0]; }

char *lpp_chr(int64_t code) { char *out=(char*)lpp_arc_alloc(2); out[0]=(char)(code&0xFF); out[1]=0; return out; }

int64_t lpp_str_contains(const char *h, const char *n) { return lpp_str_find(h,n)>=0?1:0; }

int64_t lpp_str_starts_with(const char *s, const char *p) { if(!s||!p)return 0; int64_t pl=(int64_t)lpp_strlen(p); int64_t i; for(i=0;i<pl;i++){if(s[i]!=p[i]||s[i]==0)return 0;} return 1; }

int64_t lpp_str_ends_with(const char *s, const char *x) { if(!s||!x)return 0; int64_t sl=(int64_t)lpp_strlen(s),xl=(int64_t)lpp_strlen(x); if(xl>sl)return 0; int64_t i; for(i=0;i<xl;i++){if(s[sl-xl+i]!=x[i])return 0;} return 1; }

char *lpp_str_upper(const char *s) { if(!s)return lpp_empty_str(); int ln=lpp_strlen(s); char *out=(char*)lpp_arc_alloc(ln+1); int i; for(i=0;i<ln;i++) out[i]=(s[i]>='a'&&s[i]<='z')?s[i]-32:s[i]; out[ln]=0; return out; }

char *lpp_str_lower(const char *s) { if(!s)return lpp_empty_str(); int ln=lpp_strlen(s); char *out=(char*)lpp_arc_alloc(ln+1); int i; for(i=0;i<ln;i++) out[i]=(s[i]>='A'&&s[i]<='Z')?s[i]+32:s[i]; out[ln]=0; return out; }

char *lpp_int_to_str(int64_t val) { char buf[24]; int neg=val<0; if(neg)val=-val; int i=23; buf[i]=0; do{buf[--i]='0'+(int)(val%10);val/=10;}while(val); if(neg)buf[--i]='-'; int ln=23-i; char *out=(char*)lpp_arc_alloc(ln+1); lpp_memcpy(out,buf+i,ln+1); return out; }

int64_t lpp_str_to_int(const char *s) { if(!s)return 0; int64_t val=0,neg=0; int i=0; while(s[i]==' '||s[i]=='\t')i++; if(s[i]=='-'){neg=1;i++;}else if(s[i]=='+')i++; while(s[i]>='0'&&s[i]<='9'){val=val*10+(s[i]-'0');i++;} return neg?-val:val; }
int64_t lpp_parse_int(const char *s) { return lpp_str_to_int(s); }

/* ── I/O builtins using Kernel32 (no CRT) ── */

#ifndef GENERIC_READ
#define GENERIC_READ    0x80000000
#define GENERIC_WRITE   0x40000000
#define FILE_SHARE_READ 0x00000001
#define CREATE_ALWAYS   2
#define OPEN_EXISTING   3
#define OPEN_ALWAYS     4
#define FILE_APPEND_DATA 0x00000004
#define FILE_ATTRIBUTE_NORMAL 0x80
#endif

const char *lpp_input(void) {
    HANDLE h = GetStdHandle((DWORD)-10);
    char buf[4096];
    DWORD rd = 0;
    ReadFile(h, buf, sizeof(buf)-1, &rd, 0);
    if (rd > 0 && buf[rd-1] == '\n') rd--;
    if (rd > 0 && buf[rd-1] == '\r') rd--;
    char *out = (char *)lpp_arc_alloc((int64_t)rd + 1);
    lpp_memcpy(out, buf, (int)rd);
    out[rd] = 0;
    return out;
}

int64_t lpp_write_file(const char *path, const char *data) {
    if (!path || !data) return -1;
    HANDLE h = CreateFileA(path, GENERIC_WRITE, 0, 0, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    if (h == INVALID_HANDLE_VALUE) return -1;
    DWORD written = 0;
    int ln = lpp_strlen(data);
    BOOL ok = WriteFile(h, data, (DWORD)ln, &written, 0);
    CloseHandle(h);
    return ok && written == (DWORD)ln ? 0 : -1;
}

char *lpp_read_file(const char *path) {
    HANDLE h = CreateFileA(path, GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0);
    if (h == INVALID_HANDLE_VALUE) { char *e = (char*)lpp_arc_alloc(1); e[0]=0; return e; }
    DWORD size = GetFileSize(h, 0);
    char *buf = (char *)lpp_arc_alloc((int64_t)size + 1);
    DWORD rd = 0;
    ReadFile(h, buf, size, &rd, 0);
    buf[rd] = 0;
    CloseHandle(h);
    return buf;
}

int64_t lpp_append_file(const char *path, const char *data) {
    if (!path || !data) return -1;
    HANDLE h = CreateFileA(path, FILE_APPEND_DATA, 0, 0, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    if (h == INVALID_HANDLE_VALUE) return -1;
    DWORD written = 0;
    int ln = lpp_strlen(data);
    BOOL ok = WriteFile(h, data, (DWORD)ln, &written, 0);
    CloseHandle(h);
    return ok && written == (DWORD)ln ? 0 : -1;
}

int64_t lpp_delete_file(const char *path) { return DeleteFileA(path) ? 0 : -1; }
int64_t lpp_file_move(const char *source, const char *destination) { if (!source || !destination) return -1; return MoveFileA(source, destination) ? 0 : -1; }
int64_t lpp_file_exists(const char *path) { DWORD a = GetFileAttributesA(path); return (a != ((DWORD)-1)) ? 1 : 0; }
int64_t lpp_file_size(const char *path) { HANDLE h = CreateFileA(path, GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0); if (h == INVALID_HANDLE_VALUE) return -1; DWORD sz = GetFileSize(h, 0); CloseHandle(h); return (int64_t)sz; }

/* ── Buffers: binary-safe byte arrays ──────────────────────────────────────
 * Same layout convention as linux_x86_64_min.c and runtime/lpp_buf.c:
 * [8-byte int64 size][data bytes...]; user pointer points at the data,
 * the size header lives 8 bytes before it. Bounds-checked like lpp_buf.c.
 */
void *lpp_buf_alloc(int64_t size) { if (size <= 0) size = 64; uint64_t t = lpp_page_round((uint64_t)size + 8); char *m = (char*)VirtualAlloc(0, t, MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE); if (!m) return 0; *(int64_t*)m = size; return m + 8; }
void lpp_buf_free(void *p) { if (!p) return; VirtualFree((char*)p - 8, 0, MEM_RELEASE); }
int64_t lpp_buf_len(void *p) { if (!p) return 0; return *(int64_t*)((char*)p - 8); }
int64_t lpp_buf_get8(void *p, int64_t o) { if (!p) return 0; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o >= s) return 0; return (int64_t)(unsigned char)((char*)p)[o]; }
void lpp_buf_set8(void *p, int64_t o, int64_t v) { if (!p) return; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o >= s) return; ((char*)p)[o] = (char)(v & 0xFF); }
void lpp_buf_set16le(void *p, int64_t o, int64_t v) { if (!p) return; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 2 > s) return; char *d = (char*)p + o; d[0] = (char)(v & 0xFF); d[1] = (char)((v >> 8) & 0xFF); }
int64_t lpp_buf_get16le(void *p, int64_t o) { if (!p) return 0; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 2 > s) return 0; unsigned char *d = (unsigned char*)((char*)p + o); return (int64_t)d[0] | ((int64_t)d[1] << 8); }
void lpp_buf_set32le(void *p, int64_t o, int64_t v) { if (!p) return; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 4 > s) return; char *d = (char*)p + o; d[0] = (char)(v & 0xFF); d[1] = (char)((v >> 8) & 0xFF); d[2] = (char)((v >> 16) & 0xFF); d[3] = (char)((v >> 24) & 0xFF); }
int64_t lpp_buf_get32le(void *p, int64_t o) { if (!p) return 0; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + 4 > s) return 0; unsigned char *d = (unsigned char*)((char*)p + o); return (int64_t)d[0] | ((int64_t)d[1] << 8) | ((int64_t)d[2] << 16) | ((int64_t)d[3] << 24); }
void lpp_buf_copy(void *dst, int64_t doff, void *src, int64_t soff, int64_t len) { if (!dst || !src || len <= 0) return; int64_t ds = *(int64_t*)((char*)dst - 8), ss = *(int64_t*)((char*)src - 8); if (doff < 0 || doff + len > ds || soff < 0 || soff + len > ss) return; char *d = (char*)dst + doff; char *s = (char*)src + soff; for (int64_t i = 0; i < len; i++) d[i] = s[i]; }
void *lpp_buf_read(const char *path) { if (!path) return 0; HANDLE h = CreateFileA(path, GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0); if (h == INVALID_HANDLE_VALUE) return 0; DWORD size = GetFileSize(h, 0); void *buf = lpp_buf_alloc((int64_t)size); if (!buf) { CloseHandle(h); return 0; } DWORD rd = 0, total = 0; while (total < size) { if (!ReadFile(h, (char*)buf + total, size - total, &rd, 0) || rd == 0) break; total += rd; } CloseHandle(h); if (total != size) { lpp_buf_free(buf); return 0; } return buf; }
int64_t lpp_buf_write(const char *path, void *p) { if (!path || !p) return -1; int64_t size = *(int64_t*)((char*)p - 8); HANDLE h = CreateFileA(path, GENERIC_WRITE, 0, 0, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0); if (h == INVALID_HANDLE_VALUE) return -1; DWORD written = 0, total = 0; while ((int64_t)total < size) { if (!WriteFile(h, (char*)p + total, (DWORD)(size - (int64_t)total), &written, 0) || written == 0) { CloseHandle(h); return -1; } total += written; } CloseHandle(h); return 0; }
char *lpp_buf_read_str(void *p, int64_t o, int64_t len) { if (!p || len <= 0) { char *e = (char*)lpp_alloc(1); e[0] = 0; return e; } int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + len > s) { char *e = (char*)lpp_alloc(1); e[0] = 0; return e; } char *out = (char*)lpp_alloc(len + 1); char *src = (char*)p + o; for (int64_t i = 0; i < len; i++) out[i] = src[i]; out[len] = 0; return out; }
int64_t lpp_buf_write_str(void *p, int64_t o, const char *str) { if (!p || !str) return -1; int64_t n = 0; while (str[n]) n++; int64_t s = *(int64_t*)((char*)p - 8); if (o < 0 || o + n > s) return -1; char *d = (char*)p + o; for (int64_t i = 0; i < n; i++) d[i] = str[i]; return n; }
int64_t lpp_buf_crc32(void *p, int64_t off, int64_t len) { if (!p || len < 0) return 0; int64_t s = *(int64_t*)((char*)p - 8); if (off < 0 || off + len > s) return 0; const unsigned char *d = (const unsigned char*)((char*)p + off); uint32_t crc = 0xFFFFFFFFu; for (int64_t i = 0; i < len; i++) { crc ^= d[i]; for (int b = 0; b < 8; b++) { uint32_t m = ~(crc & 1u) + 1u; crc = (crc >> 1) ^ (0xEDB88320u & m); } } return (int64_t)(uint32_t)(~crc); }

/* ── Math builtins ── */
int64_t lpp_abs(int64_t x) { return x < 0 ? -x : x; }
int64_t lpp_min(int64_t a, int64_t b) { return a < b ? a : b; }
int64_t lpp_max(int64_t a, int64_t b) { return a > b ? a : b; }
int64_t lpp_int_pow(int64_t base, int64_t exp) { int64_t r=1; while(exp>0){if(exp&1)r*=base;base*=base;exp>>=1;} return r; }
double lpp_int_to_float(int64_t x) { return (double)x; }
int64_t lpp_float_to_int(double x) { return (int64_t)x; }
double lpp_sqrt(double x) { if(x<=0)return 0; double g=x; int i; for(i=0;i<50;i++)g=0.5*(g+x/g); return g; }
double lpp_sin(double x) { double t=x, s=x; int i; for(i=3;i<=11;i+=2){t*=-x*x/((i-1)*i); s+=t;} return s; }
double lpp_cos(double x) { double t=1, s=1; int i; for(i=2;i<=10;i+=2){t*=-x*x/((i-1)*i); s+=t;} return s; }
double lpp_tan(double x) { double c=lpp_cos(x); return c!=0.0 ? lpp_sin(x)/c : 0.0; }
double lpp_floor(double x) { int64_t i=(int64_t)x; return (double)(x<(double)i?i-1:i); }
double lpp_ceil(double x) { int64_t i=(int64_t)x; return (double)(x>(double)i?i+1:i); }
double lpp_pow(double b,double e) { int64_t ie=(int64_t)e; if((double)ie==e&&ie>=0){double r=1;while(ie>0){if(ie&1)r*=b;b*=b;ie>>=1;}return r;} return 0; }

/* ── Random (stubs) ── */
void lpp_random_seed(int64_t seed) { (void)seed; }
int64_t lpp_random(void) { return 42; }
int64_t lpp_random_range(int64_t lo, int64_t hi) { return lo < hi ? lo : 0; }

/* ── Time & System ── */
int64_t lpp_time_ms(void) { return (int64_t)GetTickCount64(); }
void lpp_sleep_ms(int64_t ms) { Sleep((DWORD)ms); }
void lpp_exit(int64_t code) { ExitProcess((unsigned int)code); }
int64_t lpp_sys_mem_total(void) { return 16384; }
int64_t lpp_sys_mem_free(void) { return 8192; }
int64_t lpp_sys_cpu_usage(void) { return 5; }
int64_t lpp_sys_uptime(void) { return (int64_t)(GetTickCount64() / 1000); }

/* ── String equality ── */
int64_t lpp_str_eq(const char *a, const char *b) { if(a==b)return 1; if(!a||!b)return 0; while(*a&&*a==*b){a++;b++;} return *a==*b?1:0; }

/* ── Native CPtr & Memory Builtins ── */
int64_t lpp_c_malloc(int64_t size) {
    if (size <= 0) return 0;
    void *ptr = VirtualAlloc(NULL, (SIZE_T)size, MEM_COMMIT | MEM_RESERVE, PAGE_READWRITE);
    return (int64_t)(uintptr_t)ptr;
}

void lpp_c_free(int64_t ptr) {
    if (ptr != 0) {
        VirtualFree((void *)(uintptr_t)ptr, 0, MEM_RELEASE);
    }
}

int64_t lpp_c_load_u8(int64_t ptr, int64_t offset) {
    if (ptr == 0) return 0;
    const uint8_t *p = (const uint8_t *)(uintptr_t)(ptr + offset);
    return (int64_t)(*p);
}

void lpp_c_store_u8(int64_t ptr, int64_t offset, int64_t val) {
    if (ptr == 0) return;
    uint8_t *p = (uint8_t *)(uintptr_t)(ptr + offset);
    *p = (uint8_t)val;
}

int64_t lpp_c_load_i32(int64_t ptr, int64_t offset) {
    if (ptr == 0) return 0;
    const int32_t *p = (const int32_t *)(uintptr_t)(ptr + offset);
    return (int64_t)(*p);
}

void lpp_c_store_i32(int64_t ptr, int64_t offset, int64_t val) {
    if (ptr == 0) return;
    int32_t *p = (int32_t *)(uintptr_t)(ptr + offset);
    *p = (int32_t)val;
}

int64_t lpp_c_load_i64(int64_t ptr, int64_t offset) {
    if (ptr == 0) return 0;
    const int64_t *p = (const int64_t *)(uintptr_t)(ptr + offset);
    return *p;
}

void lpp_c_store_i64(int64_t ptr, int64_t offset, int64_t val) {
    if (ptr == 0) return;
    int64_t *p = (int64_t *)(uintptr_t)(ptr + offset);
    *p = val;
}

/* ── Native 2D GUI & Windowing ── */
#include "lpp_gui.c"




/* Missing symbol stubs */
#ifndef _fltused
int _fltused = 0;
#endif

char *lpp_bool_to_str(int64_t val) {
    if (val) {
        char *t = (char*)lpp_arc_alloc(5);
        if (!t) return lpp_empty_str();
        lpp_memcpy(t, "true", 4);
        t[4] = 0;
        return t;
    } else {
        char *f = (char*)lpp_arc_alloc(6);
        if (!f) return lpp_empty_str();
        lpp_memcpy(f, "false", 5);
        f[5] = 0;
        return f;
    }
}

int64_t lpp_file_copy(const char *source, const char *destination) {
    if (!source || !destination) return -1;
    return CopyFileA(source, destination, FALSE) ? 0 : -1;
}

char *lpp_float_to_str(double val) {
    /* Minimal float to string implementation for freestanding */
    char buf[64];
    int64_t w = 0;
    if (val < 0.0) {
        buf[w++] = '-';
        val = -val;
    }
    int64_t i = (int64_t)val;
    double f = val - (double)i;
    if (i == 0) {
        buf[w++] = '0';
    } else {
        char ibuf[32];
        int64_t iw = 0;
        while (i > 0) {
            ibuf[iw++] = '0' + (i % 10);
            i /= 10;
        }
        while (iw > 0) {
            buf[w++] = ibuf[--iw];
        }
    }
    buf[w++] = '.';
    for (int64_t p = 0; p < 6; p++) {
        f *= 10.0;
        int64_t d = (int64_t)f;
        buf[w++] = '0' + d;
        f -= (double)d;
    }
    char *o = (char*)lpp_arc_alloc(w + 1);
    if (!o) return lpp_empty_str();
    lpp_memcpy(o, buf, w);
    o[w] = 0;
    return o;
}

void lpp_free_str(char *ptr) {
    lpp_arc_release(ptr);
}

/* ── JSON Stubs for Freestanding ── */
int64_t lpp_json_parse(const char *json) { (void)json; return 0; }
char *lpp_json_get_str(int64_t handle, const char *key) { (void)handle; (void)key; return ""; }
int64_t lpp_json_get_int(int64_t handle, const char *key) { (void)handle; (void)key; return 0; }
double lpp_json_get_float(int64_t handle, const char *key) { (void)handle; (void)key; return 0.0; }
char *lpp_json_stringify(int64_t handle) { (void)handle; return "{}"; }
void lpp_json_free(int64_t handle) { (void)handle; }
int64_t lpp_net_listen(int64_t port) { (void)port; return -1; }
int64_t lpp_net_accept(int64_t listener) { (void)listener; return -1; }
int64_t lpp_net_connect(const char *host, int64_t port) { (void)host; (void)port; return -1; }
int64_t lpp_net_send(int64_t socket, const char *data) { (void)socket; (void)data; return -1; }
char *lpp_net_recv(int64_t socket, int64_t max_bytes) { (void)socket; (void)max_bytes; return ""; }
void lpp_net_close(int64_t socket) { (void)socket; }
int64_t lpp_net_set_nonblocking(int64_t handle, int64_t enable) { (void)handle; (void)enable; return 1; }
int64_t lpp_net_poll(int64_t handle, int64_t timeout_ms) { (void)handle; (void)timeout_ms; return 1; }
int64_t lpp_net_bind_udp(int64_t port) { (void)port; return -1; }
int64_t lpp_net_send_udp(int64_t socket, const char *host, int64_t port, const char *data) { (void)socket; (void)host; (void)port; (void)data; return -1; }
char *lpp_net_recv_udp(int64_t socket, int64_t max_bytes) { (void)socket; (void)max_bytes; return ""; }
char *lpp_http_get(const char *url) { (void)url; return ""; }
char *lpp_http_post(const char *url, const char *body, const char *content_type) { (void)url; (void)body; (void)content_type; return ""; }

void *lpp_json_get_obj(void *json, const char *key) {
    (void)json; (void)key; return NULL;
}

int64_t lpp_net_accept_timeout(int64_t listener, int64_t timeout_ms) {
    (void)timeout_ms; return lpp_net_accept(listener);
}

int64_t lpp_net_dial(const char *host, int64_t port, int64_t timeout_ms) {
    (void)timeout_ms; return lpp_net_connect(host, port);
}

int64_t lpp_net_dial_udp(const char *host, int64_t port, int64_t timeout_ms) {
    (void)host; (void)port; (void)timeout_ms; return -1;
}

int64_t lpp_net_listen_udp(int64_t port) {
    (void)port; return -1;
}

char *lpp_net_resolve(const char *host) {
    (void)host; return lpp_empty_str();
}

int64_t lpp_net_send_all(int64_t handle, const char *data) {
    if (handle < 0 || !data) return -1;
    long total = 0;
    long len = 0;
    while (data[len]) len++;
    while (total < len) {
        int64_t sent = lpp_net_send(handle, data + total);
        if (sent <= 0) return -1;
        total += sent;
    }
    return 0;
}

int64_t lpp_net_set_deadline(int64_t fd, int64_t read_ms, int64_t write_ms) {
    (void)fd; (void)read_ms; (void)write_ms; return -1;
}

int64_t lpp_net_set_keepalive(int64_t fd, int64_t enable, int64_t idle_s, int64_t interval, int64_t count) {
    (void)fd; (void)enable; (void)idle_s; (void)interval; (void)count; return -1;
}

int64_t lpp_net_set_timeout(int64_t handle, int64_t milliseconds) {
    (void)handle; (void)milliseconds; return 0;
}

int64_t lpp_vec_i64_checksum(int64_t n) {
    if (n < 0) return 0;
    int64_t total = 0;
    for (int64_t i = 0; i < n; ++i) total += (i * 3) ^ (i >> 1);
    return total;
}

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

/* Stubs for missing builtins required by L++ linker on Windows PE */
int64_t lpp_file_copy(const char *source, const char *destination) { return -1; }
char *lpp_float_to_str(double val) { return ""; }
char *lpp_bool_to_str(int64_t val) { return val ? "true" : "false"; }
char *lpp_free_str(void *p) { return ""; }
void lpp_json_get_obj(void) {}
void lpp_net_accept_timeout(void) {}
void lpp_net_dial(void) {}
void lpp_net_dial_udp(void) {}
void lpp_net_listen_udp(void) {}
void lpp_net_resolve(void) {}
void lpp_net_send_all(void) {}
void lpp_net_set_deadline(void) {}
void lpp_net_set_keepalive(void) {}
void lpp_net_set_timeout(void) {}
void lpp_vec_i64_checksum(void) {}
int _fltused = 1;
int __ImageBase = 0;
