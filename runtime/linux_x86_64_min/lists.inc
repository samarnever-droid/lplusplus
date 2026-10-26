/* ── Freestanding List runtime ──────────────────────────────────────────── */
typedef struct {
    int64_t *data;
    int64_t len;
    int64_t cap;
    uint64_t data_map_size;
    int arc_elements;
} LppList;

static void lpp_list_destroy(void *payload) {
    LppList *list = (LppList *)payload;
    if (!list) return;
    if (list->arc_elements) {
        for (int64_t i = 0; i < list->len; ++i) {
            lpp_arc_release((void *)(intptr_t)list->data[i]);
        }
    }
    if (list->data) lpp_sys_munmap(list->data, list->data_map_size);
}

void lpp_list_trace(void *payload, void *ctx) {
    LppList *list = (LppList *)payload;
    if (!list || !list->arc_elements) return;
    for (int64_t i = 0; i < list->len; ++i) {
        lpp_trace_visit(ctx, (void *)(intptr_t)list->data[i]);
    }
}

static void *lpp_list_new_with_mode(int arc_elements) {
    LppList *list = (LppList *)lpp_arc_alloc_with_destructor(
        (int64_t)sizeof(LppList), lpp_list_destroy
    );
    if (!list) return 0;
    list->arc_elements = arc_elements;
    return list;
}

void *lpp_list_new(void) {
    return lpp_list_new_with_mode(0);
}

void *lpp_list_new_arc(void) {
    return lpp_list_new_with_mode(1);
}

void lpp_list_push(void *raw, int64_t value) {
    LppList *list = (LppList *)raw;
    if (!list) return;
    if (list->len == list->cap) {
        int64_t next_cap = list->cap == 0 ? 8 : list->cap * 2;
        if (next_cap < list->cap || next_cap > (int64_t)(0x7fffffffffffffffLL / 8)) return;
        uint64_t next_size = lpp_page_round((uint64_t)next_cap * sizeof(int64_t));
        int64_t *next_data = (int64_t *)lpp_sys_mmap(next_size);
        if (!next_data) return;
        for (int64_t i = 0; i < list->len; ++i) next_data[i] = list->data[i];
        if (list->data) lpp_sys_munmap(list->data, list->data_map_size);
        list->data = next_data;
        list->cap = next_cap;
        list->data_map_size = next_size;
    }
    if (list->arc_elements) lpp_arc_retain((void *)(intptr_t)value);
    list->data[list->len++] = value;
}

void lpp_list_push_arc(void *list, void *value) {
    lpp_list_push(list, (int64_t)(intptr_t)value);
}

void lpp_list_push_float(void *list, double value) {
    int64_t ival;
    for (int i = 0; i < 8; i++) ((char*)&ival)[i] = ((char*)&value)[i];
    lpp_list_push(list, ival);
}

void lpp_list_push_bool(void *list, int8_t value) {
    lpp_list_push(list, value ? 1 : 0);
}

int64_t lpp_list_get(void *raw, int64_t index) {
    LppList *list = (LppList *)raw;
    if (!list || index < 0 || index >= list->len) { __asm__ volatile("syscall"::"a"(60),"D"(101):"rcx","r11"); for(;;); }
    return list->data[index];
}

void lpp_list_set(void *raw, int64_t index, int64_t value) {
    LppList *list = (LppList *)raw;
    if (!list || index < 0 || index >= list->len) lpp_exit(101);
    if (list->arc_elements) {
        lpp_arc_retain((void *)(intptr_t)value);
        lpp_arc_release((void *)(intptr_t)list->data[index]);
    }
    list->data[index] = value;
}

void lpp_list_set_bool(void *list, int64_t index, int8_t value) {
    lpp_list_set(list, index, value ? 1 : 0);
}

void lpp_list_set_float(void *list, int64_t index, double value) {
    int64_t bits;
    for (int i = 0; i < 8; ++i) ((char *)&bits)[i] = ((char *)&value)[i];
    lpp_list_set(list, index, bits);
}

void lpp_list_set_arc(void *list, int64_t index, void *value) {
    lpp_list_set(list, index, (int64_t)(intptr_t)value);
}

double lpp_list_get_float(void *list, int64_t index) {
    int64_t ival = lpp_list_get(list, index);
    double fval;
    for (int i = 0; i < 8; i++) ((char*)&fval)[i] = ((char*)&ival)[i];
    return fval;
}

int8_t lpp_list_get_bool(void *list, int64_t index) {
    return lpp_list_get(list, index) != 0;
}

void *lpp_list_get_arc(void *list, int64_t index) {
    return (void *)(intptr_t)lpp_list_get(list, index);
}

int64_t lpp_list_len(void *raw) {
    LppList *list = (LppList *)raw;
    return list ? list->len : 0;
}

void lpp_panic_div_zero(void) { lpp_exit(101); }
int64_t lpp_list_pop(void *raw) {
    LppList *list = (LppList *)raw;
    if (!list || list->len <= 0) lpp_exit(101);
    list->len--;
    return list->data[list->len];
}

void lpp_list_free(void *list) {
    lpp_arc_release(list);
}

typedef struct { void *base; int64_t start, length, generation, kind; } LppSlice;
static void *lpp_slice_checked_base(LppSlice *view) {
    if (!view || !view->base || !view->generation) lpp_exit(101);
    int64_t raw = lpp_weak_get((int64_t)(intptr_t)view->base, view->generation);
    if (!raw) lpp_exit(101);
    return (void *)(intptr_t)raw;
}
void *lpp_slice_init(void *storage, void *base, int64_t start, int64_t length, int64_t kind) {
    if (!storage || !base || start < 0 || length < 0 || start > 0x7fffffffffffffffLL - length) lpp_exit(101);
    int64_t source_length = 0;
    if (kind == 0) { const char *p = (const char *)base; while (p[source_length]) source_length++; }
    else source_length = lpp_list_len(base);
    if (start > source_length || length > source_length - start) lpp_exit(101);
    LppSlice *view = (LppSlice *)storage;
    view->base = base; view->start = start; view->length = length;
    view->generation = lpp_weak_generation(base); view->kind = kind;
    if (!view->generation) lpp_exit(101);
    return view;
}
int64_t lpp_slice_len(void *raw) { LppSlice *view=(LppSlice *)raw; (void)lpp_slice_checked_base(view); return view->length; }
int64_t lpp_slice_get(void *raw, int64_t index) {
    LppSlice *view=(LppSlice *)raw; void *base=lpp_slice_checked_base(view);
    if (view->kind != 1 || index < 0 || index >= view->length) lpp_exit(101);
    return lpp_list_get(base, view->start + index);
}
double lpp_slice_get_float(void *raw, int64_t index) {
    int64_t bits = lpp_slice_get(raw, index);
    double value;
    for (int i = 0; i < 8; i++) {
        ((char *)&value)[i] = ((char *)&bits)[i];
    }
    return value;
}
int8_t lpp_slice_get_bool(void *raw, int64_t index) {
    return lpp_slice_get(raw, index) != 0;
}
char *lpp_str_slice_get(void *raw, int64_t index) {
    LppSlice *view = (LppSlice *)raw;
    const char *base = (const char *)lpp_slice_checked_base(view);
    if (view->kind != 0 || index < 0 || index >= view->length) lpp_exit(101);
    char *result = (char *)lpp_arc_alloc(2);
    if (!result) lpp_exit(101);
    result[0] = base[view->start + index];
    result[1] = 0;
    return result;
}
char *lpp_str_slice_to_str(void *raw) {
    LppSlice *view = (LppSlice *)raw;
    const char *base = (const char *)lpp_slice_checked_base(view);
    if (view->kind != 0) lpp_exit(101);
    char *result = (char *)lpp_arc_alloc(view->length + 1);
    if (!result) lpp_exit(101);
    for (int64_t i = 0; i < view->length; i++) {
        result[i] = base[view->start + i];
    }
    result[view->length] = 0;
    return result;
}

