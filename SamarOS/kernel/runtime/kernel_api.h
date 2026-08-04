/* SamarOS — the C surface the L++ kernel is compiled against.
 *
 * Everything the generated kernel code can touch is declared here: the
 * freestanding L++ runtime (kruntime.c) plus the hardware drivers
 * (gfx.c, input.c).  The `extern "C"` blocks in kernel/src/sys.lpp mirror
 * these prototypes one-for-one.
 */
#ifndef SAMAR_KERNEL_API_H
#define SAMAR_KERNEL_API_H

/* ---- L++ runtime ---------------------------------------------------- */
void *lpp_alloc(int size);
int   heap_used(void);
int   heap_total(void);

int   str_len(const char *s);
int   len(const char *s);
char *str_concat(const char *a, const char *b);
int   str_eq(const char *a, const char *b);
int   str_starts_with(const char *s, const char *prefix);
int   str_index_of(const char *s, int ch);
int   char_at(const char *s, int i);
char *substr(const char *s, int start, int count);
char *chr(int c);
char *int_to_str(int v);
char *pad2(int v);

void *list_new(void);
void  list_push(void *h, int v);
int   list_get(void *h, int i);
void  list_set(void *h, int i, int v);
int   list_len(void *h);
void  list_remove(void *h, int i);
void  list_insert(void *h, int i, int v);
void  list_clear(void *h);

int   lpp_abs(int v);
int   lpp_min(int a, int b);
int   lpp_max(int a, int b);
int   lpp_clamp(int v, int lo, int hi);
int   isqrt(int v);
int   sin_deg(int deg);
int   cos_deg(int deg);

/* ---- display -------------------------------------------------------- */
int  fb_width(void);
int  fb_height(void);
void gfx_clear(int color);
void gfx_pixel(int x, int y, int color);
int  gfx_get(int x, int y);
void gfx_blend(int x, int y, int color, int alpha);
void gfx_fill_rect(int x, int y, int w, int h, int color);
void gfx_blend_rect(int x, int y, int w, int h, int color, int alpha);
void gfx_grad_rect(int x, int y, int w, int h, int c0, int c1, int vertical);
void gfx_clip(int x, int y, int w, int h);
void gfx_clip_reset(void);
int  gfx_clip_x(void);
int  gfx_clip_y(void);
int  gfx_clip_w(void);
int  gfx_clip_h(void);
void gfx_cache_store(void);
void gfx_cache_restore(void);
void gfx_cache_restore_rect(int x, int y, int w, int h);
void gfx_present(void);
void gfx_present_rect(int x, int y, int w, int h);
void gfx_save_under(int x, int y, int w, int h);
void gfx_restore_under(void);

int  text_draw(int x, int y, const char *s, int color, int alpha, int font);
int  text_width(const char *s, int font);
int  text_height(int font);
int  text_ascent(int font);

/* ---- input / time ---------------------------------------------------- */
void input_poll(void);
int  key_pop(void);
int  key_ctrl(void);
int  key_shift(void);
int  mouse_x(void);
int  mouse_y(void);
int  mouse_buttons(void);
int  mouse_wheel(void);
int  time_ms(void);
int  rtc_hour(void);
int  rtc_min(void);
int  rtc_sec(void);
int  rtc_day(void);
int  rtc_month(void);
int  rtc_year(void);
int  rtc_weekday(void);

/* ---- machine --------------------------------------------------------- */
int  mem_total_kb(void);
int  cpu_mhz(void);
int  boot_drive(void);
void cpu_halt(void);
void debug_out(const char *s);

/* entry point produced by lppc from `def main()` */
void samar_main(void);

#endif /* SAMAR_KERNEL_API_H */
