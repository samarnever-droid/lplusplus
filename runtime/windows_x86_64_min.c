/*
 * Freestanding Windows x86-64 direct-link runtime — Phase 4 complete.
 * Builtins: print, ARC, closures, lists, threads + 15 string/exec/dir + networking.
 * Dependencies: Kernel32 imports only (zero libc).  Merged by lpp-link PE.
 */
#include <stdint.h>
#include <intrin.h>
#include <string.h>

typedef void (*LppArcDestructor)(void *payload);
typedef void *HANDLE;
typedef unsigned long DWORD;
typedef int BOOL;
typedef unsigned long long SIZE_T;

__declspec(dllimport) HANDLE __stdcall GetStdHandle(DWORD h);
__declspec(dllimport) BOOL   __stdcall WriteFile(HANDLE h, const void *b, DWORD n, DWORD *w, void *o);
__declspec(dllimport) void * __stdcall VirtualAlloc(void *a, SIZE_T s, DWORD t, DWORD p);
__declspec(dllimport) BOOL   __stdcall VirtualFree(void *a, SIZE_T s, DWORD t);
__declspec(dllimport) HANDLE __stdcall CreateThread(void *s, SIZE_T z, DWORD (__stdcall *f)(void*), void *p, DWORD f2, DWORD *t);
__declspec(dllimport) DWORD  __stdcall WaitForSingleObject(HANDLE h, DWORD ms);
__declspec(dllimport) BOOL   __stdcall CloseHandle(HANDLE h);
__declspec(dllimport) BOOL   __stdcall CreateProcessA(const char *a, char *c, void *s, void *t, BOOL i, DWORD f, void *e, const char *d, void *si, void *pi);
__declspec(dllimport) BOOL   __stdcall GetExitCodeProcess(HANDLE p, DWORD *c);
__declspec(dllimport) BOOL   __stdcall CreatePipe(HANDLE *r, HANDLE *w, void *a, DWORD s);
__declspec(dllimport) BOOL   __stdcall ReadFile(HANDLE f, void *b, DWORD n, DWORD *rx, void *o);
__declspec(dllimport) DWORD  __stdcall GetEnvironmentVariableA(const char *n, char *b, DWORD s);
__declspec(dllimport) BOOL   __stdcall SetEnvironmentVariableA(const char *n, const char *v);
__declspec(dllimport) BOOL   __stdcall CreateDirectoryA(const char *p, void *a);
__declspec(dllimport) BOOL   __stdcall RemoveDirectoryA(const char *p);
__declspec(dllimport) HANDLE __stdcall FindFirstFileA(const char *p, void *d);
__declspec(dllimport) BOOL   __stdcall FindNextFileA(HANDLE f, void *d);
__declspec(dllimport) BOOL   __stdcall FindClose(HANDLE f);
__declspec(dllimport) DWORD  __stdcall GetFileAttributesA(const char *p);
__declspec(dllimport) BOOL   __stdcall DeleteFileA(const char *p);
__declspec(dllimport) void   __stdcall Sleep(DWORD ms);

#define STD_OUTPUT_HANDLE ((DWORD)-11)
#define MEM_COMMIT  0x00001000UL
#define MEM_RESERVE 0x00002000UL
#define MEM_RELEASE 0x00008000UL
#define PAGE_READWRITE 0x00000004UL
#define INFINITE 0xFFFFFFFF
#define INVALID_HANDLE_VALUE ((HANDLE)(intptr_t)-1)
#define INVALID_FILE_ATTRIBUTES ((DWORD)-1)
#define TRUE 1
#define FALSE 0
#define STARTF_USESTDHANDLES 0x100
#define CREATE_NO_WINDOW 0x08000000

/* real STARTUPINFOA = 104 bytes, PROCESS_INFORMATION = 24 bytes */
typedef struct { char _[104]; } REAL_STARTUPINFOA;
typedef struct { HANDLE hProcess; HANDLE hThread; DWORD dwProcessId; DWORD dwThreadId; } PROCESS_INFORMATION;

/* WIN32_FIND_DATAA = ~320 bytes */
typedef struct { DWORD a; DWORD b; DWORD c; DWORD d; DWORD e; DWORD f; DWORD g;
    char   cFileName[260]; char cAlternateFileName[14];
    DWORD  h; DWORD  i; DWORD  j; } WIN32_FIND_DATAA;

typedef struct { long refcount; LppArcDestructor destructor; uint64_t allocation_size; } LppArcHeader;
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
#pragma function(memcpy)
#pragma function(memset)
#pragma function(strlen)
#pragma function(fmod)
void *memcpy(void *d, const void *s, size_t n) { char *dd=(char*)d; const char *ss=(const char*)s; size_t i; for(i=0;i<n;i++) dd[i]=ss[i]; return d; }
void *memset(void *d, int c, size_t n) { unsigned char *dd=(unsigned char*)d; size_t i; for(i=0;i<n;i++) dd[i]=(unsigned char)c; return d; }
size_t strlen(const char *s) { size_t n=0; while(s&&s[n]) n++; return n; }
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
void lpp_print_str(const char *t) { if(!t)return; int n=lpp_strlen(t); lpp_write(t,(DWORD)n); lpp_write("\n",1); }

double fmod(double x, double y) {
    if (y == 0.0) return 0.0;
    int64_t i = (int64_t)(x / y);
    return x - (double)i * y;
}

void *lpp_arc_alloc_with_destructor(int64_t sz, LppArcDestructor dtor) { if(sz<0)return 0; uint64_t t=lpp_page_round((uint64_t)sz+sizeof(LppArcHeader)); LppArcHeader *h=(LppArcHeader*)VirtualAlloc(0,t,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE); if(!h)return 0; h->refcount=1;h->destructor=dtor;h->allocation_size=t; return h+1; }
void *lpp_arc_alloc(int64_t sz) { return lpp_arc_alloc_with_destructor(sz,0); }
void lpp_arc_retain(void *p) { if(p)_InterlockedIncrement(&((LppArcHeader*)p-1)->refcount); }
void lpp_arc_release(void *p) { if(!p)return; LppArcHeader *h=(LppArcHeader*)p-1; if(_InterlockedDecrement(&h->refcount)==0){if(h->destructor)h->destructor(p);VirtualFree(h,0,MEM_RELEASE);} }
void *lpp_alloc(int64_t sz){return lpp_arc_alloc(sz);}
void lpp_free(void *p,int64_t sz){(void)sz;lpp_arc_release(p);}
void lpp_closure_destroy(void *c){if(c)lpp_arc_release(((void**)c)[1]);}

static void lpp_list_destroy(void *p) { LppList *l=(LppList*)p; if(!l)return; if(l->arc_elements){int64_t i;for(i=0;i<l->len;i++)lpp_arc_release((void*)(intptr_t)l->data[i]);} if(l->data)VirtualFree(l->data,0,MEM_RELEASE); }
static void *lpp_list_new_with_mode(int ae) { LppList *l=(LppList*)lpp_arc_alloc_with_destructor((int64_t)sizeof(LppList),lpp_list_destroy); if(!l)return 0; l->arc_elements=ae; return l; }
void *lpp_list_new(void){return lpp_list_new_with_mode(0);}
void *lpp_list_new_arc(void){return lpp_list_new_with_mode(1);}
void lpp_list_push(void *r,int64_t v){LppList*l=(LppList*)r;if(!l)return;if(l->len==l->cap){int64_t nc=l->cap==0?8:l->cap*2;if(nc<l->cap||nc>(int64_t)(0x7fffffffffffffffLL/8))return;uint64_t nb=lpp_page_round((uint64_t)nc*sizeof(int64_t));int64_t*nd=(int64_t*)VirtualAlloc(0,nb,MEM_COMMIT|MEM_RESERVE,PAGE_READWRITE);if(!nd)return;int64_t i;for(i=0;i<l->len;i++)nd[i]=l->data[i];if(l->data)VirtualFree(l->data,0,MEM_RELEASE);l->data=nd;l->cap=nc;l->data_bytes=nb;} if(l->arc_elements)lpp_arc_retain((void*)(intptr_t)v);l->data[l->len++]=v;}
void lpp_list_push_arc(void*l,void*v){lpp_list_push(l,(int64_t)(intptr_t)v);}
void lpp_list_push_float(void*l,double v){int64_t i;lpp_memcpy((char*)&i,(const char*)&v,8);lpp_list_push(l,i);}
int64_t lpp_list_get(void*r,int64_t i){LppList*l=(LppList*)r;return(!l||i<0||i>=l->len)?0:l->data[i];}
double lpp_list_get_float(void*l,int64_t idx){int64_t i=lpp_list_get(l,idx);double f;lpp_memcpy((char*)&f,(const char*)&i,8);return f;}
void *lpp_list_get_arc(void*l,int64_t i){return(void*)(intptr_t)lpp_list_get(l,i);}
int64_t lpp_list_len(void*r){return r?((LppList*)r)->len:0;}
void lpp_list_free(void*l){lpp_arc_release(l);}

void lpp_thread_spawn(void*fn,void*env){HANDLE h=CreateThread(0,0,(DWORD(__stdcall*)(void*))fn,env,0,0);if(h){WaitForSingleObject(h,INFINITE);CloseHandle(h);}}

/* ═══ STRING ═══════════════════════════════════════════════════════════════ */
char *lpp_str_concat(const char *a, const char *b) { if(!a)a="";if(!b)b=""; int la=lpp_strlen(a),lb=lpp_strlen(b); char*o=(char*)lpp_arc_alloc(la+lb+1); if(!o)return(char*)""; lpp_memcpy(o,a,la);lpp_memcpy(o+la,b,lb);o[la+lb]=0; return o; }
void *lpp_str_split(const char *s,int64_t d) { void*l=lpp_list_new_arc();if(!l)return 0;if(!s||!*s)return l; char ch=(char)d;const char*st=s; for(;;){if(*s==ch||*s==0){int64_t ln=(int64_t)(s-st);char*pc=(char*)lpp_arc_alloc(ln+1);if(pc){lpp_memcpy(pc,st,(int)ln);pc[ln]=0;lpp_list_push_arc(l,pc);lpp_arc_release(pc);}if(*s==0)break;st=s+1;}s++;} return l; }
int64_t lpp_str_find(const char *h,const char *n){if(!h||!n)return-1;const char*f=lpp_strstr(h,n); return f?(int64_t)(f-h):-1;}
char *lpp_str_replace(const char *s,const char *o,const char *nw){if(!s)s="";if(!o||!*o)return(char*)s;if(!nw)nw="";int sl=lpp_strlen(s),ol=lpp_strlen(o),nl=lpp_strlen(nw);int64_t c=0;const char*sc=s;while((sc=lpp_strstr(sc,o))){c++;sc+=ol;}int ol2=sl+(int)c*(nl-ol)+1;char*ou=(char*)lpp_arc_alloc(ol2);if(!ou)return(char*)"";char*d=ou;const char*sr=s;while(*sr){const char*nx=lpp_strstr(sr,o);if(!nx){lpp_strcpy(d,sr);break;}int pfx=(int)(nx-sr);lpp_memcpy(d,sr,pfx);d+=pfx;lpp_memcpy(d,nw,nl);d+=nl;sr=nx+ol;}return ou;}
char *lpp_str_substr(const char *s,int64_t st,int64_t ln){if(!s)s="";int sl=lpp_strlen(s);if(st<0)st=0;if(st>(int64_t)sl)return(char*)"";int rm=sl-(int)st;int cp=(ln<0||(size_t)ln>(size_t)rm)?rm:(int)ln;char*o=(char*)lpp_arc_alloc(cp+1);if(!o)return(char*)"";lpp_memcpy(o,s+st,cp);o[cp]=0;return o;}
char *lpp_str_trim(const char *s){if(!s)return(char*)"";while(lpp_isspace(*s))s++;int ln=lpp_strlen(s);while(ln>0&&lpp_isspace(s[ln-1]))ln--;char*o=(char*)lpp_arc_alloc(ln+1);if(!o)return(char*)"";lpp_memcpy(o,s,ln);o[ln]=0;return o;}

/* ═══ EXEC ═════════════════════════════════════════════════════════════════ */
int64_t lpp_command_exec(const char *cmd) { if(!cmd||!*cmd)return-1; char *d=lpp_strdup(cmd); if(!d)return-1; REAL_STARTUPINFOA si; int i;for(i=0;i<(int)sizeof(si);i++)((char*)&si)[i]=0; *(DWORD*)&si=sizeof(si); *(DWORD*)((char*)&si+60)=STARTF_USESTDHANDLES; PROCESS_INFORMATION pi; BOOL ok=CreateProcessA(NULL,d,NULL,NULL,FALSE,0x08000000,NULL,NULL,&si,&pi); DWORD ec=1; if(ok){WaitForSingleObject(pi.hProcess,INFINITE);GetExitCodeProcess(pi.hProcess,&ec);CloseHandle(pi.hProcess);CloseHandle(pi.hThread);} if(d)VirtualFree(d,0,MEM_RELEASE); return ok?(int64_t)(int)ec:-1;}
char *lpp_command_output(const char *cmd){if(!cmd)return(char*)"";HANDLE r,w;if(!CreatePipe(&r,&w,NULL,0))return(char*)"";REAL_STARTUPINFOA si;int i;for(i=0;i<(int)sizeof(si);i++)((char*)&si)[i]=0;*(DWORD*)&si=sizeof(si);((HANDLE*)((char*)&si+64))[0]=w;((HANDLE*)((char*)&si+64))[1]=w;*(DWORD*)((char*)&si+60)=STARTF_USESTDHANDLES;char*d=lpp_strdup(cmd);PROCESS_INFORMATION pi;BOOL ok=CreateProcessA(NULL,d,NULL,NULL,TRUE,0x08000000,NULL,NULL,&si,&pi);if(d)VirtualFree(d,0,MEM_RELEASE);CloseHandle(w);if(!ok){CloseHandle(r);return(char*)"";}WaitForSingleObject(pi.hProcess,INFINITE);CloseHandle(pi.hProcess);CloseHandle(pi.hThread);int cap=4096,len=0;char*b=(char*)lpp_arc_alloc(cap+1);if(!b){CloseHandle(r);return(char*)"";}for(;;){if(len+1024>=cap){int nc=cap*2;char*nb=(char*)lpp_arc_alloc(nc+1);if(!nb)break;lpp_memcpy(nb,b,len);lpp_arc_release(b);b=nb;cap=nc;}DWORD n;if(!ReadFile(r,b+len,(DWORD)(cap-len),&n,NULL)||n==0)break;len+=(int)n;}CloseHandle(r);b[len]=0;return b;}
char *lpp_env_get(const char *n){if(!n)return(char*)"";char v[4096];DWORD x=GetEnvironmentVariableA(n,v,sizeof(v));if(x==0||x>=sizeof(v))return(char*)"";char*o=(char*)lpp_arc_alloc((int64_t)(x+1));if(!o)return(char*)"";lpp_memcpy(o,v,(int)x);o[x]=0;return o;}
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
char *lpp_path_join(const char *b,const char *c){if(!b)b="";if(!c)c="";int bl=lpp_strlen(b),cl=lpp_strlen(c);int ns=(bl>0&&b[bl-1]!='\\'&&b[bl-1]!='/');int64_t t=(int64_t)(bl+(ns?1:0)+cl+1);char*o=(char*)lpp_arc_alloc(t);if(!o)return(char*)"";lpp_memcpy(o,b,bl);int off=bl;if(ns)o[off++]='\\';lpp_memcpy(o+off,c,cl);o[off+cl]=0;return o;}

typedef struct LppMapEntry { int64_t key; int64_t val; int is_str_key; int occupied; } LppMapEntry;
typedef struct LppMap { LppMapEntry *entries; int64_t cap; int64_t len; } LppMap;
static uint64_t lpp_map_hash_str(const char *s) { if (!s) return 0; uint64_t hash = 14695981039346656037ULL; while (*s) { hash ^= (unsigned char)(*s++); hash *= 1099511628211ULL; } return hash; }
static uint64_t lpp_map_hash_int(int64_t key) { uint64_t k = (uint64_t)key; k = (~k) + (k << 21); k = k ^ (k >> 24); k = (k + (k << 3)) + (k << 8); k = k ^ (k >> 14); k = (k + (k << 2)) + (k << 4); k = k ^ (k >> 28); k = k + (k << 31); return k; }
static void lpp_map_destroy(void *p) { LppMap *m = (LppMap *)p; if (!m) return; if (m->entries) VirtualFree(m->entries, 0, MEM_RELEASE); m->entries = 0; m->cap = 0; m->len = 0; }
void *lpp_map_new(void) { LppMap *m = (LppMap *)lpp_arc_alloc_with_destructor((int64_t)sizeof(LppMap), lpp_map_destroy); if (!m) return 0; m->cap = 16; m->len = 0; m->entries = (LppMapEntry *)VirtualAlloc(0, lpp_page_round((uint64_t)m->cap * sizeof(LppMapEntry)), MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE); return m; }
static void lpp_map_rehash(LppMap *m) { int64_t old_cap = m->cap; LppMapEntry *old_entries = m->entries; m->cap = old_cap * 2; m->entries = (LppMapEntry *)VirtualAlloc(0, lpp_page_round((uint64_t)m->cap * sizeof(LppMapEntry)), MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE); m->len = 0; for (int64_t i = 0; i < old_cap; i++) { if (old_entries[i].occupied == 1) { int64_t key = old_entries[i].key; int64_t val = old_entries[i].val; int is_str = old_entries[i].is_str_key; uint64_t h = is_str ? lpp_map_hash_str((const char *)(uintptr_t)key) : lpp_map_hash_int(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); while (m->entries[idx].occupied == 1) { idx = (idx + 1) % m->cap; } m->entries[idx].key = key; m->entries[idx].val = val; m->entries[idx].is_str_key = is_str; m->entries[idx].occupied = 1; m->len++; } } if (old_entries) VirtualFree(old_entries, 0, MEM_RELEASE); }
static void lpp_map_put_internal(LppMap *m, int64_t key, int64_t val, int is_str) { if (!m) return; if (m->len * 10 >= m->cap * 7) { lpp_map_rehash(m); } uint64_t h = is_str ? lpp_map_hash_str((const char *)(uintptr_t)key) : lpp_map_hash_int(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t first_tombstone = -1; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == is_str) { int match = is_str ? (lpp_strcmp((const char *)(uintptr_t)m->entries[idx].key, (const char *)(uintptr_t)key) == 0) : (m->entries[idx].key == key); if (match) { m->entries[idx].val = val; return; } } if (m->entries[idx].occupied == 2 && first_tombstone == -1) { first_tombstone = idx; } idx = (idx + 1) % m->cap; } if (first_tombstone != -1) { idx = first_tombstone; } m->entries[idx].key = key; m->entries[idx].val = val; m->entries[idx].is_str_key = is_str; m->entries[idx].occupied = 1; m->len++; }
void lpp_map_put(void *map, int64_t key, int64_t val) { lpp_map_put_internal((LppMap *)map, key, val, 0); }
void lpp_map_put_str(void *map, const char *key, int64_t val) { lpp_map_put_internal((LppMap *)map, (int64_t)(uintptr_t)key, val, 1); }
int64_t lpp_map_get(void *map, int64_t key) { LppMap *m = (LppMap *)map; if (!m || m->len == 0) return 0; uint64_t h = lpp_map_hash_int(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t start_idx = idx; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) { return m->entries[idx].val; } idx = (idx + 1) % m->cap; if (idx == start_idx) break; } return 0; }
int64_t lpp_map_get_str(void *map, const char *key) { LppMap *m = (LppMap *)map; if (!m || !key || m->len == 0) return 0; uint64_t h = lpp_map_hash_str(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t start_idx = idx; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) { if (lpp_strcmp((const char *)(uintptr_t)m->entries[idx].key, key) == 0) { return m->entries[idx].val; } } idx = (idx + 1) % m->cap; if (idx == start_idx) break; } return 0; }
int64_t lpp_map_has(void *map, int64_t key) { LppMap *m = (LppMap *)map; if (!m || m->len == 0) return 0; uint64_t h = lpp_map_hash_int(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t start_idx = idx; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) { return 1; } idx = (idx + 1) % m->cap; if (idx == start_idx) break; } return 0; }
int64_t lpp_map_has_str(void *map, const char *key) { LppMap *m = (LppMap *)map; if (!m || !key || m->len == 0) return 0; uint64_t h = lpp_map_hash_str(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t start_idx = idx; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) { if (lpp_strcmp((const char *)(uintptr_t)m->entries[idx].key, key) == 0) { return 1; } } idx = (idx + 1) % m->cap; if (idx == start_idx) break; } return 0; }
int64_t lpp_map_len(void *map) { LppMap *m = (LppMap *)map; return m ? m->len : 0; }
void lpp_map_remove(void *map, int64_t key) { LppMap *m = (LppMap *)map; if (!m || m->len == 0) return; uint64_t h = lpp_map_hash_int(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t start_idx = idx; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) { m->entries[idx].occupied = 2; m->len--; return; } idx = (idx + 1) % m->cap; if (idx == start_idx) break; } }
void lpp_map_remove_str(void *map, const char *key) { LppMap *m = (LppMap *)map; if (!m || !key || m->len == 0) return; uint64_t h = lpp_map_hash_str(key); int64_t idx = (int64_t)(h % (uint64_t)m->cap); int64_t start_idx = idx; while (m->entries[idx].occupied != 0) { if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) { if (lpp_strcmp((const char *)(uintptr_t)m->entries[idx].key, key) == 0) { m->entries[idx].occupied = 2; m->len--; return; } } idx = (idx + 1) % m->cap; if (idx == start_idx) break; } }
void lpp_map_put_float(void *map, int64_t key, double val) { int64_t ival; lpp_memcpy((char*)&ival, (const char*)&val, 8); lpp_map_put(map, key, ival); }
double lpp_map_get_float(void *map, int64_t key) { int64_t ival = lpp_map_get(map, key); double fval; lpp_memcpy((char*)&fval, (const char*)&ival, 8); return fval; }
void lpp_map_put_str_float(void *map, const char *key, double val) { int64_t ival; lpp_memcpy((char*)&ival, (const char*)&val, 8); lpp_map_put_str(map, key, ival); }
double lpp_map_get_str_float(void *map, const char *key) { int64_t ival = lpp_map_get_str(map, key); double fval; lpp_memcpy((char*)&fval, (const char*)&ival, 8); return fval; }
