/* ── Math builtins ── */
int64_t lpp_abs(int64_t x) { return x < 0 ? -x : x; }
int64_t lpp_min(int64_t a, int64_t b) { return a < b ? a : b; }
int64_t lpp_max(int64_t a, int64_t b) { return a > b ? a : b; }
int64_t lpp_int_pow(int64_t base, int64_t exp) { int64_t r=1; while(exp>0){if(exp&1)r*=base;base*=base;exp>>=1;} return r; }
double lpp_int_to_float(int64_t x) { return (double)x; }
int64_t lpp_float_to_int(double x) { return (int64_t)x; }
double lpp_sqrt(double x) { if(x<=0)return 0; double g=x; for(int i=0;i<50;i++)g=0.5*(g+x/g); return g; }
double lpp_floor(double x) { int64_t i=(int64_t)x; return (double)(x<(double)i?i-1:i); }
double lpp_ceil(double x) { int64_t i=(int64_t)x; return (double)(x>(double)i?i+1:i); }
double lpp_pow(double b,double e) { int64_t ie=(int64_t)e; if((double)ie==e&&ie>=0){double r=1;while(ie>0){if(ie&1)r*=b;b*=b;ie>>=1;}return r;} return 0; }

/* ── Random (stubs — no writable .bss in static freestanding) ── */
void lpp_random_seed(int64_t seed) { (void)seed; }
int64_t lpp_random(void) { return 42; }
int64_t lpp_random_range(int64_t lo, int64_t hi) { return lo < hi ? lo : 0; }

/* ── Time (syscalls) ── */
int64_t lpp_time_ms(void) { uint64_t buf[2]; __asm__ volatile("syscall":"=a"(buf[0]):"a"(228),"D"(1),"S"(buf):"rcx","r11","memory"); return (int64_t)(buf[0]*1000+buf[1]/1000000); }
void lpp_sleep_ms(int64_t ms) { uint64_t buf[2]; buf[0]=(uint64_t)(ms/1000); buf[1]=(uint64_t)((ms%1000)*1000000); __asm__ volatile("syscall"::"a"(35),"D"(buf),"S"(0):"rcx","r11","memory"); }
void lpp_exit(int64_t code) { __asm__ volatile("syscall"::"a"(60),"D"(code):"rcx","r11"); for(;;); }

/* ── String equality & comparison ── */
int64_t lpp_str_eq(const char *a, const char *b) { if(a==b)return 1; if(!a||!b)return 0; while(*a&&*a==*b){a++;b++;} return *a==*b?1:0; }
int64_t lpp_str_cmp(const char *a, const char *b) { if(a==b)return 0; if(!a)return -1; if(!b)return 1; while(*a&&*a==*b){a++;b++;} return (int64_t)((unsigned char)*a - (unsigned char)*b); }

/* ── Buffers ── */
int64_t lpp_buf_alloc(int64_t size) { if(size<=0)size=64; int64_t t=size+8; void*m=lpp_sys_mmap(lpp_page_round((uint64_t)t)); if(!m)return 0; *(int64_t*)m=size; return(int64_t)(uintptr_t)((char*)m+8); }
void lpp_buf_free(void*p) { if(!p)return; char*b=(char*)p-8; int64_t s=*(int64_t*)b; lpp_sys_munmap(b,lpp_page_round((uint64_t)(s+8))); }
/* The OS reclaims stale-context tombstones when this freestanding process exits. */
void lpp_buf_free_at_exit(int64_t p) { (void)p; }
int64_t lpp_buf_len(void*p) { if(!p)return 0; return*(int64_t*)((char*)p-8); }
int64_t lpp_buf_get8(void*p,int64_t o) { if(!p)return 0; return(int64_t)(unsigned char)((char*)p)[o]; }
void lpp_buf_set8(void*p,int64_t o,int64_t v) { if(!p)return; ((char*)p)[o]=(char)(v&0xFF); }
void lpp_buf_set16le(void*p,int64_t o,int64_t v) { if(!p)return; char*d=(char*)p+o; d[0]=(char)(v&0xFF); d[1]=(char)((v>>8)&0xFF); }
int64_t lpp_buf_get16le(void*p,int64_t o) { if(!p)return 0; unsigned char*d=(unsigned char*)((char*)p+o); return(int64_t)d[0]|((int64_t)d[1]<<8); }
void lpp_buf_set32le(void*p,int64_t o,int64_t v) { if(!p)return; char*d=(char*)p+o; d[0]=(char)(v&0xFF); d[1]=(char)((v>>8)&0xFF); d[2]=(char)((v>>16)&0xFF); d[3]=(char)((v>>24)&0xFF); }
int64_t lpp_buf_get32le(void*p,int64_t o) { if(!p)return 0; unsigned char*d=(unsigned char*)((char*)p+o); return(int64_t)d[0]|((int64_t)d[1]<<8)|((int64_t)d[2]<<16)|((int64_t)d[3]<<24); }
void lpp_buf_set64le(void*p,int64_t o,int64_t v) { if(!p)return; char*d=(char*)p+o; d[0]=(char)(v&0xFF); d[1]=(char)((v>>8)&0xFF); d[2]=(char)((v>>16)&0xFF); d[3]=(char)((v>>24)&0xFF); d[4]=(char)((v>>32)&0xFF); d[5]=(char)((v>>40)&0xFF); d[6]=(char)((v>>48)&0xFF); d[7]=(char)((v>>56)&0xFF); }
int64_t lpp_buf_get64le(void*p,int64_t o) { if(!p)return 0; unsigned char*d=(unsigned char*)((char*)p+o); return(int64_t)d[0]|((int64_t)d[1]<<8)|((int64_t)d[2]<<16)|((int64_t)d[3]<<24)|((int64_t)d[4]<<32)|((int64_t)d[5]<<40)|((int64_t)d[6]<<48)|((int64_t)d[7]<<56); }
void lpp_buf_set16be(void*p,int64_t o,int64_t v) { if(!p)return; char*d=(char*)p+o; d[0]=(char)((v>>8)&0xFF); d[1]=(char)(v&0xFF); }
int64_t lpp_buf_get16be(void*p,int64_t o) { if(!p)return 0; unsigned char*d=(unsigned char*)((char*)p+o); return((int64_t)d[0]<<8)|(int64_t)d[1]; }
void lpp_buf_set32be(void*p,int64_t o,int64_t v) { if(!p)return; char*d=(char*)p+o; d[0]=(char)((v>>24)&0xFF); d[1]=(char)((v>>16)&0xFF); d[2]=(char)((v>>8)&0xFF); d[3]=(char)(v&0xFF); }
int64_t lpp_buf_get32be(void*p,int64_t o) { if(!p)return 0; unsigned char*d=(unsigned char*)((char*)p+o); return((int64_t)d[0]<<24)|((int64_t)d[1]<<16)|((int64_t)d[2]<<8)|(int64_t)d[3]; }
void lpp_buf_set64be(void*p,int64_t o,int64_t v) { if(!p)return; char*d=(char*)p+o; d[0]=(char)((v>>56)&0xFF); d[1]=(char)((v>>48)&0xFF); d[2]=(char)((v>>40)&0xFF); d[3]=(char)((v>>32)&0xFF); d[4]=(char)((v>>24)&0xFF); d[5]=(char)((v>>16)&0xFF); d[6]=(char)((v>>8)&0xFF); d[7]=(char)(v&0xFF); }
int64_t lpp_buf_get64be(void*p,int64_t o) { if(!p)return 0; unsigned char*d=(unsigned char*)((char*)p+o); return((int64_t)d[0]<<56)|((int64_t)d[1]<<48)|((int64_t)d[2]<<40)|((int64_t)d[3]<<32)|((int64_t)d[4]<<24)|((int64_t)d[5]<<16)|((int64_t)d[6]<<8)|(int64_t)d[7]; }
void lpp_buf_copy(void*dst,int64_t do2,void*src,int64_t so,int64_t len) { if(!dst||!src||len<=0)return; char*d=(char*)dst+do2; char*s=(char*)src+so; for(int64_t i=0;i<len;i++)d[i]=s[i]; }
char*lpp_buf_read_str(void*p,int64_t o,int64_t len) { if(!p||len<=0){char*e=(char*)lpp_alloc(1);e[0]=0;return e;} char*out=(char*)lpp_alloc(len+1); char*s=(char*)p+o; for(int64_t i=0;i<len;i++)out[i]=s[i]; out[len]=0; return out; }

/* Mirror runtime/lpp_buf.c semantics: 0 on success, -1 on error. */
void *lpp_buf_read(const char*path) { if(!path)return 0; long fd=lpp_sys_open(path,0,0); if(fd<0)return 0; long size=lpp_sys_lseek(fd,0,2); (void)lpp_sys_lseek(fd,0,0); if(size<0){lpp_sys_close(fd);return 0;} void*buf=(void*)(uintptr_t)lpp_buf_alloc(size); if(!buf){lpp_sys_close(fd);return 0;} long off=0; while(off<size){ long r=lpp_sys_read(fd,(char*)buf+off,size-off); if(r<=0)break; off+=r; } lpp_sys_close(fd); if(off!=size){lpp_buf_free(buf);return 0;} return buf; }
int64_t lpp_buf_write(const char*path,void*p) { if(!path||!p)return -1; int64_t size=*(int64_t*)((char*)p-8); long fd=lpp_sys_open(path,0101,0644); if(fd<0)return -1; int64_t off=0; while(off<size){ long w=lpp_sys_write(fd,(char*)p+off,size-off); if(w<=0){lpp_sys_close(fd);return -1;} off+=w; } lpp_sys_close(fd); return 0; }
int64_t lpp_buf_write_str(void*p,int64_t o,const char*str) { if(!p||!str)return -1; int64_t n=0; while(str[n])n++; int64_t cap=lpp_buf_len(p); if(o<0||o+n>cap)return -1; char*d=(char*)p+o; for(int64_t i=0;i<n;i++)d[i]=str[i]; return n; }
int64_t lpp_buf_crc32(void*p,int64_t off,int64_t len) { if(!p||len<0)return 0; int64_t cap=lpp_buf_len(p); if(off<0||off+len>cap)return 0; const unsigned char*d=(const unsigned char*)((char*)p+off); uint32_t crc=0xFFFFFFFFu; for(int64_t i=0;i<len;i++){ crc^=d[i]; for(int b=0;b<8;b++){ uint32_t m=~(crc&1u)+1u; crc=(crc>>1)^(0xEDB88320u&m); } } return (int64_t)(uint32_t)(~crc); }

/* ── Additional net stubs ── */
int64_t lpp_net_accept_timeout(int64_t l,int64_t t){(void)t;return lpp_net_accept(l);}
int64_t lpp_net_dial(const char*h,int64_t p,int64_t t){(void)t;return lpp_net_connect(h,p);}
int64_t lpp_net_dial_udp(const char*h,int64_t p,int64_t t){(void)h;(void)p;(void)t;return 0;}
int64_t lpp_net_listen_udp(int64_t p){(void)p;return 0;}
int64_t lpp_net_set_deadline(int64_t f,int64_t r,int64_t w){return lpp_net_set_timeout(f,r>w?r:w);}
int64_t lpp_net_set_keepalive(int64_t f,int64_t e,int64_t i,int64_t v,int64_t c){(void)f;(void)e;(void)i;(void)v;(void)c;return 1;}

/* Runtime AVX2 dispatch for the freestanding x86-64 profile. Check CPU
 * support, OSXSAVE, and XCR0 explicitly: unlike the hosted runtime, this
 * image cannot rely on libc's __builtin_cpu_supports constructor. */
#if defined(__GNUC__) && defined(__x86_64__)
static void lpp_min_cpuid(uint32_t leaf, uint32_t subleaf,
                          uint32_t *a, uint32_t *b, uint32_t *c, uint32_t *d) {
    uint32_t eax = leaf, ebx, ecx = subleaf, edx;
    __asm__ volatile("cpuid" : "+a"(eax), "=b"(ebx), "+c"(ecx), "=d"(edx));
    *a = eax; *b = ebx; *c = ecx; *d = edx;
}

static int lpp_min_cpu_has_avx2(void) {
    uint32_t max_leaf, b, c, d;
    lpp_min_cpuid(0, 0, &max_leaf, &b, &c, &d);
    if (max_leaf < 7) return 0;
    uint32_t a;
    lpp_min_cpuid(1, 0, &a, &b, &c, &d);
    if ((c & (1u << 27)) == 0 || (c & (1u << 28)) == 0) return 0;
    uint32_t xcr0_low, xcr0_high;
    __asm__ volatile("xgetbv" : "=a"(xcr0_low), "=d"(xcr0_high) : "c"(0));
    (void)xcr0_high;
    if ((xcr0_low & 0x6) != 0x6) return 0;
    lpp_min_cpuid(7, 0, &a, &b, &c, &d);
    return (b & (1u << 5)) != 0;
}

typedef int64_t lpp_i64x4 __attribute__((vector_size(32)));
__attribute__((target("avx2")))
static int64_t lpp_vec_i64_checksum_avx2(int64_t n) {
    int64_t i = 0;
    const lpp_i64x4 three = {3, 3, 3, 3};
    lpp_i64x4 total = {0, 0, 0, 0};
    while (n - i >= 4) {
        lpp_i64x4 v = {i, i + 1, i + 2, i + 3};
        total += (v * three) ^ (v >> 1);
        i += 4;
    }
    int64_t sum = total[0] + total[1] + total[2] + total[3];
    for (; i < n; ++i) sum += (i * 3) ^ (i >> 1);
    return sum;
}
#endif
int64_t lpp_vec_i64_checksum(int64_t n) {
    if (n < 0) return 0;
#if defined(__GNUC__) && defined(__x86_64__)
    if (n >= 64 && lpp_min_cpu_has_avx2()) return lpp_vec_i64_checksum_avx2(n);
#endif
    int64_t total = 0;
    for (int64_t i = 0; i < n; ++i) total += (i * 3) ^ (i >> 1);
    return total;
}

