/* SamarOS — freestanding L++ runtime.
 *
 * The hosted L++ runtime (lpp_runtime.c in the repo root) assumes libc, an
 * OS and a heap.  Inside the kernel none of that exists, so this file
 * re-implements the slice of the language runtime the SamarOS kernel
 * actually uses: the bump allocator, Str helpers and the dynamic List type.
 *
 * Strings are NUL terminated char buffers; Lists are {data,len,cap} handles
 * of machine words, which is how the compiler lowers List[Int], List[Str]
 * and List[<struct>] alike.
 */
#include "../arch/samar.h"

/* ---- memory -------------------------------------------------------- */
static u32 heap_ptr, heap_base, heap_end;

void heap_init(u32 start, u32 end)
{
    heap_base = start;
    heap_ptr  = start;
    heap_end  = end;
}

void *lpp_alloc(int size)
{
    if (size <= 0) size = 1;
    u32 n = ((u32)size + 15u) & ~15u;
    if (heap_ptr + n >= heap_end) return NULL;      /* out of memory     */
    void *p = (void *)heap_ptr;
    heap_ptr += n;
    return p;
}

int heap_used(void)  { return (int)((heap_ptr - heap_base) >> 10); }   /* KiB */
int heap_total(void) { return (int)((heap_end - heap_base) >> 10); }   /* KiB */

void *k_memset(void *dst, int c, unsigned n)
{
    u8 *d = (u8 *)dst;
    while (n--) *d++ = (u8)c;
    return dst;
}

void *k_memcpy(void *dst, const void *src, unsigned n)
{
    u8 *d = (u8 *)dst;
    const u8 *s = (const u8 *)src;
    while (n >= 4) { *(u32 *)d = *(const u32 *)s; d += 4; s += 4; n -= 4; }
    while (n--) *d++ = *s++;
    return dst;
}

/* gcc lowers struct copies / array init to these */
void *memset(void *d, int c, unsigned n) { return k_memset(d, c, n); }
void *memcpy(void *d, const void *s, unsigned n) { return k_memcpy(d, s, n); }
void *memmove(void *d, const void *s, unsigned n)
{
    u8 *dd = (u8 *)d; const u8 *ss = (const u8 *)s;
    if (dd == ss || !n) return d;
    if (dd < ss) { while (n--) *dd++ = *ss++; }
    else { dd += n; ss += n; while (n--) *--dd = *--ss; }
    return d;
}

/* ---- Str ----------------------------------------------------------- */
int str_len(const char *s)
{
    int n = 0;
    if (!s) return 0;
    while (s[n]) n++;
    return n;
}

int len(const char *s) { return str_len(s); }

char *str_concat(const char *a, const char *b)
{
    int la = str_len(a), lb = str_len(b);
    char *out = (char *)lpp_alloc(la + lb + 1);
    if (!out) return (char *)"";
    for (int i = 0; i < la; i++) out[i] = a[i];
    for (int i = 0; i < lb; i++) out[la + i] = b[i];
    out[la + lb] = 0;
    return out;
}

int str_eq(const char *a, const char *b)
{
    if (!a || !b) return a == b;
    while (*a && *b) { if (*a != *b) return 0; a++; b++; }
    return *a == *b;
}

int char_at(const char *s, int i)
{
    if (!s || i < 0 || i >= str_len(s)) return 0;
    return (int)(unsigned char)s[i];
}

char *substr(const char *s, int start, int count)
{
    int n = str_len(s);
    if (start < 0) start = 0;
    if (start > n) start = n;
    if (count < 0 || start + count > n) count = n - start;
    char *out = (char *)lpp_alloc(count + 1);
    if (!out) return (char *)"";
    for (int i = 0; i < count; i++) out[i] = s[start + i];
    out[count] = 0;
    return out;
}

char *chr(int c)
{
    char *out = (char *)lpp_alloc(2);
    if (!out) return (char *)"";
    out[0] = (char)c;
    out[1] = 0;
    return out;
}

char *int_to_str(int v)
{
    char tmp[16];
    int i = 0, neg = v < 0;
    unsigned u = neg ? (unsigned)(-v) : (unsigned)v;
    if (!u) tmp[i++] = '0';
    while (u) { tmp[i++] = (char)('0' + (u % 10)); u /= 10; }
    if (neg) tmp[i++] = '-';
    char *out = (char *)lpp_alloc(i + 1);
    if (!out) return (char *)"";
    for (int j = 0; j < i; j++) out[j] = tmp[i - 1 - j];
    out[i] = 0;
    return out;
}

/* zero padded, used for clocks: pad2(7) == "07" */
char *pad2(int v)
{
    char *out = (char *)lpp_alloc(3);
    if (!out) return (char *)"";
    if (v < 0) v = 0;
    out[0] = (char)('0' + ((v / 10) % 10));
    out[1] = (char)('0' + (v % 10));
    out[2] = 0;
    return out;
}

int str_starts_with(const char *s, const char *prefix)
{
    if (!s || !prefix) return 0;
    while (*prefix) { if (*s++ != *prefix++) return 0; }
    return 1;
}

int str_index_of(const char *s, int ch)
{
    int n = str_len(s);
    for (int i = 0; i < n; i++) if ((int)(unsigned char)s[i] == ch) return i;
    return -1;
}

/* ---- List ---------------------------------------------------------- */
typedef struct { int *data; int len; int cap; } lpp_list;

void *list_new(void)
{
    lpp_list *l = (lpp_list *)lpp_alloc(sizeof(lpp_list));
    if (!l) return NULL;
    l->cap  = 8;
    l->len  = 0;
    l->data = (int *)lpp_alloc((int)sizeof(int) * l->cap);
    return l;
}

static void list_grow(lpp_list *l)
{
    int ncap = l->cap * 2;
    int *nd = (int *)lpp_alloc((int)sizeof(int) * ncap);
    if (!nd) return;
    for (int i = 0; i < l->len; i++) nd[i] = l->data[i];
    l->data = nd;
    l->cap  = ncap;
}

void list_push(void *h, int v)
{
    lpp_list *l = (lpp_list *)h;
    if (!l) return;
    if (l->len == l->cap) list_grow(l);
    if (l->len == l->cap) return;
    l->data[l->len++] = v;
}

int list_get(void *h, int i)
{
    lpp_list *l = (lpp_list *)h;
    if (!l || i < 0 || i >= l->len) return 0;
    return l->data[i];
}

void list_set(void *h, int i, int v)
{
    lpp_list *l = (lpp_list *)h;
    if (!l || i < 0 || i >= l->len) return;
    l->data[i] = v;
}

int list_len(void *h)
{
    lpp_list *l = (lpp_list *)h;
    return l ? l->len : 0;
}

void list_remove(void *h, int i)
{
    lpp_list *l = (lpp_list *)h;
    if (!l || i < 0 || i >= l->len) return;
    for (int j = i; j < l->len - 1; j++) l->data[j] = l->data[j + 1];
    l->len--;
}

void list_insert(void *h, int i, int v)
{
    lpp_list *l = (lpp_list *)h;
    if (!l) return;
    if (i < 0) i = 0;
    if (i > l->len) i = l->len;
    if (l->len == l->cap) list_grow(l);
    if (l->len == l->cap) return;
    for (int j = l->len; j > i; j--) l->data[j] = l->data[j - 1];
    l->data[i] = v;
    l->len++;
}

void list_clear(void *h)
{
    lpp_list *l = (lpp_list *)h;
    if (l) l->len = 0;
}

/* ---- numeric helpers ----------------------------------------------- */
int lpp_abs(int v)   { return v < 0 ? -v : v; }
int lpp_min(int a, int b) { return a < b ? a : b; }
int lpp_max(int a, int b) { return a > b ? a : b; }
int lpp_clamp(int v, int lo, int hi) { return v < lo ? lo : (v > hi ? hi : v); }

/* integer square root — used by the UI for radial falloff */
int isqrt(int v)
{
    if (v <= 0) return 0;
    int x = v, y = (x + 1) / 2;
    while (y < x) { x = y; y = (x + v / x) / 2; }
    return x;
}

/* fixed point sine, input in degrees, output scaled by 1000 */
static const short sin_tab[91] = {
       0,  17,  35,  52,  70,  87, 105, 122, 139, 156, 174, 191, 208, 225, 242,
     259, 276, 292, 309, 326, 342, 358, 375, 391, 407, 423, 438, 454, 469, 485,
     500, 515, 530, 545, 559, 574, 588, 602, 616, 629, 643, 656, 669, 682, 695,
     707, 719, 731, 743, 755, 766, 777, 788, 799, 809, 819, 829, 839, 848, 857,
     866, 875, 883, 891, 899, 906, 914, 921, 927, 934, 940, 946, 951, 956, 961,
     966, 970, 974, 978, 982, 985, 988, 990, 993, 995, 996, 998, 999, 999, 1000,
    1000
};

int sin_deg(int deg)
{
    deg %= 360;
    if (deg < 0) deg += 360;
    if (deg <= 90)  return sin_tab[deg];
    if (deg <= 180) return sin_tab[180 - deg];
    if (deg <= 270) return -sin_tab[deg - 180];
    return -sin_tab[360 - deg];
}

int cos_deg(int deg) { return sin_deg(deg + 90); }
