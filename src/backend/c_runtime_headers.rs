pub const C_BUILTINS_IO: &str = r#"
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

static void lpp_print_int(int64_t value) {
    printf("%lld\n", (long long)value);
    fflush(stdout);
}

static void lpp_print_float(double value) {
    printf("%f\n", value);
    fflush(stdout);
}

static void lpp_print_str(const char *ptr) {
    if (ptr) { puts(ptr); fflush(stdout); }
}

static void lpp_free_str(char *ptr) {
    free(ptr);
}

static int64_t lpp_parse_int(const char *str) {
    if (!str || *str == '\0') return 0;
    const char *p = str;
    while (*p == ' ' || *p == '\t' || *p == '\r' || *p == '\n') p++;
    if (*p == '\0') return 0;
    char *endptr;
    long long val = strtoll(p, &endptr, 10);
    return (int64_t)val;
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

static void lpp_list_push_float(void *list, double value) {
    int64_t ival;
    memcpy(&ival, &value, sizeof(double));
    lpp_list_push(list, ival);
}

static int64_t lpp_list_get(void *list, int64_t index) {
    LppList *l = (LppList *)list;
    if (!l || index < 0 || index >= l->len) {
        fprintf(stderr, "[L++ Runtime Error] list index out of bounds: %lld\n", (long long)index);
        abort();
    }
    return l->data[index];
}

static double lpp_list_get_float(void *list, int64_t index) {
    int64_t ival = lpp_list_get(list, index);
    double fval;
    memcpy(&fval, &ival, sizeof(double));
    return fval;
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

/* ── String library (runtime/lpp_str.c) ───────────────────────────────── */
static char *lpp_str_concat(const char *a, const char *b) {
    if (!a) a = "";
    if (!b) b = "";
    size_t la = strlen(a), lb = strlen(b);
    char *out = (char *)lpp_arc_alloc((int64_t)(la + lb + 1));
    if (!out) return (char *)"";
    memcpy(out, a, la);
    memcpy(out + la, b, lb);
    out[la + lb] = 0;
    return out;
}

static void *lpp_str_split(const char *s, int64_t delim) {
    void *list = lpp_list_new_arc();
    if (!list) return 0;
    if (!s || !*s) return list;

    char ch = (char)delim;
    const char *start = s;

    for (;;) {
        if (*s == ch || *s == 0) {
            int64_t len = (int64_t)(s - start);
            char *piece = (char *)lpp_arc_alloc(len + 1);
            if (piece) {
                memcpy(piece, start, (size_t)len);
                piece[len] = 0;
                lpp_list_push_arc(list, piece);
                lpp_arc_release(piece);
            }
            if (*s == 0) break;
            start = s + 1;
        }
        s++;
    }
    return list;
}

static int64_t lpp_str_find(const char *haystack, const char *needle) {
    if (!haystack || !needle) return -1;
    const char *found = strstr(haystack, needle);
    if (!found) return -1;
    return (int64_t)(found - haystack);
}

static char *lpp_str_replace(const char *s, const char *old, const char *new_) {
    if (!s) s = "";
    if (!old || !*old) return (char *)s;
    if (!new_) new_ = "";

    size_t slen = strlen(s), olen = strlen(old), nlen = strlen(new_);
    int64_t count = 0;
    const char *scan = s;
    while ((scan = strstr(scan, old))) { count++; scan += olen; }

    size_t outlen = slen + (size_t)count * (nlen - olen) + 1;
    char *out = (char *)lpp_arc_alloc((int64_t)outlen);
    if (!out) return (char *)"";

    char *dst = out;
    const char *src = s;
    while (*src) {
        const char *next = strstr(src, old);
        if (!next) { strcpy(dst, src); break; }
        size_t prefix = (size_t)(next - src);
        memcpy(dst, src, prefix); dst += prefix;
        memcpy(dst, new_, nlen);   dst += nlen;
        src = next + olen;
    }
    return out;
}

static char *lpp_str_substr(const char *s, int64_t start, int64_t length) {
    if (!s) s = "";
    size_t slen = strlen(s);
    if (start < 0) start = 0;
    if (start > (int64_t)slen) return (char *)"";

    size_t remain = slen - (size_t)start;
    size_t copy = (length < 0 || (size_t)length > remain) ? remain : (size_t)length;

    char *out = (char *)lpp_arc_alloc((int64_t)(copy + 1));
    if (!out) return (char *)"";
    memcpy(out, s + start, copy);
    out[copy] = 0;
    return out;
}

static char *lpp_str_trim(const char *s) {
    if (!s) return (char *)"";
    while (*s == ' ' || *s == '\t' || *s == '\n' || *s == '\r') s++;
    const char *end = s + strlen(s);
    while (end > s && (end[-1] == ' ' || end[-1] == '\t' || end[-1] == '\n' || end[-1] == '\r'))
        end--;

    int64_t len = (int64_t)(end - s);
    char *out = (char *)lpp_arc_alloc(len + 1);
    if (!out) return (char *)"";
    memcpy(out, s, (size_t)len);
    out[len] = 0;
    return out;
}

/* ── Process execution (runtime/lpp_exec.c) ────────────────────────────── */
#if defined(_WIN32)
static int64_t lpp_command_exec(const char *cmdline) {
    if (!cmdline) return -1;
    STARTUPINFOA si = {sizeof(si)};
    PROCESS_INFORMATION pi = {0};
    si.dwFlags = STARTF_USESTDHANDLES;
    char *dup = malloc(strlen(cmdline) + 1); if (dup) strcpy(dup, cmdline);
    if (!dup) return -1;
    BOOL ok = CreateProcessA(NULL, dup, NULL, NULL, FALSE,
                              CREATE_NO_WINDOW, NULL, NULL, &si, &pi);
    free(dup);
    if (!ok) return -1;
    WaitForSingleObject(pi.hProcess, INFINITE);
    DWORD code;
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);
    return (int64_t)(int)code;
}

static char *lpp_command_output(const char *cmdline) {
    if (!cmdline) return (char *)"";
    HANDLE hRead, hWrite;
    SECURITY_ATTRIBUTES sa = {sizeof(sa), NULL, TRUE};
    if (!CreatePipe(&hRead, &hWrite, &sa, 0)) return (char *)"";

    STARTUPINFOA si = {sizeof(si)};
    PROCESS_INFORMATION pi = {0};
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdOutput = hWrite;
    si.hStdError  = hWrite;

    char *dup = malloc(strlen(cmdline) + 1); if (dup) strcpy(dup, cmdline);
    BOOL ok = CreateProcessA(NULL, dup, NULL, NULL, TRUE,
                              CREATE_NO_WINDOW, NULL, NULL, &si, &pi);
    free(dup);
    CloseHandle(hWrite);
    if (!ok) { CloseHandle(hRead); return (char *)""; }

    WaitForSingleObject(pi.hProcess, INFINITE);
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);

    int cap = 4096, len = 0;
    char *buf = (char *)lpp_arc_alloc((int64_t)(cap + 1));
    if (!buf) { CloseHandle(hRead); return (char *)""; }
    for (;;) {
        if (len + 1024 >= cap) {
            int nc = cap * 2;
            char *nb = (char *)lpp_arc_alloc((int64_t)(nc + 1));
            if (!nb) break;
            memcpy(nb, buf, (size_t)len);
            lpp_arc_release(buf);
            buf = nb; cap = nc;
        }
        DWORD n;
        if (!ReadFile(hRead, buf + len, (DWORD)(cap - len), &n, NULL) || n == 0) break;
        len += (int)n;
    }
    CloseHandle(hRead);
    buf[len] = 0;
    return buf;
}

static char *lpp_env_get(const char *name) {
    if (!name) return (char *)"";
    char val[4096];
    DWORD n = GetEnvironmentVariableA(name, val, sizeof(val));
    if (n == 0 || n >= sizeof(val)) return (char *)"";
    char *out = (char *)lpp_arc_alloc((int64_t)(n + 1));
    if (!out) return (char *)"";
    memcpy(out, val, n);
    out[n] = 0;
    return out;
}

static int64_t lpp_env_set(const char *name, const char *value) {
    if (!name) return -1;
    return SetEnvironmentVariableA(name, value ? value : "") ? 0 : -1;
}
#else
#include <sys/wait.h>
#include <unistd.h>
#include <spawn.h>

extern char **environ;

static int64_t lpp_command_exec(const char *cmdline) {
    if (!cmdline) return -1;
    pid_t pid;
    char *sh = "/bin/sh";
    char *argv[] = {sh, (char *)"-c", (char *)cmdline, NULL};
    int status = posix_spawn(&pid, sh, NULL, NULL, argv, environ);
    if (status != 0) return -1;
    waitpid(pid, &status, 0);
    return WIFEXITED(status) ? (int64_t)WEXITSTATUS(status) : -1;
}

static char *lpp_command_output(const char *cmdline) {
    if (!cmdline) return (char *)"";
    int pipefd[2];
    if (pipe(pipefd) < 0) return (char *)"";

    pid_t pid = fork();
    if (pid < 0) { close(pipefd[0]); close(pipefd[1]); return (char *)""; }

    if (pid == 0) {
        close(pipefd[0]);
        dup2(pipefd[1], STDOUT_FILENO);
        dup2(pipefd[1], STDERR_FILENO);
        close(pipefd[1]);
        execl("/bin/sh", "sh", "-c", cmdline, (char *)NULL);
        _exit(127);
    }

    close(pipefd[1]);
    int cap = 4096, len = 0;
    char *buf = (char *)lpp_arc_alloc((int64_t)(cap + 1));
    if (!buf) { close(pipefd[0]); waitpid(pid, NULL, 0); return (char *)""; }

    for (;;) {
        if (len + 1024 >= cap) {
            int nc = cap * 2;
            char *nb = (char *)lpp_arc_alloc((int64_t)(nc + 1));
            if (!nb) break;
            memcpy(nb, buf, (size_t)len);
            lpp_arc_release(buf);
            buf = nb; cap = nc;
        }
        ssize_t n = read(pipefd[0], buf + len, (size_t)(cap - len));
        if (n <= 0) break;
        len += (int)n;
    }
    close(pipefd[0]);
    waitpid(pid, NULL, 0);
    buf[len] = 0;
    return buf;
}

static char *lpp_env_get(const char *name) {
    if (!name) return (char *)"";
    const char *val = getenv(name);
    if (!val) return (char *)"";
    int64_t len = (int64_t)strlen(val);
    char *out = (char *)lpp_arc_alloc(len + 1);
    if (!out) return (char *)"";
    memcpy(out, val, (size_t)len);
    out[len] = 0;
    return out;
}

static int64_t lpp_env_set(const char *name, const char *value) {
    if (!name) return -1;
    return setenv(name, value ? value : "", 1) == 0 ? 0 : -1;
}
#endif

/* ── Directory / filesystem (runtime/lpp_dir.c) ────────────────────────── */
#if defined(_WIN32)
static int64_t lpp_dir_create(const char *path) {
    if (!path) return -1;
    return CreateDirectoryA(path, NULL) ? 0 : -1;
}

static void *lpp_dir_list(const char *path) {
    void *list = lpp_list_new_arc();
    if (!list) return 0;
    if (!path) return list;

    char pattern[MAX_PATH];
    snprintf(pattern, sizeof(pattern), "%s\\*", path);
    WIN32_FIND_DATAA fd;
    HANDLE h = FindFirstFileA(pattern, &fd);
    if (h == INVALID_HANDLE_VALUE) return list;

    do {
        if (strcmp(fd.cFileName, ".") == 0 || strcmp(fd.cFileName, "..") == 0)
            continue;
        size_t len = strlen(fd.cFileName);
        char *copy = (char *)lpp_arc_alloc((int64_t)(len + 1));
        if (copy) { memcpy(copy, fd.cFileName, len); copy[len] = 0;
                    lpp_list_push_arc(list, copy); lpp_arc_release(copy); }
    } while (FindNextFileA(h, &fd));
    FindClose(h);
    return list;
}

static int64_t lpp_dir_remove(const char *path) {
    if (!path) return -1;
    char cmd[MAX_PATH + 32];
    snprintf(cmd, sizeof(cmd), "rmdir /s /q \"%s\"", path);
    return system(cmd) == 0 ? 0 : -1;
}

static int64_t lpp_path_exists(const char *path) {
    if (!path) return 0;
    DWORD attr = GetFileAttributesA(path);
    return (attr != INVALID_FILE_ATTRIBUTES) ? 1 : 0;
}

static char *lpp_path_join(const char *base, const char *child) {
    if (!base) base = "";
    if (!child) child = "";
    size_t blen = strlen(base), clen = strlen(child);
    int need_sep = (blen > 0 && base[blen - 1] != '\\' && base[blen - 1] != '/');
    int64_t total = (int64_t)(blen + (need_sep ? 1 : 0) + clen + 1);
    char *out = (char *)lpp_arc_alloc(total);
    if (!out) return (char *)"";
    memcpy(out, base, blen);
    size_t off = blen;
    if (need_sep) out[off++] = '\\';
    memcpy(out + off, child, clen);
    out[off + clen] = 0;
    return out;
}
#else
#include <sys/stat.h>
#include <dirent.h>

static int64_t lpp_dir_create(const char *path) {
    if (!path) return -1;
    return mkdir(path, 0755) == 0 ? 0 : -1;
}

static void *lpp_dir_list(const char *path) {
    void *list = lpp_list_new_arc();
    if (!list) return 0;
    if (!path) return list;

    DIR *d = opendir(path);
    if (!d) return list;

    struct dirent *entry;
    while ((entry = readdir(d)) != NULL) {
        if (strcmp(entry->d_name, ".") == 0 || strcmp(entry->d_name, "..") == 0)
            continue;
        size_t len = strlen(entry->d_name);
        char *copy = (char *)lpp_arc_alloc((int64_t)(len + 1));
        if (copy) { memcpy(copy, entry->d_name, len); copy[len] = 0;
                    lpp_list_push_arc(list, copy); lpp_arc_release(copy); }
    }
    closedir(d);
    return list;
}

static int64_t lpp_dir_remove(const char *path) {
    if (!path) return -1;
    char cmd[4096];
    snprintf(cmd, sizeof(cmd), "rm -rf \"%s\"", path);
    return system(cmd) == 0 ? 0 : -1;
}

static int64_t lpp_path_exists(const char *path) {
    if (!path) return 0;
    struct stat st;
    return stat(path, &st) == 0 ? 1 : 0;
}

static char *lpp_path_join(const char *base, const char *child) {
    if (!base) base = "";
    if (!child) child = "";
    size_t blen = strlen(base), clen = strlen(child);
    int need_sep = (blen > 0 && base[blen - 1] != '/');
    int64_t total = (int64_t)(blen + (need_sep ? 1 : 0) + clen + 1);
    char *out = (char *)lpp_arc_alloc(total);
    if (!out) return (char *)"";
    memcpy(out, base, blen);
    size_t off = blen;
    if (need_sep) out[off++] = '/';
    memcpy(out + off, child, clen);
    out[off + clen] = 0;
    return out;
}
#endif

/* ── Binary buffer library (runtime/lpp_buf.c) ────────────────────────── */
static int64_t lpp_buf_alloc(int64_t size) {
    if (size < 0) return 0;
    uint8_t *buf = (uint8_t *)calloc(1, (size_t)(8 + size));
    if (!buf) return 0;
    *(int64_t *)buf = size;
    return (int64_t)(uintptr_t)buf;
}

static void lpp_buf_free(void *ptr) {
    free(ptr);
}

static int64_t lpp_buf_len(void *ptr) {
    if (!ptr) return 0;
    return *(int64_t *)ptr;
}

static int64_t lpp_buf_get8(void *ptr, int64_t offset) {
    if (!ptr) return 0;
    int64_t size = *(int64_t *)ptr;
    if (offset < 0 || offset >= size) return 0;
    return ((uint8_t *)ptr)[8 + offset];
}

static void lpp_buf_set8(void *ptr, int64_t offset, int64_t value) {
    if (!ptr) return;
    int64_t size = *(int64_t *)ptr;
    if (offset < 0 || offset >= size) return;
    ((uint8_t *)ptr)[8 + offset] = (uint8_t)(value & 0xFF);
}

static void lpp_buf_set32le(void *ptr, int64_t offset, int64_t value) {
    if (!ptr) return;
    int64_t size = *(int64_t *)ptr;
    if (offset < 0 || offset + 4 > size) return;
    uint8_t *base = ((uint8_t *)ptr) + 8 + offset;
    uint32_t v = (uint32_t)value;
    base[0] = (uint8_t)(v);
    base[1] = (uint8_t)(v >> 8);
    base[2] = (uint8_t)(v >> 16);
    base[3] = (uint8_t)(v >> 24);
}

static int64_t lpp_buf_get32le(void *ptr, int64_t offset) {
    if (!ptr) return 0;
    int64_t size = *(int64_t *)ptr;
    if (offset < 0 || offset + 4 > size) return 0;
    uint8_t *base = ((uint8_t *)ptr) + 8 + offset;
    return (int64_t)((uint32_t)base[0] | ((uint32_t)base[1] << 8) |
                     ((uint32_t)base[2] << 16) | ((uint32_t)base[3] << 24));
}

static void lpp_buf_set16le(void *ptr, int64_t offset, int64_t value) {
    if (!ptr) return;
    int64_t size = *(int64_t *)ptr;
    if (offset < 0 || offset + 2 > size) return;
    uint8_t *base = ((uint8_t *)ptr) + 8 + offset;
    uint16_t v = (uint16_t)value;
    base[0] = (uint8_t)(v);
    base[1] = (uint8_t)(v >> 8);
}

static int64_t lpp_buf_get16le(void *ptr, int64_t offset) {
    if (!ptr) return 0;
    int64_t size = *(int64_t *)ptr;
    if (offset < 0 || offset + 2 > size) return 0;
    uint8_t *base = ((uint8_t *)ptr) + 8 + offset;
    return (int64_t)((uint16_t)base[0] | ((uint16_t)base[1] << 8));
}

static void lpp_buf_copy(void *dst, int64_t dst_off, void *src, int64_t src_off, int64_t len) {
    if (!dst || !src) return;
    int64_t dst_size = *(int64_t *)dst;
    int64_t src_size = *(int64_t *)src;
    if (dst_off < 0 || dst_off + len > dst_size) return;
    if (src_off < 0 || src_off + len > src_size) return;
    memcpy(((uint8_t *)dst) + 8 + dst_off, ((uint8_t *)src) + 8 + src_off, (size_t)len);
}

static int64_t lpp_buf_read(const char *path) {
    if (!path) return 0;
    FILE *f = fopen(path, "rb");
    if (!f) return 0;
    fseek(f, 0, SEEK_END);
    long sz = ftell(f);
    fseek(f, 0, SEEK_SET);
    if (sz < 0) { fclose(f); return 0; }
    void *buf = (void *)(uintptr_t)lpp_buf_alloc((int64_t)sz);
    if (!buf) { fclose(f); return 0; }
    size_t read = fread(((uint8_t *)buf) + 8, 1, (size_t)sz, f);
    fclose(f);
    if (read != (size_t)sz) {
        lpp_buf_free(buf);
        return 0;
    }
    return (int64_t)(uintptr_t)buf;
}

static int64_t lpp_buf_write(const char *path, void *ptr) {
    if (!path || !ptr) return -1;
    int64_t size = *(int64_t *)ptr;
    FILE *f = fopen(path, "wb");
    if (!f) return -1;
    size_t written = fwrite(((uint8_t *)ptr) + 8, 1, (size_t)size, f);
    fclose(f);
    return (written == (size_t)size) ? 0 : -1;
}

static uint32_t crc32_table[256];
static int crc32_table_ready = 0;

static void crc32_init_table(void) {
    if (crc32_table_ready) return;
    for (uint32_t i = 0; i < 256; i++) {
        uint32_t crc = i;
        for (int j = 0; j < 8; j++) {
            crc = (crc >> 1) ^ ((crc & 1) ? 0xEDB88320UL : 0);
        }
        crc32_table[i] = crc;
    }
    crc32_table_ready = 1;
}

static int64_t lpp_buf_crc32(void *ptr, int64_t off, int64_t len) {
    if (!ptr) return 0;
    int64_t size = *(int64_t *)ptr;
    if (off < 0 || len < 0 || off + len > size) return 0;
    crc32_init_table();
    uint32_t crc = 0xFFFFFFFFUL;
    uint8_t *data = ((uint8_t *)ptr) + 8 + off;
    for (int64_t i = 0; i < len; i++) {
        crc = crc32_table[(crc ^ data[i]) & 0xFF] ^ (crc >> 8);
    }
    return (int64_t)(crc ^ 0xFFFFFFFFUL);
}

static int64_t lpp_str_len(const char *s) {
    if (!s) return 0;
    return (int64_t)strlen(s);
}

static char *lpp_buf_to_str(void *ptr, int64_t off, int64_t len) {
    if (!ptr) return NULL;
    int64_t size = *(int64_t *)ptr;
    if (off < 0 || len < 0 || off + len > size) return NULL;
    char *s = (char *)malloc((size_t)len + 1);
    if (!s) return NULL;
    memcpy(s, ((uint8_t *)ptr) + 8 + off, (size_t)len);
    s[len] = 0;
    return s;
}

static int64_t lpp_buf_write_str(void *ptr, int64_t offset, const char *str) {
    if (!ptr || !str) return -1;
    int64_t size = *(int64_t *)ptr;
    int64_t len = (int64_t)strlen(str);
    if (offset < 0 || offset + len > size) return -1;
    memcpy(((uint8_t *)ptr) + 8 + offset, str, (size_t)len);
    int64_t new_end = offset + len;
    if (new_end > size) { *(int64_t *)ptr = new_end; }
    return len;
}

static char *lpp_buf_read_str(void *ptr, int64_t offset, int64_t len) {
    return lpp_buf_to_str(ptr, offset, len);
}

#define lpp_buf_free(p) lpp_buf_free((void*)(uintptr_t)(p))
#define lpp_buf_len(p) lpp_buf_len((void*)(uintptr_t)(p))
#define lpp_buf_get8(p, o) lpp_buf_get8((void*)(uintptr_t)(p), (o))
#define lpp_buf_set8(p, o, v) lpp_buf_set8((void*)(uintptr_t)(p), (o), (v))
#define lpp_buf_set32le(p, o, v) lpp_buf_set32le((void*)(uintptr_t)(p), (o), (v))
#define lpp_buf_get32le(p, o) lpp_buf_get32le((void*)(uintptr_t)(p), (o))
#define lpp_buf_set16le(p, o, v) lpp_buf_set16le((void*)(uintptr_t)(p), (o), (v))
#define lpp_buf_get16le(p, o) lpp_buf_get16le((void*)(uintptr_t)(p), (o))
#define lpp_buf_copy(d, do, s, so, l) lpp_buf_copy((void*)(uintptr_t)(d), (do), (void*)(uintptr_t)(s), (so), (l))
#define lpp_buf_write(f, p) lpp_buf_write((f), (void*)(uintptr_t)(p))
#define lpp_buf_crc32(p, o, l) lpp_buf_crc32((void*)(uintptr_t)(p), (o), (l))
#define lpp_buf_write_str(p, o, s) lpp_buf_write_str((void*)(uintptr_t)(p), (o), (s))
#define lpp_buf_read_str(p, o, l) lpp_buf_read_str((void*)(uintptr_t)(p), (o), (l))

/* ── Hash Map library (runtime/lpp_map.c) ──────────────────────────────────
 * Open-addressing linear probing hash map supporting Int and Str keys/values.
 */

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

static void *lpp_map_new(void) {
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

static void lpp_map_put(void *map, int64_t key, int64_t val) {
    lpp_map_put_internal((LppMap *)map, key, val, 0);
}

static void lpp_map_put_str(void *map, const char *key, int64_t val) {
    lpp_map_put_internal((LppMap *)map, (int64_t)(uintptr_t)key, val, 1);
}

static int64_t lpp_map_get(void *map, int64_t key) {
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

static int64_t lpp_map_get_str(void *map, const char *key) {
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

static int64_t lpp_map_has(void *map, int64_t key) {
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

static int64_t lpp_map_has_str(void *map, const char *key) {
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

static int64_t lpp_map_len(void *map) {
    LppMap *m = (LppMap *)map;
    return m ? m->len : 0;
}

static void lpp_map_remove(void *map, int64_t key) {
    LppMap *m = (LppMap *)map;
    if (!m || m->len == 0) return;

    uint64_t h = lpp_hash_int(key);
    int64_t idx = (int64_t)(h % (uint64_t)m->cap);
    int64_t start_idx = idx;

    while (m->entries[idx].occupied != 0) {
        if (m->entries[idx].occupied == 1 && m->entries[idx].is_str_key == 0 && m->entries[idx].key == key) {
            m->entries[idx].occupied = 2; /* Mark tombstone */
            m->len--;
            return;
        }
        idx = (idx + 1) % m->cap;
        if (idx == start_idx) break;
    }
}

static void lpp_map_remove_str(void *map, const char *key) {
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

static void lpp_map_put_float(void *map, int64_t key, double val) {
    int64_t ival;
    memcpy(&ival, &val, sizeof(double));
    lpp_map_put(map, key, ival);
}

static double lpp_map_get_float(void *map, int64_t key) {
    int64_t ival = lpp_map_get(map, key);
    double fval;
    memcpy(&fval, &ival, sizeof(double));
    return fval;
}

static void lpp_map_put_str_float(void *map, const char *key, double val) {
    int64_t ival;
    memcpy(&ival, &val, sizeof(double));
    lpp_map_put_str(map, key, ival);
}

static double lpp_map_get_str_float(void *map, const char *key) {
    int64_t ival = lpp_map_get_str(map, key);
    double fval;
    memcpy(&fval, &ival, sizeof(double));
    return fval;
}

#define lpp_map_get(m, k) lpp_map_get((m), (int64_t)(uintptr_t)(k))
#define lpp_map_has(m, k) lpp_map_has((m), (int64_t)(uintptr_t)(k))
#define lpp_map_remove(m, k) lpp_map_remove((m), (int64_t)(uintptr_t)(k))
#define lpp_map_put(m, k, v) lpp_map_put((m), (int64_t)(uintptr_t)(k), (int64_t)(uintptr_t)(v))
#define lpp_map_put_float(m, k, v) lpp_map_put_float((m), (int64_t)(uintptr_t)(k), (v))
#define lpp_map_get_float(m, k) lpp_map_get_float((m), (int64_t)(uintptr_t)(k))
#define lpp_map_put_str_float(m, k, v) lpp_map_put_str_float((m), (const char*)(uintptr_t)(k), (v))
#define lpp_map_get_str_float(m, k) lpp_map_get_str_float((m), (const char*)(uintptr_t)(k))
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
