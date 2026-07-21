#if !defined(_WIN32) && !defined(_POSIX_C_SOURCE)
#  define _POSIX_C_SOURCE 200112L
#endif
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <limits.h>
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
typedef void (*LppArcDestructor)(void* payload);
typedef struct { lpp__arc_cnt_t rc; LppArcDestructor destructor; } LppArcHdr;
static void* lpp_arc_alloc_with_destructor(int64_t sz, LppArcDestructor destructor) {
    LppArcHdr* h = (LppArcHdr*)calloc(1, sizeof(LppArcHdr) + (size_t)sz);
    if (!h) return NULL;
#if defined(_MSC_VER)
    h->rc = 1;
#else
    atomic_init(&h->rc, 1);
#endif
    h->destructor = destructor;
    return (void*)(h + 1);
}
static void* lpp_arc_alloc(int64_t sz) {
    return lpp_arc_alloc_with_destructor(sz, NULL);
}
static void lpp_arc_retain(void* p) {
    if (!p) return;
    LppArcHdr* h = (LppArcHdr*)p - 1;
    LPP__ARC_INC(&h->rc);
}
static void lpp_arc_release(void* p) {
    if (!p) return;
    LppArcHdr* h = (LppArcHdr*)p - 1;
    if ((int)LPP__ARC_DEC(&h->rc) == 1) {
        if (h->destructor) h->destructor(p);
        free(h);
    }
}


/* ARC closure payload: [code pointer, owned environment pointer]. */
static void lpp_closure_destroy(void* closure) {
    if (!closure) return;
    void** parts = (void**)closure;
    lpp_arc_release(parts[1]);
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
    if (!filename || !content) return -1;
    FILE* f = fopen(filename, "wb");
    if (!f) return -1;
    size_t length = strlen(content);
    int failed = fwrite(content, 1, length, f) != length || fclose(f) != 0;
    return failed ? -1 : 0;
}

static int64_t lpp_file_size(const char* filename) {
    if (!filename) return -1;
    FILE* f = fopen(filename, "rb");
    if (!f) return -1;
    if (fseek(f, 0, SEEK_END) != 0) { fclose(f); return -1; }
    long size = ftell(f); fclose(f);
    return size < 0 ? -1 : (int64_t)size;
}

static int64_t lpp_file_copy(const char* source, const char* destination) {
    if (!source || !destination) return -1;
    FILE* in = fopen(source, "rb");
    if (!in) return -1;
    FILE* out = fopen(destination, "wb");
    if (!out) { fclose(in); return -1; }
    unsigned char buffer[8192]; int failed = 0;
    for (;;) {
        size_t got = fread(buffer, 1, sizeof(buffer), in);
        if (got && fwrite(buffer, 1, got, out) != got) { failed = 1; break; }
        if (got < sizeof(buffer)) { if (ferror(in)) failed = 1; break; }
    }
    if (fclose(in) != 0 || fclose(out) != 0) failed = 1;
    if (failed) { remove(destination); return -1; }
    return 0;
}

static int64_t lpp_file_move(const char* source, const char* destination) {
    return source && destination && rename(source, destination) == 0 ? 0 : -1;
}

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
    lpp_arc_retain((void *)(intptr_t)value);
}

static void lpp_list_arc_drop_element(int64_t value) {
    lpp_arc_release((void *)(intptr_t)value);
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

static void *lpp_list_new_with_ownership(
    LppListElementFn retain_element,
    LppListElementFn drop_element
) {
    LppList *l = (LppList *)lpp_arc_alloc_with_destructor(
        (int64_t)sizeof(LppList), lpp_list_destroy
    );
    if (!l) {
        fprintf(stderr, "[L++ Runtime Error] out of memory while creating list\n");
        abort();
    }
    l->retain_element = retain_element;
    l->drop_element = drop_element;
    return l;
}

/* List[Int] stores values and owns no element references. */
static void* lpp_list_new(void) {
    return lpp_list_new_with_ownership(NULL, NULL);
}

/* List[ARC Object] owns one retained reference per element. */
static void* lpp_list_new_arc(void) {
    return lpp_list_new_with_ownership(
        lpp_list_arc_retain_element,
        lpp_list_arc_drop_element
    );
}

static void lpp_list_push(void *list, int64_t value) {
    LppList *l = (LppList *)list;
    if (!l) {
        fprintf(stderr, "[L++ Runtime Error] push to null list\n");
        abort();
    }
    if (l->len == l->cap) {
        if (l->cap > INT64_MAX / 2) {
            fprintf(stderr, "[L++ Runtime Error] list capacity overflow\n");
            abort();
        }
        int64_t new_cap = l->cap == 0 ? 8 : l->cap * 2;
        if (new_cap > INT64_MAX / (int64_t)sizeof(int64_t)) {
            fprintf(stderr, "[L++ Runtime Error] list allocation size overflow\n");
            abort();
        }
        int64_t *new_data = (int64_t *)realloc(l->data, (size_t)new_cap * sizeof(int64_t));
        if (!new_data) {
            fprintf(stderr, "[L++ Runtime Error] out of memory while growing list\n");
            abort();
        }
        l->data = new_data;
        l->cap = new_cap;
    }
    if (l->retain_element) l->retain_element(value);
    l->data[l->len++] = value;
}

/* Store one ARC object reference in List[T]. */
static void lpp_list_push_arc(void *list, void *value) {
    lpp_list_push(list, (int64_t)(intptr_t)value);
}

static int64_t lpp_list_get(void *list, int64_t index) {
    LppList *l = (LppList *)list;
    if (!l || index < 0 || index >= l->len) {
        fprintf(stderr, "[L++ Runtime Error] list index out of bounds: %lld\n", (long long)index);
        abort();
    }
    return l->data[index];
}

/* List element reads are borrowed; callers retain only when they create an
 * additional owner (assignment/return/store). */
static void* lpp_list_get_arc(void *list, int64_t index) {
    return (void *)(intptr_t)lpp_list_get(list, index);
}

static int64_t lpp_list_len(void *list) {
    LppList *l = (LppList *)list;
    return l ? l->len : 0;
}

static void lpp_list_free(void *list) {
    /* Compatibility entry point. In ownership-aware AOT code list lifetime is
     * automatic, so this is only a single reference release, never raw free. */
    lpp_arc_release(list);
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
#  include <sys/time.h>
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

/* A successful OS send may be a partial write. This helper completes the
 * UTF-8 payload so protocol callers never mistake a prefix for a request. */
static int64_t lpp_net_send_all(int64_t handle, const char* data) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET || !data) return -1;
    size_t length = strlen(data);
    size_t sent_total = 0;
    while (sent_total < length) {
        size_t remaining = length - sent_total;
#ifdef _WIN32
        int chunk = remaining > (size_t)INT_MAX ? INT_MAX : (int)remaining;
        int sent = send(sock, data + sent_total, chunk, 0);
#else
        int flags = 0;
# ifdef MSG_NOSIGNAL
        flags |= MSG_NOSIGNAL;
# endif
        ssize_t sent = send(sock, data + sent_total, remaining, flags);
#endif
        if (sent <= 0) return -1;
        sent_total += (size_t)sent;
    }
    return (int64_t)sent_total;
}

static int64_t lpp_net_send(int64_t handle, const char* data) {
    return lpp_net_send_all(handle, data);
}

static int64_t lpp_net_set_timeout(int64_t handle, int64_t milliseconds) {
    lpp_socket_t sock = lpp__socket_load(handle);
    if (sock == LPP_INVALID_SOCKET || milliseconds <= 0) return 0;
#ifdef _WIN32
    DWORD timeout = milliseconds > 0xFFFFFFFFLL ? (DWORD)0xFFFFFFFFUL : (DWORD)milliseconds;
    return setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, (const char*)&timeout, sizeof(timeout)) == 0
        && setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, (const char*)&timeout, sizeof(timeout)) == 0;
#else
    struct timeval timeout;
    timeout.tv_sec = (time_t)(milliseconds / 1000);
    timeout.tv_usec = (suseconds_t)((milliseconds % 1000) * 1000);
    return setsockopt(sock, SOL_SOCKET, SO_RCVTIMEO, &timeout, sizeof(timeout)) == 0
        && setsockopt(sock, SOL_SOCKET, SO_SNDTIMEO, &timeout, sizeof(timeout)) == 0;
#endif
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
typedef struct Task Task_t;
struct Task {
    int64_t id;
    char* title;
    int completed;
};

static void lpp_drop_Task(void* raw) {
    Task_t* self = (Task_t*)raw;
}

void lpp_main(void);
Task_t* create_task(int64_t id, char* title);
void show_task(Task_t* t);

void lpp_main(void) {
    lpp_print_str("=== Welcome to the L++ Task Manager ===");
    lpp_print_str("This is a multi-file project demo running on native Cranelift.");
    /* Storage: Arc */
    void* tasks_4 = lpp_list_new();
    /* Storage: Value */
    Task_t* t1_5 = create_task(101, "Review Compiler Architecture");
    /* Storage: Value */
    Task_t* t2_6 = create_task(102, "Optimize Memory Dataflow ARC");
    t1_5->completed = 1;
    lpp_list_push(tasks_4, t1_5);
    lpp_list_push(tasks_4, t2_6);
    /* Storage: Value */
    int64_t total_7 = lpp_list_len(tasks_4);
    lpp_print_str("Total tasks registered:");
    printf("%lld\n", (long long)(total_7));
    lpp_print_str("Listing all tasks:");
    /* Storage: Arc */
    void* __for_list_0_8 = tasks_4;
    /* Storage: Value */
    int64_t __for_idx_0_9 = 0;
    while ((__for_idx_0_9 < lpp_list_len(__for_list_0_8))) {
        /* Storage: Value */
        int64_t t_10 = lpp_list_get(__for_list_0_8, __for_idx_0_9);
        show_task(t_10);
        __for_idx_0_9 = (__for_idx_0_9 + 1);
    }
    lpp_print_str("Writing summary to tasks_log.txt...");
    /* Storage: Value */
    int64_t log_res_11 = lpp_write_file("tasks_log.txt", "L++ Task Verification Log
Status: Complete
");
    if ((log_res_11 == 0)) {
        lpp_print_str("File written successfully!");
        lpp_print_str("Reading log contents:");
        /* Storage: Value */
        char* contents_12 = lpp_read_file("tasks_log.txt");
        lpp_print_str(contents_12);
        lpp_delete_file("tasks_log.txt");
    } else {
        lpp_print_str("Error writing log file.");
    }
    lpp_print_str("All tests passed successfully!");
}

Task_t* create_task(int64_t id_13, char* title_14) {
    /* Storage: Arc */
    Task_t* t_15 = (Task_t*)lpp_arc_alloc_with_destructor(sizeof(Task_t), lpp_drop_Task);
    t_15->id = id_13;
    t_15->title = title_14;
    t_15->completed = 0;
    return t_15;
}

void show_task(Task_t* t_16) {
    lpp_print_str("--- Task Detail ---");
    lpp_print_str("ID:");
    printf("%lld\n", (long long)(t_16->id));
    lpp_print_str("Title:");
    lpp_print_str(t_16->title);
    lpp_print_str("Status:");
    if (t_16->completed) {
        lpp_print_str("Completed");
    } else {
        lpp_print_str("Pending");
    }
    lpp_print_str("-------------------");
}

int main() {
    lpp_main();
    return 0;
}
