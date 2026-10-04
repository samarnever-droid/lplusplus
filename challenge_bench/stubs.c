int _fltused = 0;
int lpp_main(void);
void lpp_free_str(void* p) {}
void* lpp_net_dial(void* a, void* b) { return 0; }
void* lpp_net_dial_udp(void* a, void* b) { return 0; }
void* lpp_net_listen_udp(void* a) { return 0; }
int lpp_net_accept_timeout(void* a, void* b) { return -1; }
int lpp_net_send_all(void* a, void* b, long c) { return -1; }
int lpp_net_set_deadline(void* a, long b) { return -1; }
int lpp_net_set_timeout(void* a, long b) { return -1; }
int lpp_net_set_keepalive(void* a) { return -1; }
void* lpp_net_resolve(void* a) { return 0; }
void* lpp_json_get_obj(void* a, void* b) { return 0; }
int lpp_file_copy(void* a, void* b) { return -1; }
int lpp_file_move(void* a, void* b) { return -1; }
long lpp_vec_i64_checksum(void* a, long b) { return 0; }
void* lpp_float_to_str(double d) { return 0; }
void* lpp_bool_to_str(int b) { return 0; }
int lpp_gui_draw_circle(int a, int b, int c, int d) { return 0; }
int lpp_gui_draw_line(int a, int b, int c, int d, int e) { return 0; }
int lpp_gui_measure_text_width(void* a, int b) { return 0; }
int lpp_gui_dialog_message(void* a) { return 0; }
long lpp_gui_get_ticks_ms(void) { return 0; }
