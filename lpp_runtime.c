/*
 * lpp_runtime.c  —  L++ Runtime Library (C implementation)
 *
 * Compile once into a static library or object file, then link with any
 * L++-generated object file to produce a native executable.
 *
 * Build:
 *   cl.exe  /nologo /O2 /c lpp_runtime.c /Fo:lpp_runtime.obj
 *   gcc -O2 -c lpp_runtime.c -o lpp_runtime.o
 *   clang -O2 -c lpp_runtime.c -o lpp_runtime.o
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <errno.h>

/* ── I/O ──────────────────────────────────────────────────────────────────── */

void lpp_print_int(int64_t value) {
    printf("%lld\n", (long long)value);
    fflush(stdout);
}

void lpp_print_str(const char *ptr) {
    if (ptr) { puts(ptr); fflush(stdout); }
}

/* Read one line from stdin (strips trailing newline).
   Returns a heap-allocated string; caller frees with lpp_free_str. */
char *lpp_input(void) {
    char buf[4096];
    if (!fgets(buf, sizeof(buf), stdin)) return NULL;
    size_t len = strlen(buf);
    if (len > 0 && buf[len - 1] == '\n') buf[--len] = '\0';
    char *result = (char *)malloc(len + 1);
    if (!result) return NULL;
    memcpy(result, buf, len + 1);
    return result;
}

void lpp_free_str(char *ptr) {
    free(ptr);
}

int64_t lpp_parse_int(const char *str) {
    if (!str || *str == '\0') {
        fprintf(stderr, "[L++ Runtime Error] Invalid integer format: empty string\n");
        exit(1);
    }
    
    // Skip leading whitespace
    const char *p = str;
    while (*p == ' ' || *p == '\t' || *p == '\r' || *p == '\n') {
        p++;
    }
    
    if (*p == '\0') {
        fprintf(stderr, "[L++ Runtime Error] Invalid integer format: \"%s\"\n", str);
        exit(1);
    }
    
    char *endptr;
    errno = 0;
    long long val = strtoll(p, &endptr, 10);
    
    // Check for overflow/underflow
    if (errno == ERANGE) {
        fprintf(stderr, "[L++ Runtime Error] Integer overflow/underflow: \"%s\" exceeds 64-bit limits\n", str);
        exit(1);
    }
    
    // Check for trailing garbage (invalid chars)
    while (*endptr == ' ' || *endptr == '\t' || *endptr == '\r' || *endptr == '\n') {
        endptr++;
    }
    if (*endptr != '\0') {
        fprintf(stderr, "[L++ Runtime Error] Invalid integer format: \"%s\"\n", str);
        exit(1);
    }
    
    return (int64_t)val;
}

/* ── File I/O ─────────────────────────────────────────────────────────────── */

/* Read entire file contents. Returns heap-allocated string or NULL on error. */
char *lpp_read_file(const char *path) {
    FILE *f = fopen(path, "rb");
    if (!f) return NULL;
    fseek(f, 0, SEEK_END);
    long size = ftell(f);
    fseek(f, 0, SEEK_SET);
    char *buf = (char *)malloc(size + 1);
    if (!buf) { fclose(f); return NULL; }
    fread(buf, 1, size, f);
    buf[size] = '\0';
    fclose(f);
    return buf;
}

/* Write data to file. Returns 0 on success, -1 on error. */
int64_t lpp_write_file(const char *path, const char *data) {
    FILE *f = fopen(path, "wb");
    if (!f) return -1;
    size_t len = data ? strlen(data) : 0;
    fwrite(data, 1, len, f);
    fclose(f);
    return 0;
}

/* ── ARC (reference counting stubs) ──────────────────────────────────────── */

void lpp_arc_retain(void *ptr) {
    /* TODO: atomically increment refcount in ARC header before ptr */
    (void)ptr;
}

void lpp_arc_release(void *ptr) {
    /* TODO: atomically decrement; free when zero */
    (void)ptr;
}

/* ── Allocator ───────────────────────────────────────────────────────────── */

void *lpp_alloc(int64_t size) {
    return calloc(1, (size_t)size);
}

void lpp_free(void *ptr, int64_t size) {
    (void)size;
    free(ptr);
}

/* ── List<Int> ───────────────────────────────────────────────────────────── */

typedef struct {
    int64_t *data;
    int64_t  len;
    int64_t  cap;
} LppList;

void *lpp_list_new(void) {
    LppList *l = (LppList *)calloc(1, sizeof(LppList));
    return l;
}

void lpp_list_push(void *list, int64_t value) {
    LppList *l = (LppList *)list;
    if (!l) return;
    if (l->len == l->cap) {
        int64_t new_cap = l->cap == 0 ? 8 : l->cap * 2;
        l->data = (int64_t *)realloc(l->data, (size_t)(new_cap * sizeof(int64_t)));
        l->cap = new_cap;
    }
    l->data[l->len++] = value;
}

int64_t lpp_list_get(void *list, int64_t index) {
    LppList *l = (LppList *)list;
    if (!l || index < 0 || index >= l->len) return 0;
    return l->data[index];
}

int64_t lpp_list_len(void *list) {
    LppList *l = (LppList *)list;
    return l ? l->len : 0;
}

void lpp_list_free(void *list) {
    LppList *l = (LppList *)list;
    free(l->data);
    free(l);
}

/* ── Thread (minimal) ────────────────────────────────────────────────────── */

#if defined(_WIN32)
#include <windows.h>
typedef struct { void (*fn)(void*); void *env; } ThreadArg;
static DWORD WINAPI thread_trampoline(LPVOID arg) {
    ThreadArg *a = (ThreadArg *)arg;
    a->fn(a->env);
    free(a);
    return 0;
}
void lpp_thread_spawn(void (*fn)(void*), void *env) {
    ThreadArg *a = (ThreadArg *)malloc(sizeof(ThreadArg));
    a->fn = fn; a->env = env;
    CreateThread(NULL, 0, thread_trampoline, a, 0, NULL);
}
#else
#include <pthread.h>
typedef struct { void (*fn)(void*); void *env; } ThreadArg;
static void *thread_trampoline(void *arg) {
    ThreadArg *a = (ThreadArg *)arg;
    a->fn(a->env);
    free(a);
    return NULL;
}
void lpp_thread_spawn(void (*fn)(void*), void *env) {
    ThreadArg *a = (ThreadArg *)malloc(sizeof(ThreadArg));
    a->fn = fn; a->env = env;
    pthread_t t; pthread_create(&t, NULL, thread_trampoline, a);
    pthread_detach(t);
}
#endif

/* ── JSON Parser and Accessors (Builtin Standard Library) ────────────────── */

typedef struct lpp_JsonNode {
    char *key;
    int type; // 0=int, 1=str, 2=obj
    union {
        int64_t int_val;
        char *str_val;
        struct lpp_JsonNode *obj_val;
    } value;
    struct lpp_JsonNode *next;
} lpp_JsonNode;

static void skip_json_ws(const char **p) {
    while (**p == ' ' || **p == '\t' || **p == '\r' || **p == '\n') {
        (*p)++;
    }
}

static char *parse_json_string(const char **p) {
    skip_json_ws(p);
    if (**p != '"') return NULL;
    (*p)++; // skip '"'
    const char *start = *p;
    while (**p && **p != '"') {
        (*p)++;
    }
    size_t len = *p - start;
    char *res = malloc(len + 1);
    memcpy(res, start, len);
    res[len] = '\0';
    if (**p == '"') (*p)++; // skip '"'
    return res;
}

static lpp_JsonNode *parse_json_object(const char **p);

static lpp_JsonNode *parse_json_value(const char **p) {
    skip_json_ws(p);
    if (**p == '{') {
        return parse_json_object(p);
    } else if (**p == '"') {
        char *s = parse_json_string(p);
        lpp_JsonNode *n = calloc(1, sizeof(lpp_JsonNode));
        n->type = 1;
        n->value.str_val = s;
        return n;
    } else if ((**p >= '0' && **p <= '9') || **p == '-') {
        char *end;
        long long val = strtoll(*p, &end, 10);
        *p = end;
        lpp_JsonNode *n = calloc(1, sizeof(lpp_JsonNode));
        n->type = 0;
        n->value.int_val = (int64_t)val;
        return n;
    }
    return NULL;
}

static lpp_JsonNode *parse_json_object(const char **p) {
    skip_json_ws(p);
    if (**p != '{') return NULL;
    (*p)++; // skip '{'
    
    lpp_JsonNode *head = NULL;
    lpp_JsonNode *tail = NULL;
    
    while (**p && **p != '}') {
        skip_json_ws(p);
        if (**p == '}') break;
        char *key = parse_json_string(p);
        skip_json_ws(p);
        if (**p != ':') {
            free(key);
            break;
        }
        (*p)++; // skip ':'
        lpp_JsonNode *val = parse_json_value(p);
        if (val) {
            val->key = key;
            if (!head) {
                head = val;
                tail = val;
            } else {
                tail->next = val;
                tail = val;
            }
        } else {
            free(key);
        }
        skip_json_ws(p);
        if (**p == ',') {
            (*p)++; // skip ','
        } else if (**p != '}') {
            break;
        }
    }
    if (**p == '}') (*p)++; // skip '}'
    
    lpp_JsonNode *n = calloc(1, sizeof(lpp_JsonNode));
    n->type = 2;
    n->value.obj_val = head;
    return n;
}

void *lpp_json_parse(const char *str) {
    if (!str) return NULL;
    const char *p = str;
    return parse_json_value(&p);
}

int64_t lpp_json_get_int(void *json, const char *key) {
    lpp_JsonNode *node = (lpp_JsonNode *)json;
    if (!node) return 0;
    if (node->type == 2) {
        lpp_JsonNode *curr = node->value.obj_val;
        while (curr) {
            if (curr->key && strcmp(curr->key, key) == 0) {
                if (curr->type == 0) return curr->value.int_val;
                return 0;
            }
            curr = curr->next;
        }
    }
    return 0;
}

const char *lpp_json_get_str(void *json, const char *key) {
    lpp_JsonNode *node = (lpp_JsonNode *)json;
    if (!node) return "";
    if (node->type == 2) {
        lpp_JsonNode *curr = node->value.obj_val;
        while (curr) {
            if (curr->key && strcmp(curr->key, key) == 0) {
                if (curr->type == 1) return curr->value.str_val ? curr->value.str_val : "";
                return "";
            }
            curr = curr->next;
        }
    }
    return "";
}

void *lpp_json_get_obj(void *json, const char *key) {
    lpp_JsonNode *node = (lpp_JsonNode *)json;
    if (!node) return NULL;
    if (node->type == 2) {
        lpp_JsonNode *curr = node->value.obj_val;
        while (curr) {
            if (curr->key && strcmp(curr->key, key) == 0) {
                if (curr->type == 2) return curr;
                return NULL;
            }
            curr = curr->next;
        }
    }
    return NULL;
}

static void lpp_json_free_node(lpp_JsonNode *node) {
    if (!node) return;
    if (node->key) free(node->key);
    if (node->type == 1) {
        if (node->value.str_val) free(node->value.str_val);
    } else if (node->type == 2) {
        lpp_JsonNode *curr = node->value.obj_val;
        while (curr) {
            lpp_JsonNode *next = curr->next;
            lpp_json_free_node(curr);
            curr = next;
        }
    }
    free(node);
}

void lpp_json_free(void *json) {
    lpp_json_free_node((lpp_JsonNode *)json);
}
