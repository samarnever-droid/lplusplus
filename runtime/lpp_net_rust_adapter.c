/* L++ compatibility adapter for the Rust network static runtime.
 * Compile lpp_runtime.c with -DLPP_NO_NETWORK and link this object plus
 * liblpp_net_runtime.a. Returned Rust strings are copied into libc storage so
 * legacy lpp_free_str remains safe for generated L++ programs. */
#include <stdint.h>
#include <stdlib.h>
#include <string.h>
#include "lpp-net/include/lpp_net_runtime.h"

int64_t lpp_net_connect(const char *host, int64_t port) {
    return lpp_net_rs_connect(host, port, 30000);
}
int64_t lpp_net_listen(int64_t port) { return lpp_net_rs_listen(port); }
int64_t lpp_net_accept(int64_t listener) { return lpp_net_rs_accept(listener); }
int64_t lpp_net_send(int64_t handle, const char *data) { return lpp_net_rs_send_all(handle, data); }
int64_t lpp_net_send_all(int64_t handle, const char *data) { return lpp_net_rs_send_all(handle, data); }
int64_t lpp_net_set_timeout(int64_t handle, int64_t milliseconds) { return lpp_net_rs_set_timeout(handle, milliseconds); }
char *lpp_net_recv(int64_t handle, int64_t max_bytes) {
    char *rust = lpp_net_rs_recv(handle, max_bytes);
    if (!rust) return NULL;
    size_t length = strlen(rust);
    char *compat = (char *)malloc(length + 1);
    if (compat) memcpy(compat, rust, length + 1);
    lpp_net_rs_free_string(rust);
    return compat;
}
void lpp_net_close(int64_t handle) { lpp_net_rs_close(handle); }
