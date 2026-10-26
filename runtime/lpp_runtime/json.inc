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
    /* Returned to L++ via lpp_json_get_str, so it needs a real ARC header. */
    char *res = (char *)lpp_arc_alloc((int64_t)(len + 1));
    if (!res) return NULL;
    memcpy(res, start, len);
    res[len] = '\0';
    if (**p == '"') (*p)++; // skip '"'
    return res;
}

/* Recursion guard: parse_json_value/object recurse once per nesting level, so
 * an attacker-supplied document with unbounded `{{{{...` nesting would smash
 * the stack. Refuse to descend past a sane bound instead of crashing. */
#define LPP_JSON_MAX_DEPTH 256

static lpp_JsonNode *parse_json_object(const char **p, int depth);

static lpp_JsonNode *parse_json_value(const char **p, int depth) {
    if (depth > LPP_JSON_MAX_DEPTH) return NULL;
    skip_json_ws(p);
    if (**p == '{') {
        return parse_json_object(p, depth + 1);
    } else if (**p == '"') {
        char *s = parse_json_string(p);
        lpp_JsonNode *n = calloc(1, sizeof(lpp_JsonNode));
        if (!n) { if (s) lpp_arc_release(s); return NULL; }
        n->type = 1;
        n->value.str_val = s;
        return n;
    } else if ((**p >= '0' && **p <= '9') || **p == '-') {
        char *end;
        long long val = strtoll(*p, &end, 10);
        *p = end;
        lpp_JsonNode *n = calloc(1, sizeof(lpp_JsonNode));
        if (!n) return NULL;
        n->type = 0;
        n->value.int_val = (int64_t)val;
        return n;
    }
    return NULL;
}

static lpp_JsonNode *parse_json_object(const char **p, int depth) {
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
            lpp_arc_release(key);   /* key is an ARC-owned string */
            break;
        }
        (*p)++; // skip ':'
        lpp_JsonNode *val = parse_json_value(p, depth + 1);
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
            lpp_arc_release(key);   /* key is an ARC-owned string */
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
    if (!n) return NULL;
    n->type = 2;
    n->value.obj_val = head;
    return n;
}

void *lpp_json_parse(const char *str) {
    if (!str) return NULL;
    const char *p = str;
    return parse_json_value(&p, 0);
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
    if (!node) return lpp_empty_str();
    if (node->type == 2) {
        lpp_JsonNode *curr = node->value.obj_val;
        while (curr) {
            if (curr->key && strcmp(curr->key, key) == 0) {
                if (curr->type == 1) return curr->value.str_val ? curr->value.str_val : lpp_empty_str();
                return lpp_empty_str();
            }
            curr = curr->next;
        }
    }
    return lpp_empty_str();
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
    /* Keys are ARC-owned strings (allocated via lpp_arc_alloc in
     * parse_json_string), so they must be reclaimed with lpp_arc_release —
     * raw free() on the payload address bypasses its ARC header. The node
     * struct itself is plain calloc, so free(node) below is correct. */
    if (node->key) lpp_arc_release(node->key);
    if (node->type == 1) {
        if (node->value.str_val) lpp_arc_release(node->value.str_val);
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

#if (defined(__x86_64__) || defined(_M_X64) || defined(__i386__) || defined(_M_IX86)) && !defined(__aarch64__) && !defined(_M_ARM64) && !defined(__arm64__)
#define LPP_HAS_X86_SSE2 1
#include <emmintrin.h>
#include <xmmintrin.h>
#endif

#include "../lpp_str.c"
#define LPP_EXEC_EXCLUDE_BUILTINS
#include "../lpp_exec.c"
#undef LPP_EXEC_EXCLUDE_BUILTINS
#include "../lpp_dir.c"
#include "../lpp_buf.c"
#include "../lpp_map.c"
#include "../lpp_gui.c"
#include "../lpp_webview.c"
#include "../lpp_int.c"
#include "../lpp_atomic.c"
#include "../lpp_concur.c"
#include "../lpp_clock_rng.c"


