with open("runtime/windows_x86_64_min.c", "r") as f:
    content = f.read()

import re

# We need to find missing symbols from lpp-link PE direct linker:
# __ImageBase
# _fltused
# lpp_bool_to_str
# lpp_file_copy
# lpp_float_to_str
# lpp_free_str
# lpp_json_get_obj
# lpp_net_accept_timeout
# lpp_net_dial
# lpp_net_dial_udp
# lpp_net_listen_udp
# lpp_net_resolve
# lpp_net_send_all
# lpp_net_set_deadline
# lpp_net_set_keepalive
# lpp_net_set_timeout
# lpp_vec_i64_checksum
