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

/* ── String equality & comparison ── */
int64_t lpp_str_eq(const char *a, const char *b) { if(a==b)return 1; if(!a||!b)return 0; while(*a&&*a==*b){a++;b++;} return *a==*b?1:0; }
int64_t lpp_str_cmp(const char *a, const char *b) { if(a==b)return 0; if(!a)return -1; if(!b)return 1; while(*a&&*a==*b){a++;b++;} return (int64_t)((unsigned char)*a - (unsigned char)*b); }

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

