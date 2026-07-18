#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>
#if defined(_MSC_VER)
#  include <malloc.h>
#else
#  include <alloca.h>
#endif


/* ── ARC (Automatic Reference Counting) ─────────────────────────────────── */
#if defined(_MSC_VER)
#  ifndef WIN32_LEAN_AND_MEAN
#    define WIN32_LEAN_AND_MEAN
#  endif
#  include <windows.h>
   typedef volatile LONG lpp__arc_cnt_t;
#  define LPP__ARC_INC(p) InterlockedIncrement((p))
#  define LPP__ARC_DEC(p) InterlockedDecrement((p))
#else
#  include <stdatomic.h>
   typedef _Atomic(int) lpp__arc_cnt_t;
#  define LPP__ARC_INC(p) atomic_fetch_add_explicit((p),  1, __ATOMIC_ACQ_REL)
#  define LPP__ARC_DEC(p) atomic_fetch_sub_explicit((p),  1, __ATOMIC_ACQ_REL)
#endif
typedef struct { lpp__arc_cnt_t rc; } LppArcHdr;
static void* lpp_arc_alloc(int64_t sz) {
    LppArcHdr* h = (LppArcHdr*)calloc(1, sizeof(LppArcHdr) + (size_t)sz);
    if (!h) return NULL;
#if defined(_MSC_VER)
    h->rc = 1;
#else
    atomic_init(&h->rc, 1);
#endif
    return (void*)(h + 1);
}
static void lpp_arc_retain(void* p) {
    if (!p) return;
    LppArcHdr* h = (LppArcHdr*)p - 1;
    LPP__ARC_INC(&h->rc);
}
static void lpp_arc_release(void* p) {
    if (!p) return;
    LppArcHdr* h = (LppArcHdr*)p - 1;
    if ((int)LPP__ARC_DEC(&h->rc) == 1) free(h);
}


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

typedef struct {
    int64_t *data;
    int64_t  len;
    int64_t  cap;
} LppList;

static void* lpp_list_new(void) {
    LppList *l = (LppList *)calloc(1, sizeof(LppList));
    return l;
}

static void lpp_list_push(void *list, int64_t value) {
    LppList *l = (LppList *)list;
    if (!l) return;
    if (l->len == l->cap) {
        int64_t new_cap = l->cap == 0 ? 8 : l->cap * 2;
        l->data = (int64_t *)realloc(l->data, (size_t)(new_cap * sizeof(int64_t)));
        l->cap = new_cap;
    }
    l->data[l->len++] = value;
}

static int64_t lpp_list_get(void *list, int64_t index) {
    LppList *l = (LppList *)list;
    if (!l || index < 0 || index >= l->len) return 0;
    return l->data[index];
}

static int64_t lpp_list_len(void *list) {
    LppList *l = (LppList *)list;
    return l ? l->len : 0;
}

static void lpp_list_free(void *list) {
    LppList *l = (LppList *)list;
    if (!l) return;
    if (l->data) free(l->data);
    free(l);
}

#if defined(_WIN32)
#  if defined(_MSC_VER)
#    pragma comment(lib, "Ws2_32.lib")
#  endif
#  include <winsock2.h>
#  include <ws2tcpip.h>
typedef SOCKET lpp_socket_t;
#  define LPP_INVALID_SOCKET INVALID_SOCKET
#  define lpp_close_socket closesocket
static int lpp__net_started = 0;
static void lpp__net_init(void) {
    if (!lpp__net_started) {
        WSADATA wsa;
        if (WSAStartup(MAKEWORD(2, 2), &wsa) == 0) lpp__net_started = 1;
    }
}
#else
#  include <sys/types.h>
#  include <sys/socket.h>
#  include <netdb.h>
#  include <arpa/inet.h>
#  include <unistd.h>
typedef int lpp_socket_t;
#  define LPP_INVALID_SOCKET (-1)
#  define lpp_close_socket close
static void lpp__net_init(void) {}
#endif

static lpp_socket_t lpp__socket_table[256];

static int64_t lpp__socket_store(lpp_socket_t sock) {
    for (int64_t i = 0; i < 256; ++i) {
        if (lpp__socket_table[i] == 0 || lpp__socket_table[i] == LPP_INVALID_SOCKET) {
            lpp__socket_table[i] = sock;
            return i + 1;
        }
    }
    return 0;
}

static lpp_socket_t lpp__socket_load(int64_t handle) {
    if (handle <= 0 || handle > 256) return LPP_INVALID_SOCKET;
    return lpp__socket_table[handle - 1];
}

static void lpp__socket_clear(int64_t handle) {
    if (handle > 0 && handle <= 256) lpp__socket_table[handle - 1] = 0;
}

static int64_t lpp_net_connect(const char* host, int64_t port) {
    lpp__net_init();
    if (!host) return 0;
    char port_buf[32];
    snprintf(port_buf, sizeof(port_buf), "%lld", (long long)port);
    struct addrinfo hints, *result = NULL, *rp = NULL;
    memset(&hints, 0, sizeof(hints));
    hints.ai_family = AF_UNSPEC;
    hints.ai_socktype = SOCK_STREAM;
    if (getaddrinfo(host, port_buf, &hints, &result) != 0) return 0;
    lpp_socket_t sock = LPP_INVALID_SOCKET;
    for (rp = result; rp; rp = rp->ai_next) {
        sock = (lpp_socket_t)socket(rp->ai_family, rp->ai_socktype, rp->ai_protocol);
        if (sock == LPP_INVALID_SOCKET) continue;
        if (connect(sock, rp->ai_addr, (int)rp->ai_addrlen) == 0) break;
        lpp_close_socket(sock);
        sock = LPP_INVALID_SOCKET;
    }
    freeaddrinfo(result);
    if (sock == LPP_INVALID_SOCKET) return 0;
    return lpp__socket_store(sock);
}

static int64_t lpp_net_listen(int64_t port) {
    lpp__net_init();
    lpp_socket_t sock = (lpp_socket_t)socket(AF_INET, SOCK_STREAM, 0);
    if (sock == LPP_INVALID_SOCKET) return 0;
    int yes = 1;
    setsockopt(sock, SOL_SOCKET, SO_REUSEADDR, (const char*)&yes, sizeof(yes));
    struct sockaddr_in addr;
    memset(&addr, 0, sizeof(addr));
    addr.sin_family = AF_INET;
    addr.sin_addr.s_addr = htonl(INADDR_ANY);
    addr.sin_port = htons((unsigned short)port);
    if (bind(sock, (struct sockaddr*)&addr, sizeof(addr)) != 0 || listen(sock, 16) != 0) {
        lpp_close_socket(sock);
        return 0;
    }
    return lpp__socket_store(sock);
}

static int64_t lpp_net_accept(int64_t listener) {
    lpp_socket_t server = lpp__socket_load(listener);
    if (server == LPP_INVALID_SOCKET) return 0;
    lpp_socket_t client = accept(server, NULL, NULL);
    if (client == LPP_INVALID_SOCKET) return 0;
    return lpp__socket_store(client);
}

static int64_t lpp_net_send(int64_t handle, const char* data) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET || !data) return -1;
    return (int64_t)send(sock, data, (int)strlen(data), 0);
}

static char* lpp_net_recv(int64_t handle, int64_t max_bytes) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET || max_bytes <= 0) {
        char* empty = (char*)malloc(1);
        if (empty) empty[0] = 0;
        return empty;
    }
    int size = (int)max_bytes;
    char* buf = (char*)malloc((size_t)size + 1);
    if (!buf) return NULL;
    int received = recv(sock, buf, size, 0);
    if (received <= 0) {
        buf[0] = 0;
        return buf;
    }
    buf[received] = 0;
    return buf;
}

static void lpp_net_close(int64_t handle) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET) return;
    lpp_close_socket(sock);
    lpp__socket_clear(handle);
}

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
int64_t mul(int64_t a, int64_t b);
int64_t factorial(int64_t n);
void lpp_main(void);

int64_t mul(int64_t a_3, int64_t b_4) {
    return (a_3 * b_4);
}

int64_t factorial(int64_t n_5) {
    if ((n_5 <= 1)) {
        return 1;
    }
    return mul_0(n_5, factorial_1((n_5 - 1)));
}

void lpp_main(void) {
    printf("%lld\n", (long long)(factorial_1(5)));
}

int main() {
    lpp_main();
    return 0;
}
