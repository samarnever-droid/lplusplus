/* SamarOS — framebuffer driver + the small set of raster primitives that
 * the L++ user-interface layer builds everything else out of.
 *
 * Everything is drawn into a 32bpp back buffer in main memory; gfx_present()
 * pushes it to the VESA linear framebuffer in one pass.  All primitives obey
 * the current clip rectangle so windows can scissor their own content.
 */
#include "samar.h"
#include "font.h"

static u32 *back;      /* back buffer                       */
static u32 *cache;     /* cached layer (wallpaper, blur)    */
static u32 *lfb;       /* hardware linear framebuffer       */
static int  W, H, PITCH_PX;

static int clip_x0, clip_y0, clip_x1, clip_y1;

void gfx_init(void)
{
    back  = (u32 *)BACKBUFFER_ADDR;
    cache = (u32 *)WALLCACHE_ADDR;
    lfb   = (u32 *)g_boot->fb_addr;
    W     = (int)g_boot->fb_width;
    H     = (int)g_boot->fb_height;
    PITCH_PX = (int)(g_boot->fb_pitch >> 2);
    clip_x0 = 0; clip_y0 = 0; clip_x1 = W; clip_y1 = H;
}

int fb_width(void)  { return W; }
int fb_height(void) { return H; }

void gfx_clip(int x, int y, int w, int h)
{
    int x1 = x + w, y1 = y + h;
    clip_x0 = x  < 0 ? 0 : x;
    clip_y0 = y  < 0 ? 0 : y;
    clip_x1 = x1 > W ? W : x1;
    clip_y1 = y1 > H ? H : y1;
    if (clip_x1 < clip_x0) clip_x1 = clip_x0;
    if (clip_y1 < clip_y0) clip_y1 = clip_y0;
}

void gfx_clip_reset(void) { clip_x0 = 0; clip_y0 = 0; clip_x1 = W; clip_y1 = H; }
int  gfx_clip_x(void) { return clip_x0; }
int  gfx_clip_y(void) { return clip_y0; }
int  gfx_clip_w(void) { return clip_x1 - clip_x0; }
int  gfx_clip_h(void) { return clip_y1 - clip_y0; }

void gfx_pixel(int x, int y, int color)
{
    if (x < clip_x0 || x >= clip_x1 || y < clip_y0 || y >= clip_y1) return;
    back[y * W + x] = (u32)color;
}

int gfx_get(int x, int y)
{
    if (x < 0 || x >= W || y < 0 || y >= H) return 0;
    return (int)back[y * W + x];
}

/* src over dst with an 0..255 coverage value */
static inline u32 blend(u32 dst, u32 src, u32 a)
{
    u32 ia = 255u - a;
    u32 rb = ((((src & 0x00FF00FFu) * a) + ((dst & 0x00FF00FFu) * ia)) >> 8) & 0x00FF00FFu;
    u32 g  = ((((src & 0x0000FF00u) * a) + ((dst & 0x0000FF00u) * ia)) >> 8) & 0x0000FF00u;
    return rb | g;
}

void gfx_blend(int x, int y, int color, int alpha)
{
    if (alpha <= 0) return;
    if (x < clip_x0 || x >= clip_x1 || y < clip_y0 || y >= clip_y1) return;
    if (alpha >= 255) { back[y * W + x] = (u32)color; return; }
    u32 *p = &back[y * W + x];
    *p = blend(*p, (u32)color, (u32)alpha);
}

void gfx_fill_rect(int x, int y, int w, int h, int color)
{
    int x0 = x < clip_x0 ? clip_x0 : x;
    int y0 = y < clip_y0 ? clip_y0 : y;
    int x1 = x + w > clip_x1 ? clip_x1 : x + w;
    int y1 = y + h > clip_y1 ? clip_y1 : y + h;
    for (int yy = y0; yy < y1; yy++) {
        u32 *row = &back[yy * W];
        for (int xx = x0; xx < x1; xx++) row[xx] = (u32)color;
    }
}

void gfx_blend_rect(int x, int y, int w, int h, int color, int alpha)
{
    if (alpha <= 0) return;
    if (alpha >= 255) { gfx_fill_rect(x, y, w, h, color); return; }
    int x0 = x < clip_x0 ? clip_x0 : x;
    int y0 = y < clip_y0 ? clip_y0 : y;
    int x1 = x + w > clip_x1 ? clip_x1 : x + w;
    int y1 = y + h > clip_y1 ? clip_y1 : y + h;
    for (int yy = y0; yy < y1; yy++) {
        u32 *row = &back[yy * W];
        for (int xx = x0; xx < x1; xx++) row[xx] = blend(row[xx], (u32)color, (u32)alpha);
    }
}

/* horizontal linear gradient band, used by the desktop + headers */
void gfx_grad_rect(int x, int y, int w, int h, int c0, int c1, int vertical)
{
    int r0 = (c0 >> 16) & 255, g0 = (c0 >> 8) & 255, b0 = c0 & 255;
    int r1 = (c1 >> 16) & 255, g1 = (c1 >> 8) & 255, b1 = c1 & 255;
    int span = vertical ? h : w;
    if (span <= 0) return;
    for (int i = 0; i < span; i++) {
        int r = r0 + (r1 - r0) * i / span;
        int g = g0 + (g1 - g0) * i / span;
        int b = b0 + (b1 - b0) * i / span;
        int c = (r << 16) | (g << 8) | b;
        if (vertical) gfx_fill_rect(x, y + i, w, 1, c);
        else          gfx_fill_rect(x + i, y, 1, h, c);
    }
}

/* ---- cached layer -------------------------------------------------- */
void gfx_cache_store(void) { k_memcpy(cache, back, (unsigned)(W * H * 4)); }
void gfx_cache_restore(void) { k_memcpy(back, cache, (unsigned)(W * H * 4)); }

void gfx_cache_restore_rect(int x, int y, int w, int h)
{
    if (x < 0) { w += x; x = 0; }
    if (y < 0) { h += y; y = 0; }
    if (x + w > W) w = W - x;
    if (y + h > H) h = H - y;
    for (int yy = 0; yy < h; yy++)
        k_memcpy(&back[(y + yy) * W + x], &cache[(y + yy) * W + x], (unsigned)(w * 4));
}

void gfx_clear(int color)
{
    u32 c = (u32)color;
    int n = W * H;
    for (int i = 0; i < n; i++) back[i] = c;
}

/* ---- pointer overlay ------------------------------------------------
 * The cursor is composited straight onto the finished frame: we stash the
 * pixels underneath it so the next frame can put them back without a full
 * recomposite.  That is what keeps pointer motion smooth while the desktop
 * itself only repaints when something actually changes.
 */
#define UNDER_MAX 48
static u32 under[UNDER_MAX * UNDER_MAX];
static int ux, uy, uw, uh, uvalid;

void gfx_save_under(int x, int y, int w, int h)
{
    if (w > UNDER_MAX) w = UNDER_MAX;
    if (h > UNDER_MAX) h = UNDER_MAX;
    ux = x; uy = y; uw = w; uh = h; uvalid = 1;
    for (int row = 0; row < h; row++) {
        int yy = y + row;
        if (yy < 0 || yy >= H) continue;
        for (int col = 0; col < w; col++) {
            int xx = x + col;
            if (xx < 0 || xx >= W) continue;
            under[row * UNDER_MAX + col] = back[yy * W + xx];
        }
    }
}

void gfx_restore_under(void)
{
    if (!uvalid) return;
    for (int row = 0; row < uh; row++) {
        int yy = uy + row;
        if (yy < 0 || yy >= H) continue;
        for (int col = 0; col < uw; col++) {
            int xx = ux + col;
            if (xx < 0 || xx >= W) continue;
            back[yy * W + xx] = under[row * UNDER_MAX + col];
        }
    }
    uvalid = 0;
}

void gfx_present_rect(int x, int y, int w, int h)
{
    if (x < 0) { w += x; x = 0; }
    if (y < 0) { h += y; y = 0; }
    if (x + w > W) w = W - x;
    if (y + h > H) h = H - y;
    if (w <= 0 || h <= 0) return;
    for (int row = 0; row < h; row++)
        k_memcpy(&lfb[(y + row) * PITCH_PX + x], &back[(y + row) * W + x], (unsigned)(w * 4));
}

void gfx_present(void)
{
    if (PITCH_PX == W) {
        k_memcpy(lfb, back, (unsigned)(W * H * 4));
    } else {
        for (int y = 0; y < H; y++)
            k_memcpy(&lfb[y * PITCH_PX], &back[y * W], (unsigned)(W * 4));
    }
}

/* ---- text ---------------------------------------------------------- */
static const sm_face *face_of(int id)
{
    if (id < 0 || id >= sm_face_count) id = 1;
    return &sm_faces[id];
}

int text_height(int font) { return face_of(font)->line_height; }
int text_ascent(int font) { return face_of(font)->ascent; }

int text_width(const char *s, int font)
{
    const sm_face *f = face_of(font);
    int w = 0;
    if (!s) return 0;
    for (const unsigned char *p = (const unsigned char *)s; *p; p++) {
        int c = *p;
        if (c < 32 || c > 126) c = '?';
        int gi = f->index[c - 32];
        if (gi < 0) gi = f->index['?' - 32];
        if (gi < 0) continue;
        w += f->glyphs[gi].adv;
    }
    return w;
}

/* Draws `s` with (x, y) as the top-left of the line box (glyph offsets are
 * baked relative to the line top by tools/genfont.py).  Returns the advance. */
int text_draw(int x, int y, const char *s, int color, int alpha, int font)
{
    const sm_face *f = face_of(font);
    int pen = x;
    if (!s) return 0;
    for (const unsigned char *p = (const unsigned char *)s; *p; p++) {
        int c = *p;
        if (c < 32 || c > 126) c = '?';
        int gi = f->index[c - 32];
        if (gi < 0) gi = f->index['?' - 32];
        if (gi < 0) continue;
        const sm_glyph *g = &f->glyphs[gi];
        const unsigned char *bits = f->bits + g->off;
        int gx = pen + g->bx;
        int gy = y + g->by;
        for (int row = 0; row < g->h; row++) {
            int yy = gy + row;
            if (yy < clip_y0 || yy >= clip_y1) continue;
            u32 *dst = &back[yy * W];
            const unsigned char *srow = bits + row * g->w;
            for (int col = 0; col < g->w; col++) {
                int xx = gx + col;
                if (xx < clip_x0 || xx >= clip_x1) continue;
                u32 a = srow[col];
                if (!a) continue;
                if (alpha < 255) a = (a * (u32)alpha) >> 8;
                if (!a) continue;
                dst[xx] = blend(dst[xx], (u32)color, a);
            }
        }
        pen += g->adv;
    }
    return pen - x;
}
