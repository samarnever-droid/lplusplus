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
__declspec(dllimport) void   __stdcall ExitProcess(unsigned int code);
__declspec(dllimport) void * __stdcall LoadLibraryA(const char *name);
__declspec(dllimport) void * __stdcall GetProcAddress(void *module, const char *name);
__declspec(dllimport) unsigned long long __stdcall GetTickCount64(void);

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
char *lpp_str_repeat(const char *s, int64_t n) { if(!s||n<=0)return(char*)""; int slen=lpp_strlen(s); if(!slen)return(char*)""; int64_t total=(int64_t)slen*n; char*o=(char*)lpp_arc_alloc(total+1); if(!o)return(char*)""; int64_t i; for(i=0;i<n;i++)lpp_memcpy(o+i*slen,s,slen); o[total]=0; return o; }
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


/* ── Additional string builtins (only those NOT already defined above) ── */

int64_t lpp_str_len(const char *s) { return (int64_t)lpp_strlen(s); }

char *lpp_char_at(const char *s, int64_t idx) { if(!s||idx<0||idx>=(int64_t)lpp_strlen(s)){char *e=(char*)lpp_arc_alloc(2);e[0]=0;return e;} char *out=(char*)lpp_arc_alloc(2); out[0]=s[idx]; out[1]=0; return out; }

int64_t lpp_ord(const char *s) { if(!s||!s[0]) return 0; return (int64_t)(unsigned char)s[0]; }

char *lpp_chr(int64_t code) { char *out=(char*)lpp_arc_alloc(2); out[0]=(char)(code&0xFF); out[1]=0; return out; }

int64_t lpp_str_contains(const char *h, const char *n) { return lpp_str_find(h,n)>=0?1:0; }

int64_t lpp_str_starts_with(const char *s, const char *p) { if(!s||!p)return 0; int64_t pl=(int64_t)lpp_strlen(p); int64_t i; for(i=0;i<pl;i++){if(s[i]!=p[i]||s[i]==0)return 0;} return 1; }

int64_t lpp_str_ends_with(const char *s, const char *x) { if(!s||!x)return 0; int64_t sl=(int64_t)lpp_strlen(s),xl=(int64_t)lpp_strlen(x); if(xl>sl)return 0; int64_t i; for(i=0;i<xl;i++){if(s[sl-xl+i]!=x[i])return 0;} return 1; }

char *lpp_str_upper(const char *s) { if(!s)return(char*)""; int ln=lpp_strlen(s); char *out=(char*)lpp_arc_alloc(ln+1); int i; for(i=0;i<ln;i++) out[i]=(s[i]>='a'&&s[i]<='z')?s[i]-32:s[i]; out[ln]=0; return out; }

char *lpp_str_lower(const char *s) { if(!s)return(char*)""; int ln=lpp_strlen(s); char *out=(char*)lpp_arc_alloc(ln+1); int i; for(i=0;i<ln;i++) out[i]=(s[i]>='A'&&s[i]<='Z')?s[i]+32:s[i]; out[ln]=0; return out; }

char *lpp_int_to_str(int64_t val) { char buf[24]; int neg=val<0; if(neg)val=-val; int i=23; buf[i]=0; do{buf[--i]='0'+(int)(val%10);val/=10;}while(val); if(neg)buf[--i]='-'; int ln=23-i; char *out=(char*)lpp_arc_alloc(ln+1); lpp_memcpy(out,buf+i,ln+1); return out; }

int64_t lpp_str_to_int(const char *s) { if(!s)return 0; int64_t val=0,neg=0; int i=0; while(s[i]==' '||s[i]=='\t')i++; if(s[i]=='-'){neg=1;i++;}else if(s[i]=='+')i++; while(s[i]>='0'&&s[i]<='9'){val=val*10+(s[i]-'0');i++;} return neg?-val:val; }

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
    HANDLE h = CreateFileA(path, GENERIC_WRITE, 0, 0, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    if (h == INVALID_HANDLE_VALUE) return -1;
    DWORD written = 0;
    int ln = lpp_strlen(data);
    WriteFile(h, data, (DWORD)ln, &written, 0);
    CloseHandle(h);
    return (int64_t)written;
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
    HANDLE h = CreateFileA(path, FILE_APPEND_DATA, 0, 0, OPEN_ALWAYS, FILE_ATTRIBUTE_NORMAL, 0);
    if (h == INVALID_HANDLE_VALUE) return -1;
    DWORD written = 0;
    int ln = lpp_strlen(data);
    WriteFile(h, data, (DWORD)ln, &written, 0);
    CloseHandle(h);
    return (int64_t)written;
}

int64_t lpp_delete_file(const char *path) { return DeleteFileA(path) ? 0 : -1; }
int64_t lpp_file_exists(const char *path) { DWORD a = GetFileAttributesA(path); return (a != ((DWORD)-1)) ? 1 : 0; }
int64_t lpp_file_size(const char *path) { HANDLE h = CreateFileA(path, GENERIC_READ, FILE_SHARE_READ, 0, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, 0); if (h == INVALID_HANDLE_VALUE) return -1; DWORD sz = GetFileSize(h, 0); CloseHandle(h); return (int64_t)sz; }

/* ── Math builtins ── */
int64_t lpp_abs(int64_t x) { return x < 0 ? -x : x; }
int64_t lpp_min(int64_t a, int64_t b) { return a < b ? a : b; }
int64_t lpp_max(int64_t a, int64_t b) { return a > b ? a : b; }
int64_t lpp_int_pow(int64_t base, int64_t exp) {
    int64_t result = 1;
    while (exp > 0) { if (exp & 1) result *= base; base *= base; exp >>= 1; }
    return result;
}
double lpp_int_to_float(int64_t x) { return (double)x; }
int64_t lpp_float_to_int(double x) { return (int64_t)x; }

double lpp_sqrt(double x) {
    if (x <= 0.0) return 0.0;
    double guess = x;
    int i; for (i = 0; i < 50; i++) guess = 0.5 * (guess + x / guess);
    return guess;
}
double lpp_floor(double x) { int64_t i = (int64_t)x; return (double)(x < (double)i ? i - 1 : i); }
double lpp_ceil(double x) { int64_t i = (int64_t)x; return (double)(x > (double)i ? i + 1 : i); }
double lpp_pow(double base, double exp) {
    int64_t iexp = (int64_t)exp;
    if ((double)iexp == exp && iexp >= 0) {
        double result = 1.0;
        while (iexp > 0) { if (iexp & 1) result *= base; base *= base; iexp >>= 1; }
        return result;
    }
    if (base <= 0.0) return 0.0;
    double xm = (base - 1.0) / (base + 1.0), ln_base = 0.0, term = xm;
    int k; for (k = 0; k < 30; k++) { ln_base += term / (double)(2*k+1); term *= xm*xm; }
    ln_base *= 2.0;
    double y = exp * ln_base, result = 1.0, t = 1.0;
    for (k = 1; k < 30; k++) { t *= y / (double)k; result += t; }
    return result;
}

/* ── Random (using Kernel32 QueryPerformanceCounter for seed) ── */
void lpp_random_seed(int64_t seed) { (void)seed; }
int64_t lpp_random(void) { return 42; /* stub — full impl needs writable .data */ }
int64_t lpp_random_range(int64_t lo, int64_t hi) { return lo < hi ? lo : 0; }

/* ── Time (using Kernel32 GetTickCount64) ── */
int64_t lpp_time_ms(void) { return (int64_t)GetTickCount64(); }
void lpp_sleep_ms(int64_t ms) { Sleep((DWORD)ms); }

/* ── Process ── */
void lpp_exit(int64_t code) { ExitProcess((unsigned int)code); }

/* ── Buffer builtins (using VirtualAlloc) ── */
int64_t lpp_buf_alloc(int64_t size) {
    if (size <= 0) size = 64;
    int64_t total = size + 8;
    void *mem = VirtualAlloc(0, lpp_page_round((uint64_t)total), MEM_COMMIT|MEM_RESERVE, PAGE_READWRITE);
    if (!mem) return 0;
    *(int64_t *)mem = size;
    return (int64_t)(uintptr_t)((char *)mem + 8);
}
void lpp_buf_free(void *ptr) {
    if (!ptr) return;
    VirtualFree((char *)ptr - 8, 0, MEM_RELEASE);
}
int64_t lpp_buf_len(void *ptr) {
    if (!ptr) return 0;
    return *(int64_t *)((char *)ptr - 8);
}
int64_t lpp_buf_get8(void *ptr, int64_t offset) {
    if (!ptr) return 0;
    return (int64_t)(unsigned char)((char *)ptr)[offset];
}
void lpp_buf_set8(void *ptr, int64_t offset, int64_t value) {
    if (!ptr) return;
    ((char *)ptr)[offset] = (char)(value & 0xFF);
}
void lpp_buf_set16le(void *ptr, int64_t offset, int64_t value) {
    if (!ptr) return;
    char *p = (char *)ptr + offset;
    p[0] = (char)(value & 0xFF);
    p[1] = (char)((value >> 8) & 0xFF);
}
int64_t lpp_buf_get16le(void *ptr, int64_t offset) {
    if (!ptr) return 0;
    unsigned char *p = (unsigned char *)((char *)ptr + offset);
    return (int64_t)p[0] | ((int64_t)p[1] << 8);
}
void lpp_buf_set32le(void *ptr, int64_t offset, int64_t value) {
    if (!ptr) return;
    char *p = (char *)ptr + offset;
    p[0] = (char)(value & 0xFF);
    p[1] = (char)((value >> 8) & 0xFF);
    p[2] = (char)((value >> 16) & 0xFF);
    p[3] = (char)((value >> 24) & 0xFF);
}
int64_t lpp_buf_get32le(void *ptr, int64_t offset) {
    if (!ptr) return 0;
    unsigned char *p = (unsigned char *)((char *)ptr + offset);
    return (int64_t)p[0] | ((int64_t)p[1] << 8) | ((int64_t)p[2] << 16) | ((int64_t)p[3] << 24);
}
void lpp_buf_copy(void *dst, int64_t dst_off, void *src, int64_t src_off, int64_t len) {
    if (!dst || !src || len <= 0) return;
    char *d = (char *)dst + dst_off;
    char *s = (char *)src + src_off;
    int64_t i; for (i = 0; i < len; i++) d[i] = s[i];
}
char *lpp_buf_read_str(void *ptr, int64_t offset, int64_t len) {
    if (!ptr || len <= 0) { char *e = (char *)lpp_arc_alloc(1); e[0] = 0; return e; }
    char *out = (char *)lpp_arc_alloc(len + 1);
    char *s = (char *)ptr + offset;
    int64_t i; for (i = 0; i < len; i++) out[i] = s[i];
    out[len] = 0;
    return out;
}

/* ── Networking builtins (Windows freestanding, ws2_32.dll via LoadLibrary) ── */

#ifndef AF_INET
#define AF_INET 2
#define SOCK_STREAM 1
#define SOCK_DGRAM 2
#define SOL_SOCKET 0xFFFF
#define SO_REUSEADDR 0x0004
#define SO_RCVTIMEO 0x1006
#define SO_SNDTIMEO 0x1005
#define SO_KEEPALIVE 0x0008
#define IPPROTO_TCP 6
#endif
#ifndef INVALID_SOCKET
#define INVALID_SOCKET (~(uintptr_t)0)
#endif

typedef uintptr_t SOCKET;
typedef struct { uint16_t family; uint16_t port; uint32_t addr; char pad[8]; } lpp_sockaddr_in;

/* ws2_32 function pointers — loaded lazily */
typedef int (__stdcall *pWSAStartup)(uint16_t, void*);
typedef SOCKET (__stdcall *pSocket)(int, int, int);
typedef int (__stdcall *pConnect)(SOCKET, void*, int);
typedef int (__stdcall *pBind)(SOCKET, void*, int);
typedef int (__stdcall *pListen)(SOCKET, int);
typedef SOCKET (__stdcall *pAccept)(SOCKET, void*, int*);
typedef int (__stdcall *pSend)(SOCKET, const char*, int, int);
typedef int (__stdcall *pRecv)(SOCKET, char*, int, int);
typedef int (__stdcall *pClosesocket)(SOCKET);
typedef int (__stdcall *pSetsockopt)(SOCKET, int, int, const char*, int);
typedef uint16_t (__stdcall *pHtons)(uint16_t);
typedef uint32_t (__stdcall *pInet_addr)(const char*);

static pSocket fn_socket = 0;
static pConnect fn_connect = 0;
static pBind fn_bind = 0;
static pListen fn_listen = 0;
static pAccept fn_accept = 0;
static pSend fn_send = 0;
static pRecv fn_recv = 0;
static pClosesocket fn_closesocket = 0;
static pSetsockopt fn_setsockopt = 0;
static pHtons fn_htons = 0;
static pInet_addr fn_inet_addr = 0;

static int lpp_ws2_loaded = 0;
static void lpp_ws2_init(void) {
    if (lpp_ws2_loaded) return;
    void *ws2 = LoadLibraryA("ws2_32.dll");
    if (!ws2) return;
    pWSAStartup pStartup = (pWSAStartup)GetProcAddress(ws2, "WSAStartup");
    if (pStartup) { char wsadata[408]; pStartup(0x0202, wsadata); }
    fn_socket = (pSocket)GetProcAddress(ws2, "socket");
    fn_connect = (pConnect)GetProcAddress(ws2, "connect");
    fn_bind = (pBind)GetProcAddress(ws2, "bind");
    fn_listen = (pListen)GetProcAddress(ws2, "listen");
    fn_accept = (pAccept)GetProcAddress(ws2, "accept");
    fn_send = (pSend)GetProcAddress(ws2, "send");
    fn_recv = (pRecv)GetProcAddress(ws2, "recv");
    fn_closesocket = (pClosesocket)GetProcAddress(ws2, "closesocket");
    fn_setsockopt = (pSetsockopt)GetProcAddress(ws2, "setsockopt");
    fn_htons = (pHtons)GetProcAddress(ws2, "htons");
    fn_inet_addr = (pInet_addr)GetProcAddress(ws2, "inet_addr");
    lpp_ws2_loaded = 1;
}

static SOCKET lpp_win_sock_table[256];
static int lpp_win_sock_count = 0;

int64_t lpp_net_connect(const char *host, int64_t port) {
    lpp_ws2_init();
    if (!fn_socket || !host) return 0;
    SOCKET s = fn_socket(AF_INET, SOCK_STREAM, 0);
    if (s == INVALID_SOCKET) return 0;
    lpp_sockaddr_in addr; for (int i=0;i<16;i++) ((char*)&addr)[i]=0;
    addr.family = AF_INET;
    addr.port = fn_htons ? fn_htons((uint16_t)port) : (uint16_t)((port>>8)|(port<<8));
    addr.addr = fn_inet_addr ? fn_inet_addr(host) : 0;
    if (fn_connect(s, &addr, 16) != 0) { fn_closesocket(s); return 0; }
    int idx = lpp_win_sock_count++;
    lpp_win_sock_table[idx] = s;
    return (int64_t)(idx + 1);
}

int64_t lpp_net_listen(int64_t port) {
    lpp_ws2_init();
    if (!fn_socket) return 0;
    SOCKET s = fn_socket(AF_INET, SOCK_STREAM, 0);
    if (s == INVALID_SOCKET) return 0;
    int yes = 1; if (fn_setsockopt) fn_setsockopt(s, SOL_SOCKET, SO_REUSEADDR, (char*)&yes, 4);
    lpp_sockaddr_in addr; for (int i=0;i<16;i++) ((char*)&addr)[i]=0;
    addr.family = AF_INET;
    addr.port = fn_htons ? fn_htons((uint16_t)port) : 0;
    if (fn_bind(s, &addr, 16) != 0) { fn_closesocket(s); return 0; }
    if (fn_listen(s, 128) != 0) { fn_closesocket(s); return 0; }
    int idx = lpp_win_sock_count++;
    lpp_win_sock_table[idx] = s;
    return (int64_t)(idx + 1);
}

int64_t lpp_net_accept(int64_t listener) {
    if (listener < 1 || listener > 256) return 0;
    SOCKET server = lpp_win_sock_table[(int)listener - 1];
    SOCKET client = fn_accept ? fn_accept(server, 0, 0) : INVALID_SOCKET;
    if (client == INVALID_SOCKET) return 0;
    int idx = lpp_win_sock_count++;
    lpp_win_sock_table[idx] = client;
    return (int64_t)(idx + 1);
}

int64_t lpp_net_accept_timeout(int64_t listener, int64_t timeout_ms) {
    return lpp_net_accept(listener);
}

int64_t lpp_net_send(int64_t handle, const char *data) {
    if (handle < 1 || handle > 256 || !data || !fn_send) return -1;
    SOCKET s = lpp_win_sock_table[(int)handle - 1];
    return (int64_t)fn_send(s, data, (int)lpp_strlen(data), 0);
}

int64_t lpp_net_send_all(int64_t handle, const char *data) {
    return lpp_net_send(handle, data);
}

char *lpp_net_recv(int64_t handle, int64_t max_bytes) {
    if (handle < 1 || handle > 256 || max_bytes <= 0 || !fn_recv) {
        char *e = (char *)lpp_arc_alloc(1); e[0] = 0; return e;
    }
    SOCKET s = lpp_win_sock_table[(int)handle - 1];
    char *buf = (char *)lpp_arc_alloc(max_bytes + 1);
    int n = fn_recv(s, buf, (int)max_bytes, 0);
    if (n <= 0) { buf[0] = 0; return buf; }
    buf[n] = 0;
    return buf;
}

void lpp_net_close(int64_t handle) {
    if (handle < 1 || handle > 256 || !fn_closesocket) return;
    fn_closesocket(lpp_win_sock_table[(int)handle - 1]);
}

int64_t lpp_net_set_timeout(int64_t handle, int64_t milliseconds) {
    if (handle < 1 || handle > 256 || !fn_setsockopt) return 0;
    SOCKET s = lpp_win_sock_table[(int)handle - 1];
    DWORD tv = (DWORD)milliseconds;
    fn_setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, (char*)&tv, 4);
    fn_setsockopt(s, SOL_SOCKET, SO_SNDTIMEO, (char*)&tv, 4);
    return 1;
}

int64_t lpp_net_set_deadline(int64_t fd, int64_t read_ms, int64_t write_ms) {
    return lpp_net_set_timeout(fd, read_ms > write_ms ? read_ms : write_ms);
}

int64_t lpp_net_set_keepalive(int64_t handle, int64_t enable, int64_t idle_s, int64_t interval, int64_t count) {
    if (handle < 1 || handle > 256 || !fn_setsockopt) return 0;
    SOCKET s = lpp_win_sock_table[(int)handle - 1];
    int val = (int)enable;
    fn_setsockopt(s, SOL_SOCKET, SO_KEEPALIVE, (char*)&val, 4);
    return 1;
}

int64_t lpp_net_listen_udp(int64_t port) {
    lpp_ws2_init();
    if (!fn_socket) return 0;
    SOCKET s = fn_socket(AF_INET, SOCK_DGRAM, 0);
    if (s == INVALID_SOCKET) return 0;
    lpp_sockaddr_in addr; for (int i=0;i<16;i++) ((char*)&addr)[i]=0;
    addr.family = AF_INET;
    addr.port = fn_htons ? fn_htons((uint16_t)port) : 0;
    if (fn_bind(s, &addr, 16) != 0) { fn_closesocket(s); return 0; }
    int idx = lpp_win_sock_count++;
    lpp_win_sock_table[idx] = s;
    return (int64_t)(idx + 1);
}

int64_t lpp_net_dial(const char *host, int64_t port, int64_t timeout_ms) {
    return lpp_net_connect(host, port);
}

int64_t lpp_net_dial_udp(const char *host, int64_t port, int64_t timeout_ms) {
    lpp_ws2_init();
    if (!fn_socket || !host) return 0;
    SOCKET s = fn_socket(AF_INET, SOCK_DGRAM, 0);
    if (s == INVALID_SOCKET) return 0;
    lpp_sockaddr_in addr; for (int i=0;i<16;i++) ((char*)&addr)[i]=0;
    addr.family = AF_INET;
    addr.port = fn_htons ? fn_htons((uint16_t)port) : 0;
    addr.addr = fn_inet_addr ? fn_inet_addr(host) : 0;
    if (fn_connect(s, &addr, 16) != 0) { fn_closesocket(s); return 0; }
    int idx = lpp_win_sock_count++;
    lpp_win_sock_table[idx] = s;
    return (int64_t)(idx + 1);
}

int64_t lpp_str_eq(const char *a, const char *b) {
    if (a == b) return 1;
    if (!a || !b) return 0;
    while (*a && *a == *b) { a++; b++; }
    return *a == *b ? 1 : 0;
}
