/* ── List objects ────────────────────────────────────────────────────────── */

enum {
    LPP_SHIM_ELEM_UNSET = 0,
    LPP_SHIM_ELEM_SCALAR,
    LPP_SHIM_ELEM_FLOAT,
    LPP_SHIM_ELEM_BOOL,
    LPP_SHIM_ELEM_ARC,
};

typedef struct {
    int64_t len;
    int64_t cap;
    int64_t *slots;
    int element_kind;
} LppShimList;

static void lpp_shim_list_destroy(void *raw) {
    LppShimList *list = (LppShimList *)raw;
    if (list->element_kind == LPP_SHIM_ELEM_ARC) {
        for (int64_t i = 0; i < list->len; i++) {
            lpp_arc_release((void *)list->slots[i]);
        }
    }
    free(list->slots);
}

void lpp_list_trace(void *raw, void *ctx) {
    LppShimList *list = (LppShimList *)raw;
    if (!list || list->element_kind != LPP_SHIM_ELEM_ARC) return;
    for (int64_t i = 0; i < list->len; i++) {
        lpp_trace_visit(ctx, (void *)list->slots[i]);
    }
}

static LppShimList *lpp_shim_list_new(int element_kind) {
    void *payload = lpp_arc_alloc_with_destructor(sizeof(LppShimList), lpp_shim_list_destroy);
    LppShimList *list = (LppShimList *)payload;
    list->len = 0;
    list->cap = 0;
    list->slots = (int64_t *)0;
    list->element_kind = element_kind;
    return list;
}

void *lpp_list_new(void) { return lpp_shim_list_new(LPP_SHIM_ELEM_UNSET); }

void *lpp_list_new_arc(void) { return lpp_shim_list_new(LPP_SHIM_ELEM_UNSET); }

static void lpp_shim_list_reserve(LppShimList *list, int element_kind, int64_t need) {
    if (need <= list->cap) return;
    if (list->element_kind == LPP_SHIM_ELEM_UNSET) list->element_kind = element_kind;
    int64_t cap = list->cap ? list->cap * 2 : 4;
    while (cap < need) cap *= 2;
    int64_t *slots = (int64_t *)realloc(list->slots, (size_t)cap * sizeof(int64_t));
    if (!slots) { fprintf(stderr, "lpp shim: list growth failed\n"); exit(101); }
    list->slots = slots;
    list->cap = cap;
}

static LppShimList *lpp_shim_check(void *raw, const char *op) {
    LppShimList *list = (LppShimList *)raw;
    if (!list) { fprintf(stderr, "lpp shim: %s on null list\n", op); exit(101); }
    return list;
}

static void lpp_shim_push(LppShimList *list, int element_kind, int64_t value) {
    lpp_shim_list_reserve(list, element_kind, list->len + 1);
    list->slots[list->len++] = value;
}

void lpp_list_push(void *raw, int64_t value) {
    lpp_shim_push(lpp_shim_check(raw, "push"), LPP_SHIM_ELEM_SCALAR, value);
}

void lpp_list_push_arc(void *raw, void *value) {
    LppShimList *list = lpp_shim_check(raw, "push_arc");
    lpp_arc_retain(value);
    lpp_shim_push(list, LPP_SHIM_ELEM_ARC, (int64_t)(intptr_t)value);
}

void lpp_list_push_float(void *raw, double value) {
    uint64_t bits;
    memcpy(&bits, &value, sizeof(bits));
    lpp_shim_push(lpp_shim_check(raw, "push_float"), LPP_SHIM_ELEM_FLOAT, (int64_t)bits);
}

void lpp_list_push_bool(void *raw, int8_t value) {
    lpp_shim_push(lpp_shim_check(raw, "push_bool"), LPP_SHIM_ELEM_BOOL, value);
}

static int64_t lpp_shim_index(LppShimList *list, int64_t index, const char *op) {
    if (index < 0 || index >= list->len) {
        fprintf(stderr, "lpp shim: %s out of bounds (%lld >= %lld)\n",
                op, (long long)index, (long long)list->len);
        exit(101);
    }
    return list->slots[index];
}

int64_t lpp_list_get(void *raw, int64_t index) {
    return lpp_shim_index(lpp_shim_check(raw, "get"), index, "get");
}

void *lpp_list_get_arc(void *raw, int64_t index) {
    LppShimList *list = lpp_shim_check(raw, "get_arc");
    return (void *)(intptr_t)lpp_shim_index(list, index, "get_arc");
}

double lpp_list_get_float(void *raw, int64_t index) {
    uint64_t bits = (uint64_t)lpp_shim_index(lpp_shim_check(raw, "get_float"), index, "get_float");
    double value;
    memcpy(&value, &bits, sizeof(value));
    return value;
}

int8_t lpp_list_get_bool(void *raw, int64_t index) {
    return (int8_t)lpp_shim_index(lpp_shim_check(raw, "get_bool"), index, "get_bool");
}

void lpp_list_set(void *raw, int64_t index, int64_t value) {
    LppShimList *list = lpp_shim_check(raw, "set");
    if (index < 0 || index >= list->len) {
        fprintf(stderr, "lpp shim: set out of bounds\n");
        exit(101);
    }
    list->slots[index] = value;
}

void lpp_list_set_arc(void *raw, int64_t index, void *value) {
    LppShimList *list = lpp_shim_check(raw, "set_arc");
    if (index < 0 || index >= list->len) {
        fprintf(stderr, "lpp shim: set_arc out of bounds\n");
        exit(101);
    }
    /* Retain the new owner first, then release the replaced element —
     * the interpreter's store_place order (a self-store survives). */
    lpp_arc_retain(value);
    int64_t old = list->slots[index];
    list->slots[index] = (int64_t)(intptr_t)value;
    lpp_arc_release((void *)(intptr_t)old);
}

void lpp_list_set_float(void *raw, int64_t index, double value) {
    uint64_t bits;
    memcpy(&bits, &value, sizeof(bits));
    lpp_list_set(raw, index, (int64_t)bits);
}

void lpp_list_set_bool(void *raw, int64_t index, int8_t value) {
    lpp_list_set(raw, index, value);
}

int64_t lpp_list_len(void *raw) {
    return lpp_shim_check(raw, "len")->len;
}

