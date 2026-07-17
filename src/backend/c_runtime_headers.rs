pub const C_BUILTINS_IO: &str = r#"
static char* lpp_input() {
    char buffer[1024];
    if (fgets(buffer, sizeof(buffer), stdin)) {
        buffer[strcspn(buffer, "\n")] = 0;
    } else {
        buffer[0] = 0;
    }
    char* res = malloc(strlen(buffer) + 1);
    strcpy(res, buffer);
    return res;
}

static char* lpp_read_file(const char* filename) {
    char* res = NULL;
    FILE* f = fopen(filename, "rb");
    if (f) {
        fseek(f, 0, SEEK_END);
        long fsize = ftell(f);
        fseek(f, 0, SEEK_SET);
        res = malloc(fsize + 1);
        size_t read_bytes = fread(res, 1, fsize, f);
        fclose(f);
        res[read_bytes] = 0;
    } else {
        res = malloc(1);
        res[0] = 0;
    }
    return res;
}

static int64_t lpp_write_file(const char* filename, const char* content) {
    FILE* f = fopen(filename, "wb");
    if (f) {
        fwrite(content, 1, strlen(content), f);
        fclose(f);
    }
    return 0;
}
"#;

pub const C_BUILTINS_JSON: &str = r#"
struct JsonNode {
    char *key;
    int type;
    union {
        int64_t int_val;
        char *str_val;
        struct JsonNode *obj_val;
    } value;
    struct JsonNode *next;
};

static void skip_json_ws(const char **p) {
    while (**p == ' ' || **p == '\t' || **p == '\r' || **p == '\n') {
        (*p)++;
    }
}

static char *parse_json_string(const char **p) {
    skip_json_ws(p);
    if (**p != '"') return NULL;
    (*p)++;
    const char *start = *p;
    while (**p && **p != '"') {
        (*p)++;
    }
    size_t len = *p - start;
    char *res = malloc(len + 1);
    memcpy(res, start, len);
    res[len] = '\0';
    if (**p == '"') (*p)++;
    return res;
}

static struct JsonNode *parse_json_object(const char **p);

static struct JsonNode *parse_json_value(const char **p) {
    skip_json_ws(p);
    if (**p == '{') {
        return parse_json_object(p);
    } else if (**p == '"') {
        char *s = parse_json_string(p);
        struct JsonNode *n = calloc(1, sizeof(struct JsonNode));
        n->type = 1;
        n->value.str_val = s;
        return n;
    } else if ((**p >= '0' && **p <= '9') || **p == '-') {
        char *end;
        long long val = strtoll(*p, &end, 10);
        *p = end;
        struct JsonNode *n = calloc(1, sizeof(struct JsonNode));
        n->type = 0;
        n->value.int_val = (int64_t)val;
        return n;
    }
    return NULL;
}

static struct JsonNode *parse_json_object(const char **p) {
    skip_json_ws(p);
    if (**p != '{') return NULL;
    (*p)++;
    struct JsonNode *head = NULL;
    struct JsonNode *tail = NULL;
    while (**p && **p != '}') {
        skip_json_ws(p);
        if (**p == '}') break;
        char *key = parse_json_string(p);
        skip_json_ws(p);
        if (**p != ':') {
            free(key);
            break;
        }
        (*p)++;
        struct JsonNode *val = parse_json_value(p);
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
            (*p)++;
        } else if (**p != '}') {
            break;
        }
    }
    if (**p == '}') (*p)++;
    struct JsonNode *n = calloc(1, sizeof(struct JsonNode));
    n->type = 2;
    n->value.obj_val = head;
    return n;
}

static int64_t json_parse(const char *str) {
    if (!str) return 0;
    const char *p = str;
    return (int64_t)parse_json_value(&p);
}

static int64_t json_get_int(int64_t json, const char *key) {
    struct JsonNode *node = (struct JsonNode *)json;
    if (!node) return 0;
    if (node->type == 2) {
        struct JsonNode *curr = node->value.obj_val;
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

static char* json_get_str(int64_t json, const char *key) {
    struct JsonNode *node = (struct JsonNode *)json;
    if (!node) return "";
    if (node->type == 2) {
        struct JsonNode *curr = node->value.obj_val;
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

static int64_t json_get_obj(int64_t json, const char *key) {
    struct JsonNode *node = (struct JsonNode *)json;
    if (!node) return 0;
    if (node->type == 2) {
        struct JsonNode *curr = node->value.obj_val;
        while (curr) {
            if (curr->key && strcmp(curr->key, key) == 0) {
                if (curr->type == 2) return (int64_t)curr;
                return 0;
            }
            curr = curr->next;
        }
    }
    return 0;
}

static void json_free_node(struct JsonNode *node) {
    if (!node) return;
    if (node->key) free(node->key);
    if (node->type == 1) {
        if (node->value.str_val) free(node->value.str_val);
    } else if (node->type == 2) {
        struct JsonNode *curr = node->value.obj_val;
        while (curr) {
            struct JsonNode *next = curr->next;
            json_free_node(curr);
            curr = next;
        }
    }
    free(node);
}

static void json_free(int64_t json) {
    json_free_node((struct JsonNode *)json);
}
"#;
