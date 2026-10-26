/* ═══ STRING ═══════════════════════════════════════════════════════════════ */
char *lpp_str_concat(const char *a, const char *b) {
    if(!a) a="";
    if(!b) b="";
    int la = lpp_strlen(a);
    int lb = lpp_strlen(b);
    char *o = (char*)lpp_arc_alloc(la + lb + 1);
    if(!o) return lpp_empty_str();
    lpp_memcpy(o, a, la);
    lpp_memcpy(o + la, b, lb);
    o[la + lb] = 0;
    return o;
}
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
void *lpp_map_keys(void *map) { LppMap *m = (LppMap *)map; if (!m || m->len == 0) return lpp_list_new(); int has_str = 0; for (int64_t i = 0; i < m->cap; i++) { if (m->entries[i].occupied == 1 && m->entries[i].is_str_key) { has_str = 1; break; } } void *lst = has_str ? lpp_list_new_arc() : lpp_list_new(); for (int64_t i = 0; i < m->cap; i++) { if (m->entries[i].occupied == 1) { if (m->entries[i].is_str_key) { const char *s = (const char *)(uintptr_t)m->entries[i].key; int len = lpp_strlen(s); char *cp = (char *)lpp_arc_alloc(len + 1); if (cp) { lpp_memcpy(cp, s, len); cp[len] = 0; } lpp_list_push_arc(lst, cp); } else { lpp_list_push(lst, m->entries[i].key); } } } return lst; }
void *lpp_map_values(void *map) { LppMap *m = (LppMap *)map; if (!m || m->len == 0) return lpp_list_new(); void *lst = m->arc_values ? lpp_list_new_arc() : lpp_list_new(); for (int64_t i = 0; i < m->cap; i++) { if (m->entries[i].occupied == 1) { if (m->arc_values) { lpp_list_push_arc(lst, (void *)(uintptr_t)m->entries[i].val); } else { lpp_list_push(lst, m->entries[i].val); } } } return lst; }
void lpp_map_clear(void *map) { LppMap *m = (LppMap *)map; if (!m || !m->entries) return; if (m->arc_values) { for (int64_t i = 0; i < m->cap; i++) { if (m->entries[i].occupied == 1) lpp_arc_release((void *)(uintptr_t)m->entries[i].val); } } lpp_memset((void*)m->entries, 0, sizeof(LppMapEntry) * (size_t)m->cap); m->len = 0; }
int64_t lpp_map_capacity(void *map) { LppMap *m = (LppMap *)map; return m ? m->cap : 0; }



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

