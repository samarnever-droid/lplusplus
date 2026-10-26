/* ── 5C2: slice handles (Family C) ───────────────────────────────────────── */

typedef struct {
    void *base;
    int64_t start;
    int64_t length;
    int64_t generation;
    int64_t kind;
} LppShimSlice;

static int64_t lpp_shim_weak_generation(void *payload) {
    if (!payload) return 0;
    LppShimHeader *header = (LppShimHeader *)payload - 1;
    if (lpp_shim_is_immortal(header)) return (int64_t)LPP_SHIM_IMMORTAL;
    return header->generation;
}

static void *lpp_shim_weak_get(void *payload, int64_t expected_generation) {
    if (!payload || expected_generation == 0) return (void *)0;
    LppShimHeader *header = (LppShimHeader *)payload - 1;
    if (lpp_shim_is_immortal(header)) {
        return expected_generation == (int64_t)LPP_SHIM_IMMORTAL ? payload : (void *)0;
    }
    if (header->magic != LPP_SHIM_IMMORTAL || header->generation != (int32_t)expected_generation) {
        return (void *)0;
    }
    return payload;
}

static void *lpp_shim_slice_checked_base(LppShimSlice *view) {
    if (!view || !view->base || !view->generation) {
        fprintf(stderr, "lpp shim: slice of a dead object\n");
        exit(101);
    }
    void *base = lpp_shim_weak_get(view->base, view->generation);
    if (!base) {
        fprintf(stderr, "lpp shim: slice of a released object\n");
        exit(101);
    }
    return base;
}

void *lpp_slice_init(void *storage, void *base, int64_t start, int64_t length, int64_t kind) {
    if (!storage || !base || start < 0 || length < 0 ||
        start > INT64_MAX - length) {
        fprintf(stderr, "lpp shim: invalid slice range\n");
        exit(101);
    }
    int64_t source_length;
    if (kind == 0) source_length = (int64_t)strlen((const char *)base);
    else source_length = lpp_list_len(base);
    if (start > source_length || length > source_length - start) {
        fprintf(stderr, "lpp shim: slice out of range\n");
        exit(101);
    }
    LppShimSlice *view = (LppShimSlice *)storage;
    view->base = base;
    view->start = start;
    view->length = length;
    view->generation = lpp_shim_weak_generation(base);
    view->kind = kind;
    if (!view->generation) {
        fprintf(stderr, "lpp shim: slice of an untracked object\n");
        exit(101);
    }
    return view;
}

int64_t lpp_slice_len(void *raw) {
    LppShimSlice *view = (LppShimSlice *)raw;
    (void)lpp_shim_slice_checked_base(view);
    return view->length;
}

int64_t lpp_slice_get(void *raw, int64_t index) {
    LppShimSlice *view = (LppShimSlice *)raw;
    void *base = lpp_shim_slice_checked_base(view);
    if (view->kind != 1 || index < 0 || index >= view->length) {
        fprintf(stderr, "lpp shim: slice index out of range\n");
        exit(101);
    }
    return lpp_list_get(base, view->start + index);
}

double lpp_slice_get_float(void *raw, int64_t index) {
    int64_t bits = lpp_slice_get(raw, index);
    double value;
    memcpy(&value, &bits, sizeof(value));
    return value;
}

int8_t lpp_slice_get_bool(void *raw, int64_t index) {
    return lpp_slice_get(raw, index) != 0;
}

char *lpp_str_slice_get(void *raw, int64_t index) {
    LppShimSlice *view = (LppShimSlice *)raw;
    const char *base = (const char *)lpp_shim_slice_checked_base(view);
    if (view->kind != 0 || index < 0 || index >= view->length) {
        fprintf(stderr, "lpp shim: str slice index out of range\n");
        exit(101);
    }
    char *result = lpp_shim_string_capacity(1);
    result[0] = base[view->start + index];
    return result;
}

char *lpp_str_slice_to_str(void *raw) {
    LppShimSlice *view = (LppShimSlice *)raw;
    const char *base = (const char *)lpp_shim_slice_checked_base(view);
    if (view->kind != 0) {
        fprintf(stderr, "lpp shim: str_slice_to_str on a list slice\n");
        exit(101);
    }
    char *result = lpp_shim_string_capacity((size_t)view->length);
    memcpy(result, base + view->start, (size_t)view->length);
    return result;
}

