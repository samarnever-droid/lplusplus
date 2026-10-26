/* Missing symbol stubs */
#ifndef _fltused
int _fltused = 0;
int __isa_available = 1;
#endif

/* `val` MUST stay int8_t — see the note in lpp_str.c. builtins.rs declares this
 * parameter with the I8 tag, so only the low byte of the argument register is
 * defined; reading int64_t let garbage upper bits turn `false` into "true". */
char *lpp_bool_to_str(int8_t val) {
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
    (void)source; (void)destination; return -1;
}

/* The compiler emits the complete builtin ABI even when a program does not
 * use dynamic loading.  These POSIX compatibility entry points keep the
 * freestanding Windows image self-contained; Windows programs should use
 * LoadLibraryA/GetProcAddress through the native builtins instead. */
void *dlopen(const char *path, int mode) {
    (void)path; (void)mode; return 0;
}

void *dlsym(void *handle, const char *name) {
    (void)handle; (void)name; return 0;
}

int dlclose(void *handle) {
    (void)handle; return 0;
}

char *dlerror(void) {
    return 0;
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

