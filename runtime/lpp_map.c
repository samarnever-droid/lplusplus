/* ── Hash Map library (runtime/lpp_map.c) ──────────────────────────────────
 * Open-addressing linear probing hash map supporting Int and Str keys/values.
 */

#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include <stdio.h>

extern void *lpp_arc_alloc_with_destructor(int64_t size, void (*destructor)(void *));
extern void  lpp_arc_release(void *ptr);

typedef struct LppMapEntry {
    int64_t key;
    int64_t val;
    int is_str_key;
    int occupied; /* 0 = empty, 1 = occupied, 2 = deleted */
} LppMapEntry;

typedef struct LppMap {
    LppMapEntry *entries;
    int64_t cap;
    int64_t len;
} LppMap;

static uint64_t lpp_hash_str(const char *s) {
    if (!s) return 0;
    uint64_t hash = 14695981039346656037ULL;
    while (*s) {
        hash ^= (unsigned char)(*s++);
        hash *= 1099511628211ULL;
    }
    return hash;
}

static uint64_t lpp_hash_int(int64_t key) {
    uint64_t k = (uint64_t)key;
    k = (~k) + (k << 21);
    k = k ^ (k >> 24);
    k = (k + (k << 3)) + (k << 8);
    k = k ^ (k >> 14);
    k = (k + (k << 2)) + (k << 4);
    k = k ^ (k >> 28);
    k = k + (k << 31);
    return k;
}

static void lpp_map_destroy(void *payload) {
    LppMap *m = (LppMap *)payload;
    if (!m) return;
    if (m->entries) free(m->entries);
    m->entries = NULL;
    m->cap = 0;
    m->len = 0;
}

void *lpp_map_new(void) {
    LppMap *m = (LppMap *)lpp_arc_alloc_with_destructor((int64_t)sizeof(LppMap), lpp_map_destroy);
    if (!m) return NULL;
    m->cap = 16;
    m->len = 0;
    m->entries = (LppMapEntry *)calloc((size_t)m->cap, sizeof(LppMapEntry));
    return m;
}

static void lpp_map_rehash(LppMap *m) {
    int64_t old_cap = m->cap;
    LppMapEntry *old_entries = m->entries;

    m->cap = old_cap * 2;
    m->entries = (LppMapEntry *)calloc((size_t)m->cap, sizeof(LppMapEntry));
    m->len = 0;

    for (int64_t i = 0; i < old_cap; i++) {
        if (old_entries[i].occupied == 1) {
            int64_t key = old_entries[i].key;
            int64_t val = old_entries[i].val;
            int is_str = old_entries[i].is_str_key;
            uint64_t h = is_str ? lpp_hash_str((const char *)(uintptr_t)key) : lpp_hash_int(key);
            int64_t idx = (int64_t)(h % (uint64_t)m->cap);
            while (m->entries[idx].occupied == 1) {
                idx = (idx + 1) % m->cap;
            }
            m->entries[idx].key = key;
            m->entries[idx].val = val;
            m->entries[idx].is_str_key = is_str;
            m->entries[idx].occupied = 1;
            m->len++;
        }
    }
    free(old_entries);
}

static void lpp_map_put_internal(LppMap *m, int64_t key, int64_t val, int is_str) {
    if (!m) return;
    if (m->len * 10 >= m->cap * 7) {
        lpp_map_rehash(m);
    }

    uint64_t h = is_str ? lpp_hash_str((const char *)(uintptr_t)key) : lpp_hash_int(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t first_tombstone = -1;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == is_str) {
            int match = is_str
                ? (strcmp((const char *)(uintptr_t)m->entries[idx].key, (const char *)(uintptr_t)key) == 0)
                : (m->entries[idx].key == key);
            if (match) {
                m->entries[idx].val = val;
                return;
            }
        }
        if (m->entries[idx].occupied == 2 && first_tombstone == -1) {
            first_tombstone = idx;
        }
        idx = (idx + 1) % m->cap;
    }

    if (first_tombstone != -1) {
        idx = first_tombstone;
    }

    m->entries[idx].key = key;
    m->entries[idx].val = val;
    m->entries[idx].is_str_key = is_str;
    m->entries[idx].occupied = 1;
    m->len++;
}

void lpp_map_put(void *map, int64_t key, int64_t val) {
    lpp_map_put_internal((LppMap *)map, key, val, 0);
}

void lpp_map_put_str(void *map, const char *key, int64_t val) {
    lpp_map_put_internal((LppMap *)map, (int64_t)(uintptr_t)key, val, 1);
}

int64_t lpp_map_get(void *map, int64_t key) {
    LppMap *m = (LppMap *)map;
    if (!m || m->len == 0) return 0;

    uint64_t h = lpp_hash_int(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) {
            return m->entries[idx].val;
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
    return 0;
}

int64_t lpp_map_get_str(void *map, const char *key) {
    LppMap *m = (LppMap *)map;
    if (!m || !key || m->len == 0) return 0;

    uint64_t h = lpp_hash_str(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) {
            if (strcmp((const char *)(uintptr_t)m->entries[idx].key, key) == 0) {
                return m->entries[idx].val;
            }
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
    return 0;
}

int64_t lpp_map_has(void *map, int64_t key) {
    LppMap *m = (LppMap *)map;
    if (!m || m->len == 0) return 0;

    uint64_t h = lpp_hash_int(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) {
            return 1;
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
    return 0;
}

int64_t lpp_map_has_str(void *map, const char *key) {
    LppMap *m = (LppMap *)map;
    if (!m || !key || m->len == 0) return 0;

    uint64_t h = lpp_hash_str(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) {
            if (strcmp((const char *)(uintptr_t)m->entries[idx].key, key) == 0) {
                return 1;
            }
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
    return 0;
}

int64_t lpp_map_len(void *map) {
    LppMap *m = (LppMap *)map;
    return m ? m->len : 0;
}

void lpp_map_remove(void *map, int64_t key) {
    LppMap *m = (LppMap *)map;
    if (!m || m->len == 0) return;

    uint64_t h = lpp_hash_int(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) {
            m->entries[idx].occupied = 2;
            m->len--;
            return;
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
}

void lpp_map_remove_str(void *map, const char *key) {
    LppMap *m = (LppMap *)map;
    if (!m || !key || m->len == 0) return;

    uint64_t h = lpp_hash_str(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 1) {
            if (strcmp((const char *)(uintptr_t)m->entries[idx].key, key) == 0) {
                m->entries[idx].occupied = 2;
                m->len--;
                return;
            }
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
}

void lpp_map_put_float(void *map, int64_t key, double val) {
    int64_t ival;
    memcpy(&ival, &val, sizeof(double));
    lpp_map_put(map, key, ival);
}

double lpp_map_get_float(void *map, int64_t key) {
    int64_t ival = lpp_map_get(map, key);
    double fval;
    memcpy(&fval, &ival, sizeof(double));
    return fval;
}

#define lpp_map_get(m, k) lpp_map_get((m), (int64_t)(uintptr_t)(k))
#define lpp_map_has(m, k) lpp_map_has((m), (int64_t)(uintptr_t)(k))
#define lpp_map_remove(m, k) lpp_map_remove((m), (int64_t)(uintptr_t)(k))
#define lpp_map_put(m, k, v) lpp_map_put((m), (int64_t)(uintptr_t)(k), (int64_t)(uintptr_t)(v))
