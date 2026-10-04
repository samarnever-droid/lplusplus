# Generated L++ builtin ABI reference

This file is generated from `abi/builtins.toml`. Do not edit it by hand.

| Source name | Runtime symbol | Feature | Lowered signature | Targets |
|---|---|---|---|---|
| `str_slice` | `lpp_slice_init` | `string` | `(i64, i64, i64, i64, i64) -> i64` | legacy |
| `slice` | `lpp_slice_init` | `slice` | `(i64, i64, i64, i64, i64) -> i64` | legacy |
| `slice_len` | `lpp_slice_len` | `slice` | `(i64) -> i64` | legacy |
| `slice_get` | `lpp_slice_get` | `slice` | `(i64, i64) -> i64` | legacy |
| `lpp_slice_get_bool` | `lpp_slice_get_bool` | `slice` | `(i64, i64) -> bool` | legacy |
| `slice_to_str` | `lpp_str_slice_to_str` | `slice` | `(i64) -> i64` | legacy |
| `str_slice_to_str` | `lpp_str_slice_to_str` | `string` | `(i64) -> i64` | legacy |
| `lpp_tuple_alloc` | `lpp_tuple_alloc` | `tuple` | `(i64, i64, i64) -> i64` | legacy |
| `lpp_task_new` | `lpp_task_new` | `async_task` | `(i64, i64, i64) -> i64` | legacy |
| `lpp_task_poll` | `lpp_task_poll` | `async_task` | `(i64) -> i64` | legacy |
| `lpp_task_await` | `lpp_task_await` | `async_task` | `(i64) -> i64` | legacy |
| `lpp_executor_run` | `lpp_executor_run` | `async_task` | `(i64) -> i64` | legacy |
| `lpp_task_destroy` | `lpp_task_destroy` | `async_task` | `(i64) -> void` | legacy |
| `print` | `—` | `io` | `() -> void` | legacy |
| `print_str` | `lpp_print_str` | `io` | `(i64) -> void` | legacy |
| `lpp_print_str` | `lpp_print_str` | `io` | `(i64) -> void` | legacy |
| `write_str` | `lpp_write_str` | `io` | `(i64) -> void` | legacy |
| `lpp_write_str` | `lpp_write_str` | `io` | `(i64) -> void` | legacy |
| `print_int` | `lpp_print_int` | `io` | `(i64) -> void` | legacy |
| `lpp_print_int` | `lpp_print_int` | `io` | `(i64) -> void` | legacy |
| `print_float` | `lpp_print_float` | `io` | `(f64) -> void` | legacy |
| `lpp_print_float` | `lpp_print_float` | `io` | `(f64) -> void` | legacy |
| `print_bool` | `lpp_print_bool` | `io` | `(bool) -> void` | legacy |
| `lpp_print_bool` | `lpp_print_bool` | `io` | `(bool) -> void` | legacy |
| `fmod` | `fmod` | `core` | `(f64, f64) -> f64` | legacy |
| `input` | `lpp_input` | `io` | `() -> i64` | legacy |
| `lpp_input` | `lpp_input` | `io` | `() -> i64` | legacy |
| `lpp_free_str` | `lpp_free_str` | `memory` | `(i64) -> void` | legacy |
| `lpp_arc_retain` | `lpp_arc_retain` | `memory` | `(i64) -> void` | legacy |
| `lpp_arc_retain_local` | `lpp_arc_retain_local` | `memory` | `(i64) -> void` | legacy |
| `lpp_arc_retain_local` | `lpp_arc_retain_local` | `memory` | `(i64) -> void` | legacy |
| `lpp_arc_release_local` | `lpp_arc_release_local` | `memory` | `(i64) -> void` | legacy |
| `lpp_arc_release` | `lpp_arc_release` | `memory` | `(i64) -> void` | legacy |
| `lpp_arc_alloc` | `lpp_arc_alloc` | `memory` | `(i64) -> i64` | legacy |
| `lpp_arc_alloc_with_destructor` | `lpp_arc_alloc_with_destructor` | `memory` | `(i64, i64) -> i64` | legacy |
| `lpp_arena_begin` | `lpp_arena_begin` | `memory` | `() -> i64` | legacy |
| `lpp_arena_release` | `lpp_arena_release` | `memory` | `(i64) -> void` | legacy |
| `lpp_arena_retain` | `lpp_arena_retain` | `memory` | `(i64) -> void` | legacy |
| `lpp_arena_alloc` | `lpp_arena_alloc` | `memory` | `(i64, i64, i64) -> i64` | legacy |
| `lpp_arena_release_node` | `lpp_arena_release_node` | `memory` | `(i64) -> void` | legacy |
| `lpp_closure_destroy` | `lpp_closure_destroy` | `closure` | `(i64) -> void` | legacy |
| `lpp_alloc` | `lpp_alloc` | `memory` | `(i64) -> i64` | legacy |
| `lpp_free` | `lpp_free` | `memory` | `(i64, i64) -> void` | legacy |
| `lpp_thread_spawn` | `lpp_thread_spawn` | `concurrency` | `(i64, i64) -> void` | legacy |
| `len` | `lpp_list_len` | `core` | `(i64) -> i64` | legacy |
| `get` | `lpp_list_get` | `core` | `(i64, i64) -> i64` | legacy |
| `list_new` | `lpp_list_new` | `list` | `() -> i64` | legacy |
| `lpp_list_new` | `lpp_list_new` | `list` | `() -> i64` | legacy |
| `lpp_list_new_arc` | `lpp_list_new_arc` | `list` | `() -> i64` | legacy |
| `list_push` | `lpp_list_push` | `list` | `(i64, i64) -> void` | legacy |
| `lpp_list_push` | `lpp_list_push` | `list` | `(i64, i64) -> void` | legacy |
| `lpp_list_push_arc` | `lpp_list_push_arc` | `list` | `(i64, i64) -> void` | legacy |
| `lpp_list_push_float` | `lpp_list_push_float` | `list` | `(i64, f64) -> void` | legacy |
| `lpp_list_push_bool` | `lpp_list_push_bool` | `list` | `(i64, bool) -> void` | legacy |
| `list_set` | `lpp_list_set` | `list` | `(i64, i64, i64) -> void` | legacy |
| `lpp_list_set` | `lpp_list_set` | `list` | `(i64, i64, i64) -> void` | legacy |
| `lpp_list_set_bool` | `lpp_list_set_bool` | `list` | `(i64, i64, bool) -> void` | legacy |
| `lpp_list_set_float` | `lpp_list_set_float` | `list` | `(i64, i64, f64) -> void` | legacy |
| `lpp_list_set_arc` | `lpp_list_set_arc` | `list` | `(i64, i64, i64) -> void` | legacy |
| `list_get` | `lpp_list_get` | `list` | `(i64, i64) -> i64` | legacy |
| `lpp_list_get` | `lpp_list_get` | `list` | `(i64, i64) -> i64` | legacy |
| `lpp_list_get_arc` | `lpp_list_get_arc` | `list` | `(i64, i64) -> i64` | legacy |
| `lpp_list_get_float` | `lpp_list_get_float` | `list` | `(i64, i64) -> f64` | legacy |
| `lpp_list_get_bool` | `lpp_list_get_bool` | `list` | `(i64, i64) -> bool` | legacy |
| `list_len` | `lpp_list_len` | `list` | `(i64) -> i64` | legacy |
| `lpp_list_len` | `lpp_list_len` | `list` | `(i64) -> i64` | legacy |
| `list_pop` | `lpp_list_pop` | `list` | `(i64) -> i64` | legacy |
| `lpp_list_pop` | `lpp_list_pop` | `list` | `(i64) -> i64` | legacy |
| `list_free` | `lpp_list_free` | `list` | `(i64) -> void` | legacy |
| `lpp_list_free` | `lpp_list_free` | `list` | `(i64) -> void` | legacy |
| `map_new` | `lpp_map_new` | `map` | `() -> i64` | legacy |
| `lpp_map_new` | `lpp_map_new` | `map` | `() -> i64` | legacy |
| `map_new_arc` | `lpp_map_new_arc` | `map` | `() -> i64` | legacy |
| `lpp_map_new_arc` | `lpp_map_new_arc` | `map` | `() -> i64` | legacy |
| `map_put` | `lpp_map_put` | `map` | `(i64, i64, i64) -> void` | legacy |
| `lpp_map_put` | `lpp_map_put` | `map` | `(i64, i64, i64) -> void` | legacy |
| `lpp_map_put_float` | `lpp_map_put_float` | `map` | `(i64, i64, f64) -> void` | legacy |
| `lpp_map_put_str` | `lpp_map_put_str` | `map` | `(i64, i64, i64) -> void` | legacy |
| `lpp_map_put_str_float` | `lpp_map_put_str_float` | `map` | `(i64, i64, f64) -> void` | legacy |
| `lpp_map_get_str_float` | `lpp_map_get_str_float` | `map` | `(i64, i64) -> f64` | legacy |
| `map_get` | `lpp_map_get` | `map` | `(i64, i64) -> i64` | legacy |
| `lpp_map_get` | `lpp_map_get` | `map` | `(i64, i64) -> i64` | legacy |
| `lpp_map_get_float` | `lpp_map_get_float` | `map` | `(i64, i64) -> f64` | legacy |
| `lpp_map_get_str` | `lpp_map_get_str` | `map` | `(i64, i64) -> i64` | legacy |
| `lpp_map_has_str` | `lpp_map_has_str` | `map` | `(i64, i64) -> i64` | legacy |
| `lpp_map_remove_str` | `lpp_map_remove_str` | `map` | `(i64, i64) -> void` | legacy |
| `map_has` | `lpp_map_has` | `map` | `(i64, i64) -> i64` | legacy |
| `lpp_map_has` | `lpp_map_has` | `map` | `(i64, i64) -> i64` | legacy |
| `map_len` | `lpp_map_len` | `map` | `(i64) -> i64` | legacy |
| `lpp_map_len` | `lpp_map_len` | `map` | `(i64) -> i64` | legacy |
| `map_remove` | `lpp_map_remove` | `map` | `(i64, i64) -> void` | legacy |
| `lpp_map_remove` | `lpp_map_remove` | `map` | `(i64, i64) -> void` | legacy |
| `net_dial` | `lpp_net_dial` | `network` | `(i64, i64, i64) -> i64` | legacy |
| `lpp_net_dial` | `lpp_net_dial` | `network` | `(i64, i64, i64) -> i64` | legacy |
| `net_dial_udp` | `lpp_net_dial_udp` | `network` | `(i64, i64, i64) -> i64` | legacy |
| `lpp_net_dial_udp` | `lpp_net_dial_udp` | `network` | `(i64, i64, i64) -> i64` | legacy |
| `net_listen` | `lpp_net_listen` | `network` | `(i64) -> i64` | legacy |
| `lpp_net_listen` | `lpp_net_listen` | `network` | `(i64) -> i64` | legacy |
| `net_listen_udp` | `lpp_net_listen_udp` | `network` | `(i64) -> i64` | legacy |
| `lpp_net_listen_udp` | `lpp_net_listen_udp` | `network` | `(i64) -> i64` | legacy |
| `net_accept` | `lpp_net_accept` | `network` | `(i64) -> i64` | legacy |
| `lpp_net_accept` | `lpp_net_accept` | `network` | `(i64) -> i64` | legacy |
| `net_accept_timeout` | `lpp_net_accept_timeout` | `network` | `(i64, i64) -> i64` | legacy |
| `lpp_net_accept_timeout` | `lpp_net_accept_timeout` | `network` | `(i64, i64) -> i64` | legacy |
| `net_send` | `lpp_net_send` | `network` | `(i64, i64) -> i64` | legacy |
| `lpp_net_send` | `lpp_net_send` | `network` | `(i64, i64) -> i64` | legacy |
| `net_send_all` | `lpp_net_send_all` | `network` | `(i64, i64) -> i64` | legacy |
| `lpp_net_send_all` | `lpp_net_send_all` | `network` | `(i64, i64) -> i64` | legacy |
| `net_recv` | `lpp_net_recv` | `network` | `(i64, i64) -> i64` | legacy |
| `lpp_net_recv` | `lpp_net_recv` | `network` | `(i64, i64) -> i64` | legacy |
| `net_recv_udp` | `lpp_net_recv_udp` | `network` | `(i64, i64) -> i64` | legacy |
| `lpp_net_recv_udp` | `lpp_net_recv_udp` | `network` | `(i64, i64) -> i64` | legacy |
| `net_close` | `lpp_net_close` | `network` | `(i64) -> void` | legacy |
| `lpp_net_close` | `lpp_net_close` | `network` | `(i64) -> void` | legacy |
| `net_set_deadline` | `lpp_net_set_deadline` | `network` | `(i64, i64, i64) -> i64` | legacy |
| `lpp_net_set_deadline` | `lpp_net_set_deadline` | `network` | `(i64, i64, i64) -> i64` | legacy |
| `net_set_timeout` | `lpp_net_set_timeout` | `network` | `(i64, i64) -> i64` | legacy |
| `lpp_net_set_timeout` | `lpp_net_set_timeout` | `network` | `(i64, i64) -> i64` | legacy |
| `net_set_nonblocking` | `lpp_net_set_nonblocking` | `network` | `(i64, i64) -> i64` | legacy |
| `lpp_net_set_nonblocking` | `lpp_net_set_nonblocking` | `network` | `(i64, i64) -> i64` | legacy |
| `net_poll` | `lpp_net_poll` | `network` | `(i64, i64) -> i64` | legacy |
| `lpp_net_poll` | `lpp_net_poll` | `network` | `(i64, i64) -> i64` | legacy |
| `net_set_keepalive` | `lpp_net_set_keepalive` | `network` | `(i64, i64, i64, i64, i64) -> i64` | legacy |
| `lpp_net_set_keepalive` | `lpp_net_set_keepalive` | `network` | `(i64, i64, i64, i64, i64) -> i64` | legacy |
| `net_resolve` | `lpp_net_resolve` | `network` | `(i64) -> i64` | legacy |
| `lpp_net_resolve` | `lpp_net_resolve` | `network` | `(i64) -> i64` | legacy |
| `http_get` | `lpp_http_get` | `network` | `(i64, i64) -> i64` | legacy |
| `lpp_http_get` | `lpp_http_get` | `network` | `(i64, i64) -> i64` | legacy |
| `http_post` | `lpp_http_post` | `network` | `(i64, i64, i64, i64) -> i64` | legacy |
| `lpp_http_post` | `lpp_http_post` | `network` | `(i64, i64, i64, i64) -> i64` | legacy |
| `net_connect` | `lpp_net_connect` | `network` | `(i64, i64) -> i64` | legacy |
| `lpp_net_connect` | `lpp_net_connect` | `network` | `(i64, i64) -> i64` | legacy |
| `json_parse` | `lpp_json_parse` | `json` | `(i64) -> i64` | legacy |
| `lpp_json_parse` | `lpp_json_parse` | `json` | `(i64) -> i64` | legacy |
| `json_get_int` | `lpp_json_get_int` | `json` | `(i64, i64) -> i64` | legacy |
| `lpp_json_get_int` | `lpp_json_get_int` | `json` | `(i64, i64) -> i64` | legacy |
| `json_get_str` | `lpp_json_get_str` | `json` | `(i64, i64) -> i64` | legacy |
| `lpp_json_get_str` | `lpp_json_get_str` | `json` | `(i64, i64) -> i64` | legacy |
| `json_get_obj` | `lpp_json_get_obj` | `json` | `(i64, i64) -> i64` | legacy |
| `lpp_json_get_obj` | `lpp_json_get_obj` | `json` | `(i64, i64) -> i64` | legacy |
| `json_free` | `lpp_json_free` | `json` | `(i64) -> void` | legacy |
| `lpp_json_free` | `lpp_json_free` | `json` | `(i64) -> void` | legacy |
| `parse_int` | `lpp_parse_int` | `core` | `(i64) -> i64` | legacy |
| `lpp_parse_int` | `lpp_parse_int` | `core` | `(i64) -> i64` | legacy |
| `read_file` | `lpp_read_file` | `filesystem` | `(i64) -> i64` | legacy |
| `lpp_read_file` | `lpp_read_file` | `filesystem` | `(i64) -> i64` | legacy |
| `write_file` | `lpp_write_file` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `lpp_write_file` | `lpp_write_file` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `append_file` | `lpp_append_file` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `lpp_append_file` | `lpp_append_file` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `delete_file` | `lpp_delete_file` | `filesystem` | `(i64) -> i64` | legacy |
| `lpp_delete_file` | `lpp_delete_file` | `filesystem` | `(i64) -> i64` | legacy |
| `file_exists` | `lpp_file_exists` | `filesystem` | `(i64) -> i64` | legacy |
| `lpp_file_exists` | `lpp_file_exists` | `filesystem` | `(i64) -> i64` | legacy |
| `file_size` | `lpp_file_size` | `filesystem` | `(i64) -> i64` | legacy |
| `lpp_file_size` | `lpp_file_size` | `filesystem` | `(i64) -> i64` | legacy |
| `file_copy` | `lpp_file_copy` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `lpp_file_copy` | `lpp_file_copy` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `file_move` | `lpp_file_move` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `lpp_file_move` | `lpp_file_move` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `str_concat` | `lpp_str_concat` | `string` | `(i64, i64) -> i64` | legacy |
| `lpp_str_concat` | `lpp_str_concat` | `string` | `(i64, i64) -> i64` | legacy |
| `str_repeat` | `lpp_str_repeat` | `string` | `(i64, i64) -> i64` | legacy |
| `lpp_str_repeat` | `lpp_str_repeat` | `string` | `(i64, i64) -> i64` | legacy |
| `str_split` | `lpp_str_split` | `string` | `(i64, i64) -> i64` | legacy |
| `lpp_str_split` | `lpp_str_split` | `string` | `(i64, i64) -> i64` | legacy |
| `str_find` | `lpp_str_find` | `string` | `(i64, i64) -> i64` | legacy |
| `lpp_str_find` | `lpp_str_find` | `string` | `(i64, i64) -> i64` | legacy |
| `str_replace` | `lpp_str_replace` | `string` | `(i64, i64, i64) -> i64` | legacy |
| `lpp_str_replace` | `lpp_str_replace` | `string` | `(i64, i64, i64) -> i64` | legacy |
| `str_substr` | `lpp_str_substr` | `string` | `(i64, i64, i64) -> i64` | legacy |
| `lpp_str_substr` | `lpp_str_substr` | `string` | `(i64, i64, i64) -> i64` | legacy |
| `str_trim` | `lpp_str_trim` | `string` | `(i64) -> i64` | legacy |
| `lpp_str_trim` | `lpp_str_trim` | `string` | `(i64) -> i64` | legacy |
| `command_exec` | `lpp_command_exec` | `core` | `(i64) -> i64` | legacy |
| `lpp_command_exec` | `lpp_command_exec` | `core` | `(i64) -> i64` | legacy |
| `command_output` | `lpp_command_output` | `core` | `(i64) -> i64` | legacy |
| `lpp_command_output` | `lpp_command_output` | `core` | `(i64) -> i64` | legacy |
| `env_get` | `lpp_env_get` | `core` | `(i64) -> i64` | legacy |
| `lpp_env_get` | `lpp_env_get` | `core` | `(i64) -> i64` | legacy |
| `env_set` | `lpp_env_set` | `core` | `(i64, i64) -> i64` | legacy |
| `lpp_env_set` | `lpp_env_set` | `core` | `(i64, i64) -> i64` | legacy |
| `dir_create` | `lpp_dir_create` | `filesystem` | `(i64) -> i64` | legacy |
| `lpp_dir_create` | `lpp_dir_create` | `filesystem` | `(i64) -> i64` | legacy |
| `dir_list` | `lpp_dir_list` | `filesystem` | `(i64) -> i64` | legacy |
| `lpp_dir_list` | `lpp_dir_list` | `filesystem` | `(i64) -> i64` | legacy |
| `dir_remove` | `lpp_dir_remove` | `filesystem` | `(i64) -> i64` | legacy |
| `lpp_dir_remove` | `lpp_dir_remove` | `filesystem` | `(i64) -> i64` | legacy |
| `path_exists` | `lpp_path_exists` | `filesystem` | `(i64) -> i64` | legacy |
| `lpp_path_exists` | `lpp_path_exists` | `filesystem` | `(i64) -> i64` | legacy |
| `path_join` | `lpp_path_join` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `lpp_path_join` | `lpp_path_join` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `str_len` | `lpp_str_len` | `string` | `(i64) -> i64` | legacy |
| `vec_i64x2` | `lpp_vec_i64x2` | `simd` | `(i64, i64, i64, i64) -> i64` | legacy |
| `vec_i64x2_splat` | `lpp_vec_i64x2_splat` | `simd` | `(i64) -> i64` | legacy |
| `vec_i64x2_add` | `lpp_vec_i64x2_add` | `simd` | `(i64, i64) -> i64` | legacy |
| `vec_i64x2_sub` | `lpp_vec_i64x2_sub` | `simd` | `(i64, i64) -> i64` | legacy |
| `vec_i64x2_mul` | `lpp_vec_i64x2_mul` | `simd` | `(i64, i64) -> i64` | legacy |
| `vec_i64x2_xor` | `lpp_vec_i64x2_xor` | `simd` | `(i64, i64) -> i64` | legacy |
| `vec_i64x2_and` | `lpp_vec_i64x2_and` | `simd` | `(i64, i64) -> i64` | legacy |
| `vec_i64x2_or` | `lpp_vec_i64x2_or` | `simd` | `(i64, i64) -> i64` | legacy |
| `vec_i64x2_not` | `lpp_vec_i64x2_not` | `simd` | `(i64) -> i64` | legacy |
| `vec_u8x16_eq` | `lpp_vec_u8x16_eq` | `simd` | `(i64, i64) -> i64` | legacy |
| `vec_u8x16_movemask` | `lpp_vec_u8x16_movemask` | `simd` | `(i64) -> i64` | legacy |
| `vec_movemask8` | `lpp_vec_u8x16_movemask` | `simd` | `(i64) -> i64` | legacy |
| `vec_u8x16_splat` | `lpp_vec_u8x16_splat` | `simd` | `(i64) -> i64` | legacy |
| `vec_i64x2_shr` | `lpp_vec_i64x2_shr` | `simd` | `(i64, i64) -> i64` | legacy |
| `vec_i64x2_shr_var` | `lpp_vec_i64x2_shr_var` | `simd` | `(i64, i64) -> i64` | legacy |
| `vec_i64x2_extract` | `lpp_vec_i64x2_extract` | `simd` | `(i64, i64) -> i64` | legacy |
| `vec_i64x2_sum` | `lpp_vec_i64x2_sum` | `simd` | `(i64) -> i64` | legacy |
| `vec_i64_checksum` | `lpp_vec_i64_checksum` | `simd` | `(i64) -> i64` | legacy |
| `lpp_vec_i64_checksum` | `lpp_vec_i64_checksum` | `simd` | `(i64) -> i64` | legacy |
| `lpp_str_len` | `lpp_str_len` | `string` | `(i64) -> i64` | legacy |
| `buf_alloc` | `lpp_buf_alloc` | `buffer` | `(i64) -> i64` | legacy |
| `lpp_buf_alloc` | `lpp_buf_alloc` | `buffer` | `(i64) -> i64` | legacy |
| `buf_free` | `lpp_buf_free` | `buffer` | `(i64) -> void` | legacy |
| `lpp_buf_free` | `lpp_buf_free` | `buffer` | `(i64) -> void` | legacy |
| `buf_len` | `lpp_buf_len` | `buffer` | `(i64) -> i64` | legacy |
| `lpp_buf_len` | `lpp_buf_len` | `buffer` | `(i64) -> i64` | legacy |
| `buf_get8` | `lpp_buf_get8` | `buffer` | `(i64, i64) -> i64` | legacy |
| `lpp_buf_get8` | `lpp_buf_get8` | `buffer` | `(i64, i64) -> i64` | legacy |
| `buf_set8` | `lpp_buf_set8` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `lpp_buf_set8` | `lpp_buf_set8` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `buf_set32le` | `lpp_buf_set32le` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `lpp_buf_set32le` | `lpp_buf_set32le` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `buf_get32le` | `lpp_buf_get32le` | `buffer` | `(i64, i64) -> i64` | legacy |
| `lpp_buf_get32le` | `lpp_buf_get32le` | `buffer` | `(i64, i64) -> i64` | legacy |
| `buf_set16le` | `lpp_buf_set16le` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `lpp_buf_set16le` | `lpp_buf_set16le` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `buf_set64le` | `lpp_buf_set64le` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `lpp_buf_set64le` | `lpp_buf_set64le` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `buf_get64le` | `lpp_buf_get64le` | `buffer` | `(i64, i64) -> i64` | legacy |
| `lpp_buf_get64le` | `lpp_buf_get64le` | `buffer` | `(i64, i64) -> i64` | legacy |
| `buf_set64be` | `lpp_buf_set64be` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `lpp_buf_set64be` | `lpp_buf_set64be` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `buf_get64be` | `lpp_buf_get64be` | `buffer` | `(i64, i64) -> i64` | legacy |
| `lpp_buf_get64be` | `lpp_buf_get64be` | `buffer` | `(i64, i64) -> i64` | legacy |
| `buf_set32be` | `lpp_buf_set32be` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `lpp_buf_set32be` | `lpp_buf_set32be` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `buf_get32be` | `lpp_buf_get32be` | `buffer` | `(i64, i64) -> i64` | legacy |
| `lpp_buf_get32be` | `lpp_buf_get32be` | `buffer` | `(i64, i64) -> i64` | legacy |
| `buf_set16be` | `lpp_buf_set16be` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `lpp_buf_set16be` | `lpp_buf_set16be` | `buffer` | `(i64, i64, i64) -> void` | legacy |
| `buf_get16be` | `lpp_buf_get16be` | `buffer` | `(i64, i64) -> i64` | legacy |
| `lpp_buf_get16be` | `lpp_buf_get16be` | `buffer` | `(i64, i64) -> i64` | legacy |
| `buf_get16le` | `lpp_buf_get16le` | `buffer` | `(i64, i64) -> i64` | legacy |
| `lpp_buf_get16le` | `lpp_buf_get16le` | `buffer` | `(i64, i64) -> i64` | legacy |
| `buf_read` | `lpp_buf_read` | `buffer` | `(i64) -> i64` | legacy |
| `lpp_buf_read` | `lpp_buf_read` | `buffer` | `(i64) -> i64` | legacy |
| `buf_write` | `lpp_buf_write` | `buffer` | `(i64, i64) -> i64` | legacy |
| `lpp_buf_write` | `lpp_buf_write` | `buffer` | `(i64, i64) -> i64` | legacy |
| `buf_crc32` | `lpp_buf_crc32` | `buffer` | `(i64, i64, i64) -> i64` | legacy |
| `lpp_buf_crc32` | `lpp_buf_crc32` | `buffer` | `(i64, i64, i64) -> i64` | legacy |
| `file_open` | `lpp_file_open` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `lpp_file_open` | `lpp_file_open` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `file_close` | `lpp_file_close` | `filesystem` | `(i64) -> void` | legacy |
| `lpp_file_close` | `lpp_file_close` | `filesystem` | `(i64) -> void` | legacy |
| `file_pread` | `lpp_file_pread` | `filesystem` | `(i64, i64, i64, i64, i64) -> i64` | legacy |
| `lpp_file_pread` | `lpp_file_pread` | `filesystem` | `(i64, i64, i64, i64, i64) -> i64` | legacy |
| `file_pwrite` | `lpp_file_pwrite` | `filesystem` | `(i64, i64, i64, i64, i64) -> i64` | legacy |
| `lpp_file_pwrite` | `lpp_file_pwrite` | `filesystem` | `(i64, i64, i64, i64, i64) -> i64` | legacy |
| `file_fsync` | `lpp_file_fsync` | `filesystem` | `(i64) -> i64` | legacy |
| `lpp_file_fsync` | `lpp_file_fsync` | `filesystem` | `(i64) -> i64` | legacy |
| `file_fdatasync` | `lpp_file_fdatasync` | `filesystem` | `(i64) -> i64` | legacy |
| `lpp_file_fdatasync` | `lpp_file_fdatasync` | `filesystem` | `(i64) -> i64` | legacy |
| `file_ftruncate` | `lpp_file_ftruncate` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `lpp_file_ftruncate` | `lpp_file_ftruncate` | `filesystem` | `(i64, i64) -> i64` | legacy |
| `file_size` | `lpp_fd_size` | `filesystem` | `(i64) -> i64` | legacy |
| `lpp_file_size` | `lpp_fd_size` | `filesystem` | `(i64) -> i64` | legacy |
| `buf_copy` | `lpp_buf_copy` | `buffer` | `(i64, i64, i64, i64, i64) -> void` | legacy |
| `lpp_buf_copy` | `lpp_buf_copy` | `buffer` | `(i64, i64, i64, i64, i64) -> void` | legacy |
| `buf_write_str` | `lpp_buf_write_str` | `buffer` | `(i64, i64, i64) -> i64` | legacy |
| `lpp_buf_write_str` | `lpp_buf_write_str` | `buffer` | `(i64, i64, i64) -> i64` | legacy |
| `buf_read_str` | `lpp_buf_read_str` | `buffer` | `(i64, i64, i64) -> i64` | legacy |
| `lpp_buf_read_str` | `lpp_buf_read_str` | `buffer` | `(i64, i64, i64) -> i64` | legacy |
| `char_at` | `lpp_char_at` | `string` | `(i64, i64) -> i64` | legacy |
| `lpp_char_at` | `lpp_char_at` | `string` | `(i64, i64) -> i64` | legacy |
| `ord` | `lpp_ord` | `string` | `(i64) -> i64` | legacy |
| `lpp_ord` | `lpp_ord` | `string` | `(i64) -> i64` | legacy |
| `chr` | `lpp_chr` | `string` | `(i64) -> i64` | legacy |
| `lpp_chr` | `lpp_chr` | `string` | `(i64) -> i64` | legacy |
| `str_find` | `lpp_str_find` | `string` | `(i64, i64) -> i64` | legacy |
| `lpp_str_find` | `lpp_str_find` | `string` | `(i64, i64) -> i64` | legacy |
| `str_contains` | `lpp_str_contains` | `string` | `(i64, i64) -> i64` | legacy |
| `lpp_str_contains` | `lpp_str_contains` | `string` | `(i64, i64) -> i64` | legacy |
| `str_starts_with` | `lpp_str_starts_with` | `string` | `(i64, i64) -> i64` | legacy |
| `lpp_str_starts_with` | `lpp_str_starts_with` | `string` | `(i64, i64) -> i64` | legacy |
| `str_ends_with` | `lpp_str_ends_with` | `string` | `(i64, i64) -> i64` | legacy |
| `lpp_str_ends_with` | `lpp_str_ends_with` | `string` | `(i64, i64) -> i64` | legacy |
| `str_upper` | `lpp_str_upper` | `string` | `(i64) -> i64` | legacy |
| `str_to_upper` | `lpp_str_upper` | `string` | `(i64) -> i64` | legacy |
| `lpp_str_upper` | `lpp_str_upper` | `string` | `(i64) -> i64` | legacy |
| `str_lower` | `lpp_str_lower` | `string` | `(i64) -> i64` | legacy |
| `str_to_lower` | `lpp_str_lower` | `string` | `(i64) -> i64` | legacy |
| `lpp_str_lower` | `lpp_str_lower` | `string` | `(i64) -> i64` | legacy |
| `str_trim` | `lpp_str_trim` | `string` | `(i64) -> i64` | legacy |
| `lpp_str_trim` | `lpp_str_trim` | `string` | `(i64) -> i64` | legacy |
| `str_replace` | `lpp_str_replace` | `string` | `(i64, i64, i64) -> i64` | legacy |
| `lpp_str_replace` | `lpp_str_replace` | `string` | `(i64, i64, i64) -> i64` | legacy |
| `int_to_str` | `lpp_int_to_str` | `core` | `(i64) -> i64` | legacy |
| `lpp_int_to_str` | `lpp_int_to_str` | `core` | `(i64) -> i64` | legacy |
| `float_to_str` | `lpp_float_to_str` | `core` | `(f64) -> i64` | legacy |
| `lpp_float_to_str` | `lpp_float_to_str` | `core` | `(f64) -> i64` | legacy |
| `bool_to_str` | `lpp_bool_to_str` | `core` | `(bool) -> i64` | legacy |
| `lpp_bool_to_str` | `lpp_bool_to_str` | `core` | `(bool) -> i64` | legacy |
| `gui_window_create` | `lpp_gui_window_create` | `gui` | `(i64, i64, i64) -> i64` | legacy |
| `gui_window_is_open` | `lpp_gui_window_is_open` | `gui` | `(i64) -> i64` | legacy |
| `gui_window_width` | `lpp_gui_window_width` | `gui` | `(i64) -> i64` | legacy |
| `gui_window_height` | `lpp_gui_window_height` | `gui` | `(i64) -> i64` | legacy |
| `gui_window_poll_events` | `lpp_gui_window_poll_events` | `gui` | `(i64) -> i64` | legacy |
| `gui_clear` | `lpp_gui_clear` | `gui` | `(i64, i64) -> void` | legacy |
| `gui_draw_rect` | `lpp_gui_draw_rect` | `gui` | `(i64, i64, i64, i64, i64, i64) -> void` | legacy |
| `gui_draw_rounded_rect` | `lpp_gui_draw_rounded_rect` | `gui` | `(i64, i64, i64, i64, i64, i64, i64) -> void` | legacy |
| `gui_mouse_x` | `lpp_gui_mouse_x` | `gui` | `(i64) -> i64` | legacy |
| `gui_mouse_y` | `lpp_gui_mouse_y` | `gui` | `(i64) -> i64` | legacy |
| `gui_mouse_down` | `lpp_gui_mouse_down` | `gui` | `(i64) -> i64` | legacy |
| `gui_key_down` | `lpp_gui_key_down` | `gui` | `(i64, i64) -> i64` | legacy |
| `gui_draw_text` | `lpp_gui_draw_text` | `gui` | `(i64, i64, i64, i64, i64) -> void` | legacy |
| `gui_present` | `lpp_gui_present` | `gui` | `(i64) -> void` | legacy |
| `gui_window_close` | `lpp_gui_window_close` | `gui` | `(i64) -> void` | legacy |
| `gui_draw_circle` | `lpp_gui_draw_circle` | `gui` | `(i64, i64, i64, i64, i64) -> void` | legacy |
| `gui_draw_line` | `lpp_gui_draw_line` | `gui` | `(i64, i64, i64, i64, i64, i64, i64) -> void` | legacy |
| `gui_measure_text_width` | `lpp_gui_measure_text_width` | `gui` | `(i64, i64) -> i64` | legacy |
| `gui_dialog_message` | `lpp_gui_dialog_message` | `gui` | `(i64, i64) -> i64` | legacy |
| `gui_get_ticks_ms` | `lpp_gui_get_ticks_ms` | `gui` | `() -> i64` | legacy |
| `webview_window_create` | `lpp_webview_window_create` | `gui` | `(i64, i64, i64, i64) -> i64` | legacy |
| `webview_set_html` | `lpp_webview_set_html` | `gui` | `(i64, i64) -> void` | legacy |
| `webview_navigate` | `lpp_webview_navigate` | `gui` | `(i64, i64) -> void` | legacy |
| `webview_run` | `lpp_webview_run` | `gui` | `(i64) -> void` | legacy |
| `webview_terminate` | `lpp_webview_terminate` | `gui` | `(i64) -> void` | legacy |
| `webview_destroy` | `lpp_webview_destroy` | `gui` | `(i64) -> void` | legacy |
| `str_to_int` | `lpp_str_to_int` | `string` | `(i64) -> i64` | legacy |
| `lpp_str_to_int` | `lpp_str_to_int` | `string` | `(i64) -> i64` | legacy |
| `lpp_abs` | `lpp_abs` | `core` | `(i64) -> i64` | legacy |
| `abs` | `lpp_abs` | `core` | `(i64) -> i64` | legacy |
| `lpp_min` | `lpp_min` | `core` | `(i64, i64) -> i64` | legacy |
| `min` | `lpp_min` | `core` | `(i64, i64) -> i64` | legacy |
| `lpp_max` | `lpp_max` | `core` | `(i64, i64) -> i64` | legacy |
| `max` | `lpp_max` | `core` | `(i64, i64) -> i64` | legacy |
| `sqrt` | `lpp_sqrt` | `core` | `(f64) -> f64` | legacy |
| `lpp_sqrt` | `lpp_sqrt` | `core` | `(f64) -> f64` | legacy |
| `sin` | `lpp_sin` | `core` | `(f64) -> f64` | legacy |
| `lpp_sin` | `lpp_sin` | `core` | `(f64) -> f64` | legacy |
| `cos` | `lpp_cos` | `core` | `(f64) -> f64` | legacy |
| `lpp_cos` | `lpp_cos` | `core` | `(f64) -> f64` | legacy |
| `tan` | `lpp_tan` | `core` | `(f64) -> f64` | legacy |
| `lpp_tan` | `lpp_tan` | `core` | `(f64) -> f64` | legacy |
| `lpp_floor` | `lpp_floor` | `core` | `(f64) -> f64` | legacy |
| `floor` | `lpp_floor` | `core` | `(f64) -> f64` | legacy |
| `lpp_ceil` | `lpp_ceil` | `core` | `(f64) -> f64` | legacy |
| `ceil` | `lpp_ceil` | `core` | `(f64) -> f64` | legacy |
| `lpp_pow` | `lpp_pow` | `core` | `(f64, f64) -> f64` | legacy |
| `pow` | `lpp_pow` | `core` | `(f64, f64) -> f64` | legacy |
| `int_pow` | `lpp_int_pow` | `core` | `(i64, i64) -> i64` | legacy |
| `lpp_int_pow` | `lpp_int_pow` | `core` | `(i64, i64) -> i64` | legacy |
| `int_to_float` | `lpp_int_to_float` | `core` | `(i64) -> f64` | legacy |
| `lpp_int_to_float` | `lpp_int_to_float` | `core` | `(i64) -> f64` | legacy |
| `float_to_int` | `lpp_float_to_int` | `core` | `(f64) -> i64` | legacy |
| `lpp_float_to_int` | `lpp_float_to_int` | `core` | `(f64) -> i64` | legacy |
| `random` | `lpp_random` | `random` | `() -> i64` | legacy |
| `lpp_random` | `lpp_random` | `random` | `() -> i64` | legacy |
| `random_range` | `lpp_random_range` | `random` | `(i64, i64) -> i64` | legacy |
| `lpp_random_range` | `lpp_random_range` | `random` | `(i64, i64) -> i64` | legacy |
| `random_seed` | `lpp_random_seed` | `random` | `(i64) -> void` | legacy |
| `lpp_random_seed` | `lpp_random_seed` | `random` | `(i64) -> void` | legacy |
| `time_ms` | `lpp_time_ms` | `time` | `() -> i64` | legacy |
| `lpp_time_ms` | `lpp_time_ms` | `time` | `() -> i64` | legacy |
| `sys_mem_total` | `lpp_sys_mem_total` | `system` | `() -> i64` | legacy |
| `lpp_sys_mem_total` | `lpp_sys_mem_total` | `system` | `() -> i64` | legacy |
| `sys_mem_free` | `lpp_sys_mem_free` | `system` | `() -> i64` | legacy |
| `lpp_sys_mem_free` | `lpp_sys_mem_free` | `system` | `() -> i64` | legacy |
| `sys_cpu_usage` | `lpp_sys_cpu_usage` | `system` | `() -> i64` | legacy |
| `lpp_sys_cpu_usage` | `lpp_sys_cpu_usage` | `system` | `() -> i64` | legacy |
| `sys_uptime` | `lpp_sys_uptime` | `system` | `() -> i64` | legacy |
| `lpp_sys_uptime` | `lpp_sys_uptime` | `system` | `() -> i64` | legacy |
| `sleep` | `lpp_sleep_ms` | `core` | `(i64) -> void` | legacy |
| `sleep_ms` | `lpp_sleep_ms` | `core` | `(i64) -> void` | legacy |
| `lpp_sleep_ms` | `lpp_sleep_ms` | `core` | `(i64) -> void` | legacy |
| `exit` | `lpp_exit` | `core` | `(i64) -> void` | legacy |
| `lpp_exit` | `lpp_exit` | `core` | `(i64) -> void` | legacy |
| `lpp_str_eq` | `lpp_str_eq` | `string` | `(i64, i64) -> i64` | legacy |
| `lpp_str_cmp` | `lpp_str_cmp` | `string` | `(i64, i64) -> i64` | legacy |
| `map_put_str` | `lpp_map_put_str` | `map` | `(i64, i64, i64) -> void` | legacy |
| `map_get_str` | `lpp_map_get_str` | `map` | `(i64, i64) -> i64` | legacy |
| `map_has_str` | `lpp_map_has_str` | `map` | `(i64, i64) -> i64` | legacy |
| `map_remove_str` | `lpp_map_remove_str` | `map` | `(i64, i64) -> void` | legacy |
| `lpp_c_malloc` | `lpp_c_malloc` | `ffi` | `(i64) -> i64` | legacy |
| `lpp_c_free` | `lpp_c_free` | `ffi` | `(i64) -> void` | legacy |
| `lpp_c_load_u8` | `lpp_c_load_u8` | `ffi` | `(i64, i64) -> i64` | legacy |
| `lpp_c_store_u8` | `lpp_c_store_u8` | `ffi` | `(i64, i64, i64) -> void` | legacy |
| `lpp_c_load_i32` | `lpp_c_load_i32` | `ffi` | `(i64, i64) -> i64` | legacy |
| `lpp_c_store_i32` | `lpp_c_store_i32` | `ffi` | `(i64, i64, i64) -> void` | legacy |
| `lpp_c_load_i64` | `lpp_c_load_i64` | `ffi` | `(i64, i64) -> i64` | legacy |
| `lpp_c_store_i64` | `lpp_c_store_i64` | `ffi` | `(i64, i64, i64) -> void` | legacy |
| `dlopen` | `dlopen` | `ffi` | `(i64, i64) -> i64` | legacy |
| `dlsym` | `dlsym` | `ffi` | `(i64, i64) -> i64` | legacy |
| `dlclose` | `dlclose` | `ffi` | `(i64) -> i64` | legacy |
| `dlerror` | `dlerror` | `ffi` | `() -> i64` | legacy |
| `write_str` | `lpp_write_str` | `io` | `(i64) -> void` | legacy |
| `lpp_write_str` | `lpp_write_str` | `io` | `(i64) -> void` | legacy |
| `write_int` | `lpp_write_int` | `io` | `(i64) -> void` | legacy |
| `lpp_write_int` | `lpp_write_int` | `io` | `(i64) -> void` | legacy |
| `write_uint` | `lpp_write_uint` | `io` | `(i64) -> void` | legacy |
| `write_float` | `lpp_write_float` | `io` | `(f64) -> void` | legacy |
| `write_bool` | `lpp_write_bool` | `io` | `(bool) -> void` | legacy |
| `write_char` | `lpp_write_char` | `io` | `(i64) -> void` | legacy |
| `write_nl` | `lpp_write_nl` | `io` | `() -> void` | legacy |
| `ewrite_str` | `lpp_ewrite_str` | `core` | `(i64) -> void` | legacy |
| `eprint_str` | `lpp_eprint_str` | `io` | `(i64) -> void` | legacy |
| `flush` | `lpp_flush` | `io` | `() -> void` | legacy |
| `lpp_flush` | `lpp_flush` | `io` | `() -> void` | legacy |
| `shr_u` | `lpp_shr_u` | `core` | `(i64, i64) -> i64` | legacy |
| `shl_u` | `lpp_shl_u` | `core` | `(i64, i64) -> i64` | legacy |
| `div_u` | `lpp_div_u` | `core` | `(i64, i64) -> i64` | legacy |
| `rem_u` | `lpp_rem_u` | `core` | `(i64, i64) -> i64` | legacy |
| `lt_u` | `lpp_lt_u` | `core` | `(i64, i64) -> i64` | legacy |
| `le_u` | `lpp_le_u` | `core` | `(i64, i64) -> i64` | legacy |
| `gt_u` | `lpp_gt_u` | `core` | `(i64, i64) -> i64` | legacy |
| `ge_u` | `lpp_ge_u` | `core` | `(i64, i64) -> i64` | legacy |
| `min_u` | `lpp_min_u` | `core` | `(i64, i64) -> i64` | legacy |
| `max_u` | `lpp_max_u` | `core` | `(i64, i64) -> i64` | legacy |
| `u64_to_str` | `lpp_u64_to_str` | `core` | `(i64) -> i64` | legacy |
| `u64_to_hex` | `lpp_u64_to_hex` | `core` | `(i64) -> i64` | legacy |
| `str_to_u64` | `lpp_str_to_u64` | `string` | `(i64) -> i64` | legacy |
| `rotl64` | `lpp_rotl64` | `core` | `(i64, i64) -> i64` | legacy |
| `rotr64` | `lpp_rotr64` | `core` | `(i64, i64) -> i64` | legacy |
| `rotl32` | `lpp_rotl32` | `core` | `(i64, i64) -> i64` | legacy |
| `rotr32` | `lpp_rotr32` | `core` | `(i64, i64) -> i64` | legacy |
| `clz64` | `lpp_clz64` | `core` | `(i64) -> i64` | legacy |
| `ctz64` | `lpp_ctz64` | `core` | `(i64) -> i64` | legacy |
| `popcount64` | `lpp_popcount64` | `core` | `(i64) -> i64` | legacy |
| `bswap16` | `lpp_bswap16` | `core` | `(i64) -> i64` | legacy |
| `bswap32` | `lpp_bswap32` | `core` | `(i64) -> i64` | legacy |
| `bswap64` | `lpp_bswap64` | `core` | `(i64) -> i64` | legacy |
| `trunc_u8` | `lpp_trunc_u8` | `core` | `(i64) -> i64` | legacy |
| `trunc_u16` | `lpp_trunc_u16` | `core` | `(i64) -> i64` | legacy |
| `trunc_u32` | `lpp_trunc_u32` | `core` | `(i64) -> i64` | legacy |
| `trunc_i8` | `lpp_trunc_i8` | `core` | `(i64) -> i64` | legacy |
| `trunc_i16` | `lpp_trunc_i16` | `core` | `(i64) -> i64` | legacy |
| `trunc_i32` | `lpp_trunc_i32` | `core` | `(i64) -> i64` | legacy |
| `add_checked` | `lpp_add_checked` | `core` | `(i64, i64) -> i64` | legacy |
| `sub_checked` | `lpp_sub_checked` | `core` | `(i64, i64) -> i64` | legacy |
| `mul_checked` | `lpp_mul_checked` | `core` | `(i64, i64) -> i64` | legacy |
| `add_wrap` | `lpp_add_wrap` | `core` | `(i64, i64) -> i64` | legacy |
| `sub_wrap` | `lpp_sub_wrap` | `core` | `(i64, i64) -> i64` | legacy |
| `mul_wrap` | `lpp_mul_wrap` | `core` | `(i64, i64) -> i64` | legacy |
| `add_overflows` | `lpp_add_overflows` | `core` | `(i64, i64) -> i64` | legacy |
| `mul_overflows` | `lpp_mul_overflows` | `core` | `(i64, i64) -> i64` | legacy |
| `atomic_new` | `lpp_atomic_new` | `atomic` | `(i64) -> i64` | legacy |
| `atomic_free` | `lpp_atomic_free` | `atomic` | `(i64) -> void` | legacy |
| `atomic_load` | `lpp_atomic_load` | `atomic` | `(i64) -> i64` | legacy |
| `atomic_load_acq` | `lpp_atomic_load_acq` | `atomic` | `(i64) -> i64` | legacy |
| `atomic_load_relaxed` | `lpp_atomic_load_relaxed` | `atomic` | `(i64) -> i64` | legacy |
| `atomic_store` | `lpp_atomic_store` | `atomic` | `(i64, i64) -> void` | legacy |
| `atomic_store_rel` | `lpp_atomic_store_rel` | `atomic` | `(i64, i64) -> void` | legacy |
| `atomic_store_relaxed` | `lpp_atomic_store_relaxed` | `atomic` | `(i64, i64) -> void` | legacy |
| `atomic_add` | `lpp_atomic_add` | `atomic` | `(i64, i64) -> i64` | legacy |
| `atomic_sub` | `lpp_atomic_sub` | `atomic` | `(i64, i64) -> i64` | legacy |
| `atomic_and` | `lpp_atomic_and` | `atomic` | `(i64, i64) -> i64` | legacy |
| `atomic_or` | `lpp_atomic_or` | `atomic` | `(i64, i64) -> i64` | legacy |
| `atomic_xor` | `lpp_atomic_xor` | `atomic` | `(i64, i64) -> i64` | legacy |
| `atomic_swap` | `lpp_atomic_swap` | `atomic` | `(i64, i64) -> i64` | legacy |
| `atomic_cas` | `lpp_atomic_cas` | `atomic` | `(i64, i64, i64) -> i64` | legacy |
| `atomic_cas_weak` | `lpp_atomic_cas_weak` | `atomic` | `(i64, i64, i64) -> i64` | legacy |
| `atomic_load32` | `lpp_atomic_load32` | `atomic` | `(i64) -> i64` | legacy |
| `atomic_store32` | `lpp_atomic_store32` | `atomic` | `(i64, i64) -> void` | legacy |
| `atomic_add32` | `lpp_atomic_add32` | `atomic` | `(i64, i64) -> i64` | legacy |
| `atomic_cas32` | `lpp_atomic_cas32` | `atomic` | `(i64, i64, i64) -> i64` | legacy |
| `atomic_fence` | `lpp_atomic_fence` | `atomic` | `() -> void` | legacy |
| `atomic_fence_acq` | `lpp_atomic_fence_acq` | `atomic` | `() -> void` | legacy |
| `atomic_fence_rel` | `lpp_atomic_fence_rel` | `atomic` | `() -> void` | legacy |
| `cpu_pause` | `lpp_cpu_pause` | `system` | `() -> void` | legacy |
| `mutex_new` | `lpp_mutex_new` | `concurrency` | `() -> i64` | legacy |
| `mutex_lock` | `lpp_mutex_lock` | `concurrency` | `(i64) -> void` | legacy |
| `mutex_trylock` | `lpp_mutex_trylock` | `concurrency` | `(i64) -> i64` | legacy |
| `mutex_unlock` | `lpp_mutex_unlock` | `concurrency` | `(i64) -> void` | legacy |
| `mutex_free` | `lpp_mutex_free` | `concurrency` | `(i64) -> void` | legacy |
| `rwlock_new` | `lpp_rwlock_new` | `concurrency` | `() -> i64` | legacy |
| `rwlock_rdlock` | `lpp_rwlock_rdlock` | `concurrency` | `(i64) -> void` | legacy |
| `rwlock_wrlock` | `lpp_rwlock_wrlock` | `concurrency` | `(i64) -> void` | legacy |
| `rwlock_rdunlock` | `lpp_rwlock_rdunlock` | `concurrency` | `(i64) -> void` | legacy |
| `rwlock_wrunlock` | `lpp_rwlock_wrunlock` | `concurrency` | `(i64) -> void` | legacy |
| `rwlock_free` | `lpp_rwlock_free` | `concurrency` | `(i64) -> void` | legacy |
| `cpu_count` | `lpp_cpu_count` | `system` | `() -> i64` | legacy |
| `thread_spawn` | `lpp_thread_spawn` | `concurrency` | `(i64, i64) -> i64` | legacy |
| `thread_join` | `lpp_thread_join` | `concurrency` | `(i64) -> i64` | legacy |
| `thread_pin` | `lpp_thread_pin` | `concurrency` | `(i64) -> i64` | legacy |
| `thread_id` | `lpp_thread_id` | `concurrency` | `() -> i64` | legacy |
| `list_insert` | `lpp_list_insert` | `list` | `(i64, i64, i64) -> void` | legacy |
| `list_remove` | `lpp_list_remove` | `list` | `(i64, i64) -> i64` | legacy |
| `list_reserve` | `lpp_list_reserve` | `list` | `(i64, i64) -> void` | legacy |
| `list_capacity` | `lpp_list_capacity` | `list` | `(i64) -> i64` | legacy |
| `list_clear` | `lpp_list_clear` | `list` | `(i64) -> void` | legacy |
| `list_truncate` | `lpp_list_truncate` | `list` | `(i64, i64) -> void` | legacy |
| `list_swap` | `lpp_list_swap` | `list` | `(i64, i64, i64) -> void` | legacy |
| `list_reverse` | `lpp_list_reverse` | `list` | `(i64) -> void` | legacy |
| `list_sort` | `lpp_list_sort` | `list` | `(i64) -> void` | legacy |
| `list_sort_desc` | `lpp_list_sort_desc` | `list` | `(i64) -> void` | legacy |
| `list_sort_u` | `lpp_list_sort_u` | `list` | `(i64) -> void` | legacy |
| `list_index_of` | `lpp_list_index_of` | `list` | `(i64, i64) -> i64` | legacy |
| `list_binary_search` | `lpp_list_binary_search` | `list` | `(i64, i64) -> i64` | legacy |
| `list_extend` | `lpp_list_extend` | `list` | `(i64, i64) -> void` | legacy |
| `map_keys` | `lpp_map_keys` | `map` | `(i64) -> i64` | legacy |
| `map_values` | `lpp_map_values` | `map` | `(i64) -> i64` | legacy |
| `map_clear` | `lpp_map_clear` | `map` | `(i64) -> void` | legacy |
| `map_capacity` | `lpp_map_capacity` | `map` | `(i64) -> i64` | legacy |
| `rng_new` | `lpp_rng_new` | `random` | `(i64) -> i64` | legacy |
| `rng_next` | `lpp_rng_next` | `random` | `(i64) -> i64` | legacy |
| `rng_range` | `lpp_rng_range` | `random` | `(i64, i64, i64) -> i64` | legacy |
| `rng_float` | `lpp_rng_float` | `random` | `(i64) -> f64` | legacy |
| `rng_free` | `lpp_rng_free` | `random` | `(i64) -> void` | legacy |
| `clock_new` | `lpp_clock_new` | `time` | `(i64) -> i64` | legacy |
| `clock_now` | `lpp_clock_now` | `time` | `(i64) -> i64` | legacy |
| `clock_advance` | `lpp_clock_advance` | `time` | `(i64, i64) -> void` | legacy |
| `clock_free` | `lpp_clock_free` | `time` | `(i64) -> void` | legacy |
| `prefetch` | `lpp_prefetch` | `core` | `(i64) -> void` | legacy |
| `prefetch_read` | `lpp_prefetch` | `core` | `(i64) -> void` | legacy |
| `prefetch_write` | `lpp_prefetch_write` | `core` | `(i64) -> void` | legacy |
