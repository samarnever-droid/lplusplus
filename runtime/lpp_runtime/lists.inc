/* ── List<Int> ───────────────────────────────────────────────────────────── */

typedef void (*LppListElementFn)(int64_t value);

typedef struct {
    int64_t *data;
    int64_t  len;
    int64_t  cap;
    /* NULL for value elements; retain/drop callbacks for ARC pointer elements. */
    LppListElementFn retain_element;
    LppListElementFn drop_element;
} LppList;

static void lpp_list_arc_retain_element(int64_t value) {
    /* Only retain heap pointers. Skip raw/static pointers (string literals,
     * small integers) that don't have an ARC header. A valid ARC header is
     * 8-byte aligned and points to a non-null heap address. */
    void *ptr = (void *)(intptr_t)value;
    if (!ptr || ((uintptr_t)ptr & 7) != 0 || (uintptr_t)ptr < 0x1000) return;
    lpp_arc_retain(ptr);
}

static void lpp_list_arc_drop_element(int64_t value) {
    void *ptr = (void *)(intptr_t)value;
    if (!ptr || ((uintptr_t)ptr & 7) != 0 || (uintptr_t)ptr < 0x1000) return;
    lpp_arc_release(ptr);
}

static void lpp_list_destroy(void *payload) {
    LppList *l = (LppList *)payload;
    if (!l) return;
    if (l->drop_element) {
        for (int64_t i = 0; i < l->len; ++i) {
            l->drop_element(l->data[i]);
        }
    }
    free(l->data);
    l->data = NULL;
    l->len = 0;
    l->cap = 0;
}

void lpp_list_trace(void *payload, void *ctx) {
    LppList *l = (LppList *)payload;
    if (!l || l->drop_element != lpp_list_arc_drop_element) return;
    for (int64_t i = 0; i < l->len; ++i) {
        lpp_trace_visit(ctx, (void *)(intptr_t)l->data[i]);
    }
}

static void *lpp_list_new_with_ownership(
    LppListElementFn retain_element,
    LppListElementFn drop_element
) {
    LppList *l = (LppList *)lpp_arc_alloc_with_destructor(
        (int64_t)sizeof(LppList), lpp_list_destroy
    );
    if (!l) {
        lpp_panic("out of memory while creating list");
    }
    l->retain_element = retain_element;
    l->drop_element = drop_element;
    return l;
}

/* List[Int] stores values and owns no element references. */
void *lpp_list_new(void) {
    return lpp_list_new_with_ownership(NULL, NULL);
}

/* List[ARC Object] owns one retained reference per element. */
void *lpp_list_new_arc(void) {
    return lpp_list_new_with_ownership(
        lpp_list_arc_retain_element,
        lpp_list_arc_drop_element
    );
}

void lpp_list_push(void *list, int64_t value) {
    LppList *l = (LppList *)list;
    if (!l) {
        lpp_panic("push attempted on null list pointer");
    }
    if (l->len == l->cap) {
        if (l->cap > INT64_MAX / 2) {
            lpp_panic("list capacity overflow");
        }
        int64_t new_cap = l->cap == 0 ? 8 : l->cap * 2;
        if (new_cap > INT64_MAX / (int64_t)sizeof(int64_t)) {
            lpp_panic("list allocation size overflow");
        }
        int64_t *new_data = (int64_t *)realloc(l->data, (size_t)new_cap * sizeof(int64_t));
        if (!new_data) {
            lpp_panic("out of memory while growing list");
        }
        l->data = new_data;
        l->cap = new_cap;
    }
    if (l->retain_element) l->retain_element(value);
    l->data[l->len++] = value;
}

/* Generated code's integer div/mod-by-zero guard: the defined L++
 * panic (exit 101), never a hardware trap. */
void lpp_panic_div_zero(void) { lpp_panic("integer division by zero"); }

/* Store one ARC object reference in List[T]. */
void lpp_list_push_arc(void *list, void *value) {
    LppList *l = (LppList *)list;
    if (l && !l->retain_element) {
        l->retain_element = lpp_list_arc_retain_element;
        l->drop_element = lpp_list_arc_drop_element;
    }
    lpp_list_push(list, (int64_t)(intptr_t)value);
}

void lpp_list_push_float(void *list, double value) {
    int64_t ival;
    memcpy(&ival, &value, sizeof(double));
    lpp_list_push(list, ival);
}

void lpp_list_push_bool(void *list, int8_t value) {
    lpp_list_push(list, value ? 1 : 0);
}

int64_t lpp_list_get(void *list, int64_t index) {
    LppList *l = (LppList *)list;
    if (!l) {
        lpp_panic("list index access attempted on null list pointer");
    }
    if (index < 0 || index >= l->len) {
        lpp_panic("list index out of bounds: index %lld, len %lld", (long long)index, (long long)l->len);
    }
    return l->data[index];
}

void lpp_list_set(void *list, int64_t index, int64_t value) {
    LppList *l = (LppList *)list;
    if (!l) {
        lpp_panic("list set attempted on null list pointer");
    }
    if (index < 0 || index >= l->len) {
        lpp_panic("list index out of bounds on set: index %lld, len %lld", (long long)index, (long long)l->len);
    }
    /* Retain the incoming edge before dropping the old one. This order is
     * required for self-assignment such as set(xs, 0, get(xs, 0)), where the
     * list may hold the last reference to that object. */
    if (l->retain_element && value) {
        l->retain_element(value);
    }
    if (l->drop_element && l->data[index]) {
        l->drop_element(l->data[index]);
    }
    l->data[index] = value;
}

void lpp_list_set_bool(void *list, int64_t index, int8_t value) {
    lpp_list_set(list, index, value ? 1 : 0);
}

void lpp_list_set_float(void *list, int64_t index, double value) {
    int64_t bits;
    memcpy(&bits, &value, sizeof(bits));
    lpp_list_set(list, index, bits);
}

void lpp_list_set_arc(void *list, int64_t index, void *value) {
    lpp_list_set(list, index, (int64_t)(intptr_t)value);
}

double lpp_list_get_float(void *list, int64_t index) {
    int64_t ival = lpp_list_get(list, index);
    double fval;
    memcpy(&fval, &ival, sizeof(double));
    return fval;
}

int8_t lpp_list_get_bool(void *list, int64_t index) {
    return lpp_list_get(list, index) != 0;
}

/* List element reads are borrowed; callers retain only when they create an
 * additional owner (assignment/return/store). */
void *lpp_list_get_arc(void *list, int64_t index) {
    return (void *)(intptr_t)lpp_list_get(list, index);
}

int64_t lpp_list_len(void *list) {
    LppList *l = (LppList *)list;
    return l ? l->len : 0;
}

int64_t lpp_list_pop(void *list) {
    LppList *l = (LppList *)list;
    if (!l || l->len <= 0) {
        lpp_panic("list pop attempted on empty or null list");
    }
    l->len--;
    return l->data[l->len];
}

void lpp_list_free(void *list) {
    /* Compatibility entry point. In ownership-aware AOT code list lifetime is
     * automatic, so this is only a single reference release, never raw free. */
    lpp_arc_release(list);
}

void lpp_list_insert(void *list, int64_t index, int64_t value) {
    LppList *l = (LppList *)list;
    if (!l) lpp_panic("list insert attempted on null pointer");
    if (index < 0 || index > l->len) lpp_panic("list insert index %lld out of bounds (len %lld)", (long long)index, (long long)l->len);
    if (l->len == l->cap) {
        int64_t new_cap = l->cap == 0 ? 8 : l->cap * 2;
        int64_t *new_data = (int64_t *)realloc(l->data, (size_t)new_cap * sizeof(int64_t));
        if (!new_data) lpp_panic("out of memory while inserting into list");
        l->data = new_data;
        l->cap = new_cap;
    }
    if (index < l->len) {
        memmove(&l->data[index + 1], &l->data[index], (size_t)(l->len - index) * sizeof(int64_t));
    }
    if (l->retain_element) l->retain_element(value);
    l->data[index] = value;
    l->len++;
}

int64_t lpp_list_remove(void *list, int64_t index) {
    LppList *l = (LppList *)list;
    if (!l) lpp_panic("list remove attempted on null pointer");
    if (index < 0 || index >= l->len) lpp_panic("list remove index %lld out of bounds (len %lld)", (long long)index, (long long)l->len);
    int64_t val = l->data[index];
    if (index < l->len - 1) {
        memmove(&l->data[index], &l->data[index + 1], (size_t)(l->len - index - 1) * sizeof(int64_t));
    }
    l->len--;
    return val;
}

void lpp_list_reserve(void *list, int64_t capacity) {
    LppList *l = (LppList *)list;
    if (!l || capacity <= l->cap) return;
    int64_t *new_data = (int64_t *)realloc(l->data, (size_t)capacity * sizeof(int64_t));
    if (!new_data) lpp_panic("out of memory in list_reserve");
    l->data = new_data;
    l->cap = capacity;
}

int64_t lpp_list_capacity(void *list) {
    LppList *l = (LppList *)list;
    return l ? l->cap : 0;
}

void lpp_list_clear(void *list) {
    LppList *l = (LppList *)list;
    if (!l) return;
    if (l->drop_element) {
        for (int64_t i = 0; i < l->len; i++) {
            l->drop_element(l->data[i]);
        }
    }
    l->len = 0;
}

void lpp_list_truncate(void *list, int64_t new_len) {
    LppList *l = (LppList *)list;
    if (!l || new_len >= l->len || new_len < 0) return;
    if (l->drop_element) {
        for (int64_t i = new_len; i < l->len; i++) {
            l->drop_element(l->data[i]);
        }
    }
    l->len = new_len;
}

void lpp_list_swap(void *list, int64_t i, int64_t j) {
    LppList *l = (LppList *)list;
    if (!l) lpp_panic("list swap on null pointer");
    if (i < 0 || i >= l->len || j < 0 || j >= l->len) lpp_panic("list swap indices out of bounds");
    int64_t tmp = l->data[i];
    l->data[i] = l->data[j];
    l->data[j] = tmp;
}

void lpp_list_reverse(void *list) {
    LppList *l = (LppList *)list;
    if (!l || l->len <= 1) return;
    int64_t i = 0, j = l->len - 1;
    while (i < j) {
        int64_t tmp = l->data[i];
        l->data[i] = l->data[j];
        l->data[j] = tmp;
        i++;
        j--;
    }
}

/* Introsort-lite: median-of-three quicksort with an insertion-sort cutoff and a
 * depth limit that falls back to heapsort. */
static int lpp_i64_lt(int64_t a, int64_t b) { return a < b; }
static int lpp_i64_gt(int64_t a, int64_t b) { return a > b; }
static int lpp_u64_lt(int64_t a, int64_t b) { return (uint64_t)a < (uint64_t)b; }

typedef int (*LppCmpFn)(int64_t, int64_t);

static void lpp_insertion_sort(int64_t *v, int64_t n, LppCmpFn before) {
    for (int64_t i = 1; i < n; ++i) {
        int64_t key = v[i];
        int64_t j = i - 1;
        while (j >= 0 && before(key, v[j])) { v[j + 1] = v[j]; --j; }
        v[j + 1] = key;
    }
}

static void lpp_sift_down(int64_t *v, int64_t start, int64_t n, LppCmpFn before) {
    int64_t root = start;
    for (;;) {
        int64_t child = 2 * root + 1;
        if (child >= n) break;
        if (child + 1 < n && before(v[child], v[child + 1])) child++;
        if (!before(v[root], v[child])) break;
        int64_t t = v[root]; v[root] = v[child]; v[child] = t;
        root = child;
    }
}

static void lpp_heap_sort(int64_t *v, int64_t n, LppCmpFn before) {
    for (int64_t i = n / 2 - 1; i >= 0; --i) lpp_sift_down(v, i, n, before);
    for (int64_t end = n - 1; end > 0; --end) {
        int64_t t = v[0]; v[0] = v[end]; v[end] = t;
        lpp_sift_down(v, 0, end, before);
    }
}

static void lpp_quick_sort(int64_t *v, int64_t n, int64_t depth, LppCmpFn before) {
    while (n > 16) {
        if (depth == 0) { lpp_heap_sort(v, n, before); return; }
        depth--;

        int64_t mid = n / 2, last = n - 1;
        if (before(v[mid], v[0]))    { int64_t t=v[mid]; v[mid]=v[0]; v[0]=t; }
        if (before(v[last], v[0]))   { int64_t t=v[last]; v[last]=v[0]; v[0]=t; }
        if (before(v[last], v[mid])) { int64_t t=v[last]; v[last]=v[mid]; v[mid]=t; }
        int64_t t0=v[mid]; v[mid]=v[0]; v[0]=t0;
        int64_t pivot = v[0];

        int64_t i = 0, j = n;
        for (;;) {
            do { ++i; } while (i < n  && before(v[i], pivot));
            do { --j; } while (j > 0   && before(pivot, v[j]));
            if (i >= j) break;
            int64_t t=v[i]; v[i]=v[j]; v[j]=t;
        }
        int64_t t1=v[0]; v[0]=v[j]; v[j]=t1;

        if (j < n - j - 1) {
            lpp_quick_sort(v, j, depth, before);
            v += j + 1; n -= j + 1;
        } else {
            lpp_quick_sort(v + j + 1, n - j - 1, depth, before);
            n = j;
        }
    }
    lpp_insertion_sort(v, n, before);
}

static void lpp_list_sort_with(void *list, LppCmpFn before) {
    LppList *l = (LppList *)list;
    if (!l || l->len < 2) return;
    int64_t depth = 0;
    for (int64_t m = l->len; m > 1; m >>= 1) depth += 2;
    lpp_quick_sort(l->data, l->len, depth, before);
}

void lpp_list_sort(void *list)      { lpp_list_sort_with(list, lpp_i64_lt); }
void lpp_list_sort_desc(void *list) { lpp_list_sort_with(list, lpp_i64_gt); }
void lpp_list_sort_u(void *list)    { lpp_list_sort_with(list, lpp_u64_lt); }

int64_t lpp_list_index_of(void *list, int64_t value) {
    LppList *l = (LppList *)list;
    if (!l) return -1;
    for (int64_t i = 0; i < l->len; ++i) {
        if (l->data[i] == value) return i;
    }
    return -1;
}

int64_t lpp_list_binary_search(void *list, int64_t value) {
    LppList *l = (LppList *)list;
    if (!l) return -1;
    int64_t lo = 0, hi = l->len - 1;
    while (lo <= hi) {
        int64_t mid = lo + (hi - lo) / 2;
        int64_t probe = l->data[mid];
        if (probe == value) return mid;
        if (probe < value) lo = mid + 1; else hi = mid - 1;
    }
    return -(lo + 1);
}

void lpp_list_extend(void *dst, void *src) {
    LppList *d = (LppList *)dst;
    LppList *s = (LppList *)src;
    if (!d) lpp_panic("list extend attempted on null destination");
    if (!s || s->len == 0) return;
    int64_t add = s->len;
    lpp_list_reserve(d, d->len + add);
    for (int64_t i = 0; i < add; ++i) {
        int64_t v = s->data[i];
        if (d->retain_element) d->retain_element(v);
        d->data[d->len + i] = v;
    }
    d->len += add;
}

