#ifndef LPP_NET_RUNTIME_H
#define LPP_NET_RUNTIME_H

/* Stable C ABI for the Rust L++ networking runtime.
 * Handles are opaque, positive int64 values. Failure conventions:
 * - constructors/connect/accept return 0
 * - writes return -1
 * - configuration returns 0
 * - recv returns NULL for invalid arguments/allocation failure; an allocated
 *   empty string represents EOF, timeout, or OS read failure in ABI v1.
 * Returned strings must be released with lpp_net_rs_free_string(). */

#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

int64_t lpp_net_rs_connect(const char *host, int64_t port, int64_t timeout_ms);
int64_t lpp_net_rs_listen(int64_t port);
int64_t lpp_net_rs_accept(int64_t listener);
int64_t lpp_net_rs_send_all(int64_t handle, const char *data);
int64_t lpp_net_rs_set_timeout(int64_t handle, int64_t milliseconds);
char *lpp_net_rs_recv(int64_t handle, int64_t max_bytes);

int64_t lpp_net_rs_udp_bind(int64_t port);
int64_t lpp_net_rs_udp_connect(int64_t handle, const char *host, int64_t port);
int64_t lpp_net_rs_udp_send(int64_t handle, const char *data);
char *lpp_net_rs_udp_recv(int64_t handle, int64_t max_bytes);

void lpp_net_rs_close(int64_t handle);
void lpp_net_rs_free_string(char *value);

#ifdef __cplusplus
}
#endif
#endif
