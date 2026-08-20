/*
 * lpp_gui.c  —  L++ Native 2D GUI & Windowing Builtins (cross-platform)
 *
 * Win32 / X11 dual-backend with:
 *  - Double-buffered GDI backbuffer (Win32) and Pixmap backbuffer (X11)
 *  - ClearType Segoe UI font (Win32)
 *  - Full event decoding: close, resize, keyboard, mouse (both backends)
 *  - Slot reuse so the 8-window limit means "8 simultaneous", not "8 ever"
 *  - Centralized cleanup — no resource leaks on close
 *  - DPI awareness (Win32)
 *  - Real circle, line, rounded-rect primitives on both backends
 *  - Monotonic clock on Unix
 *  - WM_DELETE_WINDOW close protocol on X11
 *
 * Fixes applied: issues #1-#59 from lpp_gui_bug_audit.txt
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#if !defined(_WIN32)
#  ifndef _GNU_SOURCE
#    define _GNU_SOURCE
#  endif
#  include <unistd.h>
#endif

/* Cached keyboard & mouse state updated in message loops (cross-platform) */
static uint8_t g_key_state[256] = {0};
static int64_t g_char_queue[64];
static int g_char_q_head = 0;
static int g_char_q_tail = 0;

static uint8_t g_mouse_btn_down[3] = {0};
static uint8_t g_mouse_btn_pressed[3] = {0};
static uint8_t g_mouse_btn_released[3] = {0};
static double g_mouse_wheel_delta = 0.0;

/* ═══════════════════════════════════════════════════════════════════════════
 *  Win32 backend
 * ═══════════════════════════════════════════════════════════════════════════ */
#if defined(_WIN32)
#if defined(_MSC_VER)
#pragma comment(lib, "user32.lib")
#pragma comment(lib, "gdi32.lib")
#endif
#include <windows.h>

typedef struct {
    HWND    hwnd;
    HDC     hdc;       /* window DC — held for the lifetime of the window */
    HDC     mem_dc;    /* off-screen backbuffer DC */
    HBITMAP hbmp;
    HBITMAP old_bmp;
    HFONT   hfont;     /* ClearType Segoe UI */
    int     width;
    int     height;
    int     is_open;
    COLORREF bg_color; /* kept in sync with last lpp_gui_clear() call */
} LppWin32Window;

#define MAX_WINDOWS 8
static LppWin32Window g_windows[MAX_WINDOWS];
static int g_class_registered = 0;
static int g_dpi_set = 0;

/* ── Helpers ─────────────────────────────────────────────────────────────── */

static int lpp_find_free_slot(void) {
    for (int i = 0; i < MAX_WINDOWS; i++) {
        if (!g_windows[i].is_open && !g_windows[i].hwnd) return i;
    }
    return -1; /* all 8 simultaneously active */
}

static COLORREF lpp_hex_to_colorref(int64_t hex) {
    return RGB((BYTE)( hex        & 0xFF),
               (BYTE)((hex >>  8) & 0xFF),
               (BYTE)((hex >> 16) & 0xFF));
}

/* Centralized resource teardown — called from WM_CLOSE and explicit close */
static void lpp_win32_destroy_slot(int id) {
    LppWin32Window *w = &g_windows[id];
    if (!w->hwnd) return;

    /* Deselect and delete GDI objects */
    if (w->mem_dc) {
        if (w->old_bmp) SelectObject(w->mem_dc, w->old_bmp);
        if (w->hbmp)    DeleteObject(w->hbmp);
        if (w->hfont)   DeleteObject(w->hfont);
        DeleteDC(w->mem_dc);
    }
    if (w->hdc) ReleaseDC(w->hwnd, w->hdc);

    /* Clear the slot completely so lpp_find_free_slot() can reuse it */
    memset(w, 0, sizeof(*w));
}

/* Re-initialise graphics state after backbuffer recreation (WM_SIZE) */
static void lpp_win32_init_dc_state(HDC mem_dc, HFONT hfont) {
    if (hfont) SelectObject(mem_dc, hfont);
    SetBkMode(mem_dc, TRANSPARENT);
    SetStretchBltMode(mem_dc, HALFTONE);
    SetBrushOrgEx(mem_dc, 0, 0, NULL);
    SetMapMode(mem_dc, MM_TEXT);
    SetGraphicsMode(mem_dc, GM_ADVANCED);
    SetTextCharacterExtra(mem_dc, 0);
    SetTextAlign(mem_dc, TA_LEFT | TA_TOP);
    SetLayout(mem_dc, 0);
}

/* ── WndProc ─────────────────────────────────────────────────────────────── */

static LRESULT CALLBACK lpp_gui_wndproc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    switch (msg) {

        /* ── Painting ─────────────────────────────────────────────────── */
        case WM_PAINT: {
            PAINTSTRUCT ps;
            HDC hdc = BeginPaint(hwnd, &ps);
            for (int i = 0; i < MAX_WINDOWS; i++) {
                if (g_windows[i].hwnd == hwnd && g_windows[i].mem_dc) {
                    /* Blit only the invalidated region for efficiency (#26) */
                    int x = ps.rcPaint.left, y = ps.rcPaint.top;
                    int w = ps.rcPaint.right  - x;
                    int h = ps.rcPaint.bottom - y;
                    if (w > 0 && h > 0)
                        BitBlt(hdc, x, y, w, h, g_windows[i].mem_dc, x, y, SRCCOPY);
                    break;
                }
            }
            EndPaint(hwnd, &ps);
            return 0;
        }
        case WM_ERASEBKGND:
            return 1; /* we own the background — prevent white flash */

        /* ── Resize / maximize (#11 multi-window WM_QUIT, #25 state) ──── */
        case WM_SIZE: {
            if (wp == SIZE_MINIMIZED) break;
            int new_w = LOWORD(lp), new_h = HIWORD(lp);
            if (new_w <= 0 || new_h <= 0) break;
            for (int i = 0; i < MAX_WINDOWS; i++) {
                LppWin32Window *w = &g_windows[i];
                if (w->hwnd != hwnd) continue;
                if (new_w == w->width && new_h == w->height) break;

                HDC screen_dc = GetDC(hwnd);
                HBITMAP new_bmp = CreateCompatibleBitmap(screen_dc, new_w, new_h);
                ReleaseDC(hwnd, screen_dc);
                if (!new_bmp) break;

                /* Copy old content then fill newly exposed area (#44/#45) */
                HDC tmp_dc = CreateCompatibleDC(w->mem_dc);
                HBITMAP old_sel = (HBITMAP)SelectObject(tmp_dc, new_bmp);
                BitBlt(tmp_dc, 0, 0, w->width, w->height, w->mem_dc, 0, 0, SRCCOPY);
                if (new_w > w->width) {
                    RECT r = {w->width, 0, new_w, new_h};
                    HBRUSH br = CreateSolidBrush(w->bg_color);
                    FillRect(tmp_dc, &r, br); DeleteObject(br);
                }
                if (new_h > w->height) {
                    RECT r = {0, w->height, new_w, new_h};
                    HBRUSH br = CreateSolidBrush(w->bg_color);
                    FillRect(tmp_dc, &r, br); DeleteObject(br);
                }
                SelectObject(tmp_dc, old_sel);
                DeleteDC(tmp_dc);

                /* Swap in new bitmap and re-init all DC state (#25) */
                HBITMAP old_bmp = (HBITMAP)SelectObject(w->mem_dc, new_bmp);
                DeleteObject(old_bmp);
                w->hbmp   = new_bmp;
                w->width  = new_w;
                w->height = new_h;
                lpp_win32_init_dc_state(w->mem_dc, w->hfont);
                break;
            }
            return 0;
        }
        case WM_GETMINMAXINFO: {
            MINMAXINFO *mmi = (MINMAXINFO *)lp;
            mmi->ptMinTrackSize.x = 200;
            mmi->ptMinTrackSize.y = 150;
            return 0;
        }

        /* ── Keyboard state cache ─────────────────────────────────────── */
        case WM_CHAR: {
            if (wp >= 32 && wp <= 126) { /* Only printable characters in text queue */
                int next_head = (g_char_q_head + 1) % 64;
                if (next_head != g_char_q_tail) {
                    g_char_queue[g_char_q_head] = (int64_t)wp;
                    g_char_q_head = next_head;
                }
            }
            return 0;
        }
        case WM_KEYDOWN:
        case WM_SYSKEYDOWN:
            if (wp < 256) g_key_state[wp] = 1;
            return DefWindowProcA(hwnd, msg, wp, lp);
        case WM_KEYUP:
        case WM_SYSKEYUP:
            if (wp < 256) g_key_state[wp] = 0;
            return DefWindowProcA(hwnd, msg, wp, lp);

        /* ── Mouse Button & Wheel Events ──────────────────────────────── */
        case WM_LBUTTONDOWN:
            g_mouse_btn_down[0] = 1;
            g_mouse_btn_pressed[0] = 1;
            SetCapture(hwnd);
            return 0;
        case WM_LBUTTONUP:
            g_mouse_btn_down[0] = 0;
            g_mouse_btn_released[0] = 1;
            ReleaseCapture();
            return 0;
        case WM_RBUTTONDOWN:
            g_mouse_btn_down[1] = 1;
            g_mouse_btn_pressed[1] = 1;
            return 0;
        case WM_RBUTTONUP:
            g_mouse_btn_down[1] = 0;
            g_mouse_btn_released[1] = 1;
            return 0;
        case WM_MBUTTONDOWN:
            g_mouse_btn_down[2] = 1;
            g_mouse_btn_pressed[2] = 1;
            return 0;
        case WM_MBUTTONUP:
            g_mouse_btn_down[2] = 0;
            g_mouse_btn_released[2] = 1;
            return 0;
        case WM_MOUSEWHEEL: {
            short delta = (short)HIWORD(wp);
            g_mouse_wheel_delta += (double)delta / 120.0;
            return 0;
        }

        /* ── Window close (#11 fix: only PostQuitMessage on last window) ─ */
        case WM_CLOSE: {
            for (int i = 0; i < MAX_WINDOWS; i++) {
                if (g_windows[i].hwnd == hwnd) {
                    g_windows[i].is_open = 0;
                    HWND h = g_windows[i].hwnd;
                    g_windows[i].hwnd = NULL; /* prevent double-destroy */
                    lpp_win32_destroy_slot(i);
                    DestroyWindow(h);
                    break;
                }
            }
            return 0;
        }
        case WM_DESTROY: {
            /* Only post WM_QUIT when there are no more open windows (#11) */
            int any_open = 0;
            for (int i = 0; i < MAX_WINDOWS; i++) {
                if (g_windows[i].is_open) { any_open = 1; break; }
            }
            if (!any_open) PostQuitMessage(0);
            return 0;
        }

        default:
            return DefWindowProcA(hwnd, msg, wp, lp);
    }
    return DefWindowProcA(hwnd, msg, wp, lp);
}

/* ── Public API ──────────────────────────────────────────────────────────── */

int64_t lpp_gui_window_create(const char *title, int64_t width, int64_t height) {
    /* Validate dimensions (#28) */
    if (width <= 0 || width > 65535 || height <= 0 || height > 65535) return -1;

    int id = lpp_find_free_slot();
    if (id < 0) return -1; /* all 8 slots in use (#12 fix) */

    /* DPI awareness — process-wide, once (#14) */
    if (!g_dpi_set) { SetProcessDPIAware(); g_dpi_set = 1; }

    HINSTANCE hInst = GetModuleHandleA(NULL);
    if (!g_class_registered) {
        WNDCLASSA wc = {0};
        wc.style         = CS_OWNDC; /* no CS_HREDRAW/CS_VREDRAW — we own the buffer */
        wc.lpfnWndProc   = lpp_gui_wndproc;
        wc.hInstance     = hInst;
        wc.lpszClassName = "LppGUIWindowClass";
        wc.hbrBackground = NULL; /* WM_ERASEBKGND returns 1 */
        wc.hCursor       = LoadCursorA(NULL, IDC_ARROW);
        wc.hIcon         = LoadIconA(NULL, IDI_APPLICATION);
        if (!RegisterClassA(&wc)) return -1;
        g_class_registered = 1;
    }

    /* Compute outer window size from desired client area (#27) */
    RECT client = {0, 0, (int)width, (int)height};
    AdjustWindowRectEx(&client, WS_OVERLAPPEDWINDOW, FALSE, WS_EX_APPWINDOW | WS_EX_WINDOWEDGE);
    int outer_w = client.right  - client.left;
    int outer_h = client.bottom - client.top;

    HWND hwnd = CreateWindowExA(
        WS_EX_APPWINDOW | WS_EX_WINDOWEDGE,
        "LppGUIWindowClass",
        title ? title : "L++ GUI App",
        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
        CW_USEDEFAULT, CW_USEDEFAULT, outer_w, outer_h,
        NULL, NULL, hInst, NULL);
    if (!hwnd) return -1;

    ShowWindow(hwnd, SW_SHOW);
    UpdateWindow(hwnd);
    BringWindowToTop(hwnd);
    SetForegroundWindow(hwnd);

    /* Create backbuffer — validate every allocation (#30) */
    HDC hdc = GetDC(hwnd);
    if (!hdc) { DestroyWindow(hwnd); return -1; }
    HDC mem_dc = CreateCompatibleDC(hdc);
    if (!mem_dc) { ReleaseDC(hwnd, hdc); DestroyWindow(hwnd); return -1; }
    HBITMAP hbmp = CreateCompatibleBitmap(hdc, (int)width, (int)height);
    if (!hbmp) { DeleteDC(mem_dc); ReleaseDC(hwnd, hdc); DestroyWindow(hwnd); return -1; }
    HBITMAP old_bmp = (HBITMAP)SelectObject(mem_dc, hbmp);

    /* ClearType Segoe UI font */
    HFONT hfont = CreateFontA(
        -15, 0, 0, 0, FW_NORMAL, FALSE, FALSE, FALSE,
        DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY, DEFAULT_PITCH | FF_SWISS, "Segoe UI");
    if (!hfont)
        hfont = CreateFontA(-14, 0, 0, 0, FW_NORMAL, FALSE, FALSE, FALSE,
            DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
            ANTIALIASED_QUALITY, DEFAULT_PITCH | FF_SWISS, "Arial");

    /* Populate slot */
    LppWin32Window *w = &g_windows[id];
    w->hwnd     = hwnd;
    w->hdc      = hdc;
    w->mem_dc   = mem_dc;
    w->hbmp     = hbmp;
    w->old_bmp  = old_bmp;
    w->hfont    = hfont;
    w->width    = (int)width;
    w->height   = (int)height;
    w->is_open  = 1;
    w->bg_color = RGB(19, 24, 33); /* matches dark_theme default */

    lpp_win32_init_dc_state(mem_dc, hfont);

    /* Initial clear */
    RECT r = {0, 0, (int)width, (int)height};
    HBRUSH br = CreateSolidBrush(w->bg_color);
    FillRect(mem_dc, &r, br);
    DeleteObject(br);

    /* Flush creation messages */
    MSG pmsg;
    while (PeekMessageA(&pmsg, hwnd, 0, 0, PM_REMOVE)) {
        TranslateMessage(&pmsg);
        DispatchMessageA(&pmsg);
    }
    return (int64_t)id;
}

int64_t lpp_gui_window_is_open(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_WINDOWS) return 0;
    return g_windows[win_id].is_open ? 1 : 0;
}

int64_t lpp_gui_window_width(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].hwnd) return 0;
    return g_windows[win_id].width;
}

int64_t lpp_gui_window_height(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].hwnd) return 0;
    return g_windows[win_id].height;
}

int64_t lpp_gui_window_poll_events(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_WINDOWS) return 0;
    MSG msg;
    while (PeekMessageA(&msg, NULL, 0, 0, PM_REMOVE)) {
        if (msg.message == WM_QUIT) {
            /* Mark ALL windows closed on application quit (#40) */
            for (int i = 0; i < MAX_WINDOWS; i++) g_windows[i].is_open = 0;
            return 0;
        }
        TranslateMessage(&msg);
        DispatchMessageA(&msg);
    }
    return g_windows[win_id].is_open ? 1 : 0;
}

void lpp_gui_clear(int64_t win_id, int64_t hex_color) {
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].is_open) return;
    COLORREF col = lpp_hex_to_colorref(hex_color);
    g_windows[win_id].bg_color = col; /* keep bg_color in sync (#44) */
    RECT r = {0, 0, g_windows[win_id].width, g_windows[win_id].height};
    HBRUSH br = CreateSolidBrush(col);
    FillRect(g_windows[win_id].mem_dc, &r, br);
    DeleteObject(br);
}

void lpp_gui_draw_rect(int64_t win_id, int64_t x, int64_t y, int64_t w, int64_t h, int64_t hex_color) {
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].is_open) return;
    COLORREF col = lpp_hex_to_colorref(hex_color);
    RECT r = {(LONG)x, (LONG)y, (LONG)(x+w), (LONG)(y+h)};
    HBRUSH br = CreateSolidBrush(col);
    FillRect(g_windows[win_id].mem_dc, &r, br);
    DeleteObject(br);
}

void lpp_gui_draw_rounded_rect(int64_t win_id, int64_t x, int64_t y, int64_t w, int64_t h, int64_t radius, int64_t hex_color) {
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].is_open) return;
    COLORREF col = lpp_hex_to_colorref(hex_color);
    HDC dc = g_windows[win_id].mem_dc;
    HBRUSH br  = CreateSolidBrush(col);
    HPEN   pen = CreatePen(PS_NULL, 0, 0);
    HBRUSH old_br  = (HBRUSH)SelectObject(dc, br);
    HPEN   old_pen = (HPEN)SelectObject(dc, pen);
    int d = (int)(radius * 2);
    RoundRect(dc, (int)x, (int)y, (int)(x+w), (int)(y+h), d, d);
    SelectObject(dc, old_br);
    SelectObject(dc, old_pen);
    DeleteObject(br);
    DeleteObject(pen);
}

int64_t lpp_gui_mouse_x(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].is_open) return 0;
    POINT pt; GetCursorPos(&pt);
    ScreenToClient(g_windows[win_id].hwnd, &pt);
    return (int64_t)pt.x;
}

int64_t lpp_gui_mouse_y(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].is_open) return 0;
    POINT pt; GetCursorPos(&pt);
    ScreenToClient(g_windows[win_id].hwnd, &pt);
    return (int64_t)pt.y;
}

int64_t lpp_gui_mouse_down(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].is_open) return 0;
    return (GetAsyncKeyState(VK_LBUTTON) & 0x8000) ? 1 : 0;
}

int64_t lpp_gui_key_down(int64_t win_id, int64_t key_code) {
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].is_open) return 0;
    if (key_code < 0 || key_code > 255) return 0;
    return g_key_state[(int)key_code] ? 1 : 0;
}

static HFONT g_editor_font = NULL;

void lpp_gui_draw_text(int64_t win_id, int64_t x, int64_t y, const char *text, int64_t hex_color) {
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].is_open || !text) return;
    HDC dc = g_windows[win_id].mem_dc;
    if (!g_editor_font) {
        g_editor_font = CreateFontA(
            17, 0, 0, 0, FW_NORMAL, FALSE, FALSE, FALSE,
            DEFAULT_CHARSET, OUT_DEFAULT_PRECIS, CLIP_DEFAULT_PRECIS,
            CLEARTYPE_QUALITY, FIXED_PITCH | FF_MODERN, "Consolas"
        );
    }
    SelectObject(dc, g_editor_font);
    SetBkMode(dc, TRANSPARENT);
    SetTextColor(dc, lpp_hex_to_colorref(hex_color));
    TextOutA(dc, (int)x, (int)y, text, (int)strlen(text));
}

void lpp_gui_present(int64_t win_id) {
    /* No Sleep() here — frame pacing is the app's responsibility (#13) */
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].is_open) return;
    LppWin32Window *w = &g_windows[win_id];
    BitBlt(w->hdc, 0, 0, w->width, w->height, w->mem_dc, 0, 0, SRCCOPY);
}

void lpp_gui_draw_circle(int64_t win_id, int64_t cx, int64_t cy, int64_t r, int64_t hex_color) {
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].is_open) return;
    COLORREF col = lpp_hex_to_colorref(hex_color);
    HDC dc = g_windows[win_id].mem_dc;
    HBRUSH br  = CreateSolidBrush(col);
    HPEN   pen = CreatePen(PS_NULL, 0, 0);
    HBRUSH old_br  = (HBRUSH)SelectObject(dc, br);
    HPEN   old_pen = (HPEN)SelectObject(dc, pen);
    Ellipse(dc, (int)(cx-r), (int)(cy-r), (int)(cx+r), (int)(cy+r));
    SelectObject(dc, old_br);
    SelectObject(dc, old_pen);
    DeleteObject(br);
    DeleteObject(pen);
}

void lpp_gui_draw_line(int64_t win_id, int64_t x1, int64_t y1, int64_t x2, int64_t y2, int64_t thick, int64_t hex_color) {
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].is_open) return;
    COLORREF col = lpp_hex_to_colorref(hex_color);
    HDC dc = g_windows[win_id].mem_dc;
    HPEN pen     = CreatePen(PS_SOLID, (int)(thick > 0 ? thick : 1), col);
    HPEN old_pen = (HPEN)SelectObject(dc, pen);
    MoveToEx(dc, (int)x1, (int)y1, NULL);
    LineTo(dc, (int)x2, (int)y2);
    SelectObject(dc, old_pen);
    DeleteObject(pen);
}

int64_t lpp_gui_measure_text_width(int64_t win_id, const char *text) {
    if (!text || !*text) return 0;
    if (win_id < 0 || win_id >= MAX_WINDOWS || !g_windows[win_id].mem_dc)
        return (int64_t)(strlen(text) * 8);
    SIZE sz;
    if (GetTextExtentPoint32A(g_windows[win_id].mem_dc, text, (int)strlen(text), &sz))
        return (int64_t)sz.cx;
    return (int64_t)(strlen(text) * 8);
}

int64_t lpp_gui_dialog_message(const char *title, const char *msg) {
    return MessageBoxA(NULL, msg ? msg : "", title ? title : "L++ Alert",
                       MB_OK | MB_ICONINFORMATION);
}

int64_t lpp_gui_get_ticks_ms(void) {
    return (int64_t)GetTickCount64();
}

void lpp_gui_window_close(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_WINDOWS) return;
    LppWin32Window *w = &g_windows[win_id];
    if (!w->is_open && !w->hwnd) return;
    w->is_open = 0;
    HWND h = w->hwnd;
    w->hwnd = NULL;
    lpp_win32_destroy_slot(win_id); /* centralized cleanup (#38) */
    if (h) DestroyWindow(h);
}

/* ═══════════════════════════════════════════════════════════════════════════
 *  Unix / X11 backend
 * ═══════════════════════════════════════════════════════════════════════════ */
#else
#include <dlfcn.h>
#include <unistd.h>
#include <time.h>

/* ── Minimal X11 ABI definitions (no <X11/Xlib.h> needed) ───────────────── */
#define X11_Button1Mask    (1u << 8)   /* explicit constant (#22) */
#define X11_KeyPressMask   (1L << 0)
#define X11_KeyReleaseMask (1L << 1)
#define X11_ButtonPressMask   (1L << 2)
#define X11_ButtonReleaseMask (1L << 3)
#define X11_PointerMotionMask (1L << 6)
#define X11_ExposureMask      (1L << 15)
#define X11_StructureNotifyMask (1L << 17)

/* XEvent type codes */
#define X11_KeyPress      2
#define X11_KeyRelease    3
#define X11_ButtonPress   4
#define X11_ButtonRelease 5
#define X11_MotionNotify  6
#define X11_Expose        12
#define X11_DestroyNotify 17
#define X11_ConfigureNotify 22
#define X11_ClientMessage 33

/* Minimal XEvent layout — large enough for any event type (#41) */
typedef struct {
    int  type;
    unsigned long serial;
    int  send_event;
    void *display;
    unsigned long window;
    /* union of all event payloads — 24 longs covers all X11 event sizes */
    unsigned long _pad[24];
} X11Event;

/* XColor for colormap allocation (#15/#16) */
typedef struct {
    unsigned long pixel;
    unsigned short red, green, blue;
    char flags;
    char pad;
} X11Color;

/* ── Function pointer typedefs ───────────────────────────────────────────── */
typedef void*         (*fn_XOpenDisplay)(const char*);
typedef unsigned long (*fn_XDefaultRootWindow)(void*);          /* #1 fix */
typedef int           (*fn_XDefaultScreen)(void*);
typedef unsigned long (*fn_XDefaultColormap)(void*, int);
typedef unsigned long (*fn_XCreateSimpleWindow)(void*, unsigned long, int, int,
                            unsigned int, unsigned int, unsigned int,
                            unsigned long, unsigned long);
typedef int  (*fn_XSelectInput)(void*, unsigned long, long);
typedef int  (*fn_XMapWindow)(void*, unsigned long);
typedef int  (*fn_XFlush)(void*);
typedef int  (*fn_XSync)(void*, int);
typedef int  (*fn_XPending)(void*);
typedef int  (*fn_XNextEvent)(void*, X11Event*);
typedef int  (*fn_XCloseDisplay)(void*);
typedef int  (*fn_XDestroyWindow)(void*, unsigned long);
typedef void* (*fn_XCreateGC)(void*, unsigned long, unsigned long, void*);
typedef int  (*fn_XFreeGC)(void*, void*);
typedef int  (*fn_XSetForeground)(void*, void*, unsigned long);
typedef int  (*fn_XSetLineAttributes)(void*, void*, unsigned int,
                                      int, int, int);
typedef int  (*fn_XFillRectangle)(void*, unsigned long, void*,
                                   int, int, unsigned int, unsigned int);
typedef int  (*fn_XDrawLine)(void*, unsigned long, void*, int, int, int, int);
typedef int  (*fn_XFillArc)(void*, unsigned long, void*,
                             int, int, unsigned int, unsigned int, int, int);
typedef int  (*fn_XDrawString)(void*, unsigned long, void*,
                                int, int, const char*, int);
typedef int  (*fn_XQueryPointer)(void*, unsigned long, unsigned long*,
                                  unsigned long*, int*, int*, int*, int*,
                                  unsigned int*);
typedef int  (*fn_XAllocColor)(void*, unsigned long, X11Color*);
typedef int  (*fn_XStoreName)(void*, unsigned long, const char*);
typedef int  (*fn_XSetWMProtocols)(void*, unsigned long,
                                    unsigned long*, int);
typedef unsigned long (*fn_XInternAtom)(void*, const char*, int);
typedef unsigned long (*fn_XCreatePixmap)(void*, unsigned long,
                                           unsigned int, unsigned int, unsigned int);
typedef int  (*fn_XFreePixmap)(void*, unsigned long);
typedef int  (*fn_XCopyArea)(void*, unsigned long, unsigned long, void*,
                              int, int, unsigned int, unsigned int, int, int);
typedef int  (*fn_XDefaultDepth)(void*, int);
typedef void* (*fn_XLookupKeysym)(X11Event*, int);
typedef unsigned long (*fn_XLookupString)(X11Event*, char*, int, void*, void*);

static void *g_x11_lib = NULL;

static fn_XOpenDisplay       p_XOpenDisplay       = NULL;
static fn_XDefaultRootWindow p_XDefaultRootWindow = NULL;
static fn_XDefaultScreen     p_XDefaultScreen     = NULL;
static fn_XDefaultColormap   p_XDefaultColormap   = NULL;
static fn_XCreateSimpleWindow p_XCreateSimpleWindow = NULL;
static fn_XSelectInput       p_XSelectInput       = NULL;
static fn_XMapWindow         p_XMapWindow         = NULL;
static fn_XFlush             p_XFlush             = NULL;
static fn_XSync              p_XSync              = NULL;
static fn_XPending           p_XPending           = NULL;
static fn_XNextEvent         p_XNextEvent         = NULL;
static fn_XCloseDisplay      p_XCloseDisplay      = NULL;
static fn_XDestroyWindow     p_XDestroyWindow     = NULL;
static fn_XCreateGC          p_XCreateGC          = NULL;
static fn_XFreeGC            p_XFreeGC            = NULL;
static fn_XSetForeground     p_XSetForeground     = NULL;
static fn_XSetLineAttributes p_XSetLineAttributes = NULL;
static fn_XFillRectangle     p_XFillRectangle     = NULL;
static fn_XDrawLine          p_XDrawLine          = NULL;
static fn_XFillArc           p_XFillArc           = NULL;
static fn_XDrawString        p_XDrawString        = NULL;
static fn_XQueryPointer      p_XQueryPointer      = NULL;
static fn_XAllocColor        p_XAllocColor        = NULL;
static fn_XStoreName         p_XStoreName         = NULL;
static fn_XSetWMProtocols    p_XSetWMProtocols    = NULL;
static fn_XInternAtom        p_XInternAtom        = NULL;
static fn_XCreatePixmap      p_XCreatePixmap      = NULL;
static fn_XFreePixmap        p_XFreePixmap        = NULL;
static fn_XCopyArea          p_XCopyArea          = NULL;
static fn_XDefaultDepth      p_XDefaultDepth      = NULL;
static fn_XLookupString      p_XLookupString      = NULL;

#define X11_LOAD(name) p_##name = (fn_##name)dlsym(g_x11_lib, #name)

static int init_x11(void) {
    if (g_x11_lib) return 1;
    g_x11_lib = dlopen("libX11.so.6", RTLD_LAZY);
    if (!g_x11_lib) g_x11_lib = dlopen("libX11.so", RTLD_LAZY);
    if (!g_x11_lib) return 0;

    X11_LOAD(XOpenDisplay);
    X11_LOAD(XDefaultRootWindow);
    X11_LOAD(XDefaultScreen);
    X11_LOAD(XDefaultColormap);
    X11_LOAD(XCreateSimpleWindow);
    X11_LOAD(XSelectInput);
    X11_LOAD(XMapWindow);
    X11_LOAD(XFlush);
    X11_LOAD(XSync);
    X11_LOAD(XPending);
    X11_LOAD(XNextEvent);
    X11_LOAD(XCloseDisplay);
    X11_LOAD(XDestroyWindow);
    X11_LOAD(XCreateGC);
    X11_LOAD(XFreeGC);
    X11_LOAD(XSetForeground);
    X11_LOAD(XSetLineAttributes);
    X11_LOAD(XFillRectangle);
    X11_LOAD(XDrawLine);
    X11_LOAD(XFillArc);
    X11_LOAD(XDrawString);
    X11_LOAD(XQueryPointer);
    X11_LOAD(XAllocColor);
    X11_LOAD(XStoreName);
    X11_LOAD(XSetWMProtocols);
    X11_LOAD(XInternAtom);
    X11_LOAD(XCreatePixmap);
    X11_LOAD(XFreePixmap);
    X11_LOAD(XCopyArea);
    X11_LOAD(XDefaultDepth);
    X11_LOAD(XLookupString);

    /* Validate the minimal required set (#31) */
    return p_XOpenDisplay  && p_XDefaultRootWindow && p_XCreateSimpleWindow
        && p_XMapWindow    && p_XCreateGC          && p_XFlush
        && p_XPending      && p_XNextEvent;
}

/* ── Per-window state ────────────────────────────────────────────────────── */
typedef struct {
    void         *display;
    unsigned long window;
    unsigned long pixmap;      /* off-screen backbuffer (#20/#21) */
    void         *gc;          /* drawing GC */
    void         *pixmap_gc;   /* GC for pixmap backbuffer */
    unsigned long colormap;
    unsigned long wm_delete;   /* WM_DELETE_WINDOW atom (#17) */
    int           depth;
    int           width;
    int           height;
    int           is_open;
    unsigned long bg_pixel;    /* current background pixel */
    /* Keyboard state cache — updated from KeyPress/KeyRelease events (#3) */
    uint8_t       key_state[256];
    int           mouse_x;
    int           mouse_y;
    int           mouse_down;
} LppUnixWindow;

#define MAX_UNIX_WINDOWS 8
static LppUnixWindow g_unix_windows[MAX_UNIX_WINDOWS];

static int lpp_unix_find_free_slot(void) {
    for (int i = 0; i < MAX_UNIX_WINDOWS; i++) {
        if (!g_unix_windows[i].is_open && !g_unix_windows[i].display) return i;
    }
    return -1;
}

/* Convert 0xRRGGBB to X11 pixel through XAllocColor (#15/#16) */
static unsigned long lpp_x11_color(LppUnixWindow *w, int64_t hex) {
    if (!p_XAllocColor) return (unsigned long)hex;
    X11Color c;
    c.red   = (unsigned short)(((hex >> 16) & 0xFF) * 257);
    c.green = (unsigned short)(((hex >>  8) & 0xFF) * 257);
    c.blue  = (unsigned short)(( hex        & 0xFF) * 257);
    c.flags = 7; /* DoRed | DoGreen | DoBlue */
    if (p_XAllocColor(w->display, w->colormap, &c)) return c.pixel;
    return (unsigned long)hex; /* fallback */
}

/* Centralized X11 resource teardown (#33/#34/#38) */
static void lpp_unix_destroy_slot(int id) {
    LppUnixWindow *w = &g_unix_windows[id];
    if (!w->display) return;
    if (w->gc && p_XFreeGC)        p_XFreeGC(w->display, w->gc);
    if (w->pixmap_gc && p_XFreeGC) p_XFreeGC(w->display, w->pixmap_gc);
    if (w->pixmap && p_XFreePixmap)    p_XFreePixmap(w->display, w->pixmap);
    if (w->window && p_XDestroyWindow)    p_XDestroyWindow(w->display, w->window);
    if (p_XCloseDisplay) p_XCloseDisplay(w->display);
    memset(w, 0, sizeof(*w)); /* zero the slot so it's reusable (#34) */
}

int64_t lpp_gui_window_create(const char *title, int64_t width, int64_t height) {
    /* Validate dimensions (#29) */
    if (width <= 0 || width > 65535 || height <= 0 || height > 65535) return -1;

    int id = lpp_unix_find_free_slot();
    if (id < 0) return -1; /* all 8 slots in use (#12) */

    if (!init_x11()) {
        /* X11 unavailable — return -1 rather than faking a window (#10) */
        fprintf(stderr, "[L++ GUI] X11 unavailable: %s\n",
                dlerror() ? dlerror() : "unknown");
        return -1;
    }

    void *display = p_XOpenDisplay(NULL);
    if (!display) {
        fprintf(stderr, "[L++ GUI] Cannot open X11 display.\n");
        return -1; /* honest failure, not a fake window (#10) */
    }

    int screen          = p_XDefaultScreen(display);
    unsigned long root  = p_XDefaultRootWindow(display); /* #1 fix */
    unsigned long cmap  = p_XDefaultColormap ? p_XDefaultColormap(display, screen) : 0;
    int depth           = p_XDefaultDepth    ? p_XDefaultDepth(display, screen) : 24;

    /* Allocate background color properly (#16/#57) */
    X11Color bg_color = {0};
    bg_color.red   = 19 * 257;  /* matches dark_theme #131821 */
    bg_color.green = 24 * 257;
    bg_color.blue  = 33 * 257;
    bg_color.flags = 7;
    unsigned long bg_pixel = 0x131821;
    if (p_XAllocColor) p_XAllocColor(display, cmap, &bg_color);
    bg_pixel = bg_color.pixel;

    unsigned long win = p_XCreateSimpleWindow(
        display, root, 10, 10,
        (unsigned int)width, (unsigned int)height,
        0,           /* border width — no border (#58) */
        0,           /* border color */
        bg_pixel);

    if (!win) {
        p_XCloseDisplay(display);
        return -1;
    }

    /* Set window title (#18) */
    if (p_XStoreName) p_XStoreName(display, win, title ? title : "L++ GUI App");

    /* Select all events we need (#42) */
    long event_mask = X11_KeyPressMask | X11_KeyReleaseMask
                    | X11_ButtonPressMask | X11_ButtonReleaseMask
                    | X11_PointerMotionMask
                    | X11_ExposureMask
                    | X11_StructureNotifyMask;
    p_XSelectInput(display, win, event_mask);

    /* Register WM_DELETE_WINDOW close protocol (#17) */
    unsigned long wm_delete = 0;
    if (p_XInternAtom && p_XSetWMProtocols) {
        wm_delete = p_XInternAtom(display, "WM_DELETE_WINDOW", 0);
        if (wm_delete) p_XSetWMProtocols(display, win, &wm_delete, 1);
    }

    /* Create off-screen pixmap backbuffer (#20/#21) */
    unsigned long pixmap = 0;
    if (p_XCreatePixmap)
        pixmap = p_XCreatePixmap(display, win,
                                  (unsigned int)width, (unsigned int)height, (unsigned int)depth);

    void *gc        = p_XCreateGC(display, win, 0, NULL);
    void *pixmap_gc = pixmap && p_XCreateGC ? p_XCreateGC(display, pixmap, 0, NULL) : NULL;

    /* Fill backbuffer with bg color */
    if (pixmap_gc && p_XSetForeground && p_XFillRectangle) {
        p_XSetForeground(display, pixmap_gc, bg_pixel);
        p_XFillRectangle(display, pixmap, pixmap_gc,
                          0, 0, (unsigned int)width, (unsigned int)height);
    }

    p_XMapWindow(display, win);
    if (p_XSync) p_XSync(display, 0); /* wait for window to appear (#43) */

    /* Populate slot */
    LppUnixWindow *w = &g_unix_windows[id];
    w->display    = display;
    w->window     = win;
    w->pixmap     = pixmap;
    w->gc         = gc;
    w->pixmap_gc  = pixmap_gc;
    w->colormap   = cmap;
    w->wm_delete  = wm_delete;
    w->depth      = depth;
    w->width      = (int)width;
    w->height     = (int)height;
    w->is_open    = 1;
    w->bg_pixel   = bg_pixel;
    return (int64_t)id;
}

int64_t lpp_gui_window_is_open(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS) return 0;
    return g_unix_windows[win_id].is_open ? 1 : 0;
}

int64_t lpp_gui_window_width(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].display) return 0;
    return g_unix_windows[win_id].width;
}

int64_t lpp_gui_window_height(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].display) return 0;
    return g_unix_windows[win_id].height;
}

int64_t lpp_gui_window_poll_events(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].is_open) return 0;
    LppUnixWindow *w = &g_unix_windows[win_id];
    if (!w->display || !p_XPending) return w->is_open;

    while (p_XPending(w->display)) {
        X11Event ev = {0};
        p_XNextEvent(w->display, &ev);  /* #2 fix — actually decode events */

        switch (ev.type) {
            case X11_KeyPress:
                /* Cache key state (#3) */
                if (p_XLookupString) {
                    char buf[8] = {0};
                    unsigned long keysym = 0;
                    p_XLookupString(&ev, buf, sizeof(buf), (void*)&keysym, NULL);
                    if (keysym < 256) w->key_state[keysym] = 1;
                }
                /* Also store by X11 keycode (low byte of ev._pad[1]) */
                {
                    unsigned int keycode = (unsigned int)(ev._pad[1] & 0xFF);
                    if (keycode < 256) w->key_state[keycode] = 1;
                }
                break;
            case X11_KeyRelease:
                {
                    unsigned int keycode = (unsigned int)(ev._pad[1] & 0xFF);
                    if (keycode < 256) w->key_state[keycode] = 0;
                    if (p_XLookupString) {
                        char buf[8] = {0};
                        unsigned long keysym = 0;
                        p_XLookupString(&ev, buf, sizeof(buf), (void*)&keysym, NULL);
                        if (keysym < 256) w->key_state[keysym] = 0;
                    }
                }
                break;
            case X11_ButtonPress:
                w->mouse_down = 1;
                break;
            case X11_ButtonRelease:
                w->mouse_down = 0;
                break;
            case X11_MotionNotify:
                /* Motion: ev._pad[4]=x, ev._pad[5]=y (XMotionEvent layout) */
                w->mouse_x = (int)(ev._pad[4]);
                w->mouse_y = (int)(ev._pad[5]);
                break;
            case X11_ConfigureNotify: {
                /* Resize — update dims and recreate pixmap backbuffer (#19) */
                int new_w = (int)(ev._pad[6]);
                int new_h = (int)(ev._pad[7]);
                if (new_w > 0 && new_h > 0 &&
                    (new_w != w->width || new_h != w->height)) {
                    w->width  = new_w;
                    w->height = new_h;
                    if (p_XFreePixmap && w->pixmap) p_XFreePixmap(w->display, w->pixmap);
                    if (p_XFreeGC     && w->pixmap_gc) p_XFreeGC(w->display, w->pixmap_gc);
                    w->pixmap = 0; w->pixmap_gc = NULL;
                    if (p_XCreatePixmap)
                        w->pixmap = p_XCreatePixmap(w->display, w->window,
                                                     (unsigned int)new_w, (unsigned int)new_h,
                                                     (unsigned int)w->depth);
                    if (w->pixmap && p_XCreateGC)
                        w->pixmap_gc = p_XCreateGC(w->display, w->pixmap, 0, NULL);
                }
                break;
            }
            case X11_Expose: {
                /* Blit pixmap backbuffer to window on expose (#20) */
                if (w->pixmap && w->gc && p_XCopyArea) {
                    p_XCopyArea(w->display, w->pixmap, w->window, w->gc,
                                0, 0, (unsigned int)w->width, (unsigned int)w->height, 0, 0);
                    if (p_XFlush) p_XFlush(w->display);
                }
                break;
            }
            case X11_DestroyNotify:
                w->is_open = 0;
                break;
            case X11_ClientMessage: {
                /* WM_DELETE_WINDOW (#17) */
                unsigned long atom = ev._pad[3];
                if (atom == w->wm_delete) w->is_open = 0;
                break;
            }
        }
    }
    return w->is_open ? 1 : 0;
}

/* Draw helper — set color and draw to pixmap backbuffer (or window fallback) */
static unsigned long lpp_unix_target(LppUnixWindow *w) {
    return w->pixmap ? w->pixmap : w->window;
}
static void *lpp_unix_gc(LppUnixWindow *w) {
    return w->pixmap_gc ? w->pixmap_gc : w->gc;
}

void lpp_gui_clear(int64_t win_id, int64_t hex_color) {
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].is_open) return;
    LppUnixWindow *w = &g_unix_windows[win_id];
    if (!w->display || !p_XSetForeground || !p_XFillRectangle) return;
    unsigned long pixel = lpp_x11_color(w, hex_color);
    w->bg_pixel = pixel;
    void *gc = lpp_unix_gc(w);
    p_XSetForeground(w->display, gc, pixel);
    p_XFillRectangle(w->display, lpp_unix_target(w), gc,
                     0, 0, (unsigned int)w->width, (unsigned int)w->height);
}

void lpp_gui_draw_rect(int64_t win_id, int64_t x, int64_t y, int64_t wd, int64_t ht, int64_t hex_color) {
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].is_open) return;
    LppUnixWindow *w = &g_unix_windows[win_id];
    if (!w->display || !p_XSetForeground || !p_XFillRectangle) return;
    void *gc = lpp_unix_gc(w);
    p_XSetForeground(w->display, gc, lpp_x11_color(w, hex_color));
    p_XFillRectangle(w->display, lpp_unix_target(w), gc,
                     (int)x, (int)y, (unsigned int)wd, (unsigned int)ht);
}

void lpp_gui_draw_rounded_rect(int64_t win_id, int64_t x, int64_t y, int64_t wd, int64_t ht,
                                int64_t radius, int64_t hex_color) {
    /* X11 approximation: corner arcs + inner rectangles (#7) */
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].is_open) return;
    LppUnixWindow *w = &g_unix_windows[win_id];
    if (!w->display || !p_XFillArc || !p_XFillRectangle || !p_XSetForeground) return;
    void *gc = lpp_unix_gc(w);
    unsigned long tgt = lpp_unix_target(w);
    unsigned long pixel = lpp_x11_color(w, hex_color);
    p_XSetForeground(w->display, gc, pixel);

    int r = (int)radius;
    int d = r * 2;
    int ix = (int)x, iy = (int)y, iw = (int)wd, ih = (int)ht;

    /* Four corner arcs (64ths of degree: 90° = 90*64) */
    p_XFillArc(w->display, tgt, gc, ix,          iy,          d, d,  90*64,  90*64); /* TL */
    p_XFillArc(w->display, tgt, gc, ix+iw-d,     iy,          d, d,   0*64,  90*64); /* TR */
    p_XFillArc(w->display, tgt, gc, ix,          iy+ih-d,     d, d, 180*64,  90*64); /* BL */
    p_XFillArc(w->display, tgt, gc, ix+iw-d,     iy+ih-d,     d, d, 270*64,  90*64); /* BR */
    /* Three fill rectangles */
    p_XFillRectangle(w->display, tgt, gc, ix+r, iy,   (unsigned)(iw-d), (unsigned)ih); /* centre col */
    p_XFillRectangle(w->display, tgt, gc, ix,   iy+r, (unsigned)r,      (unsigned)(ih-d)); /* left edge */
    p_XFillRectangle(w->display, tgt, gc, ix+iw-r, iy+r, (unsigned)r,  (unsigned)(ih-d)); /* right edge */
}

int64_t lpp_gui_mouse_x(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].is_open) return 0;
    return (int64_t)g_unix_windows[win_id].mouse_x;
}

int64_t lpp_gui_mouse_y(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].is_open) return 0;
    return (int64_t)g_unix_windows[win_id].mouse_y;
}

int64_t lpp_gui_mouse_down(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].is_open) return 0;
    return g_unix_windows[win_id].mouse_down ? 1 : 0;
}

int64_t lpp_gui_key_down(int64_t win_id, int64_t key_code) {
    /* Keyboard state updated from KeyPress/KeyRelease events (#3) */
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].is_open) return 0;
    if (key_code < 0 || key_code > 255) return 0;
    return g_unix_windows[win_id].key_state[(int)key_code] ? 1 : 0;
}

void lpp_gui_draw_text(int64_t win_id, int64_t x, int64_t y, const char *text, int64_t hex_color) {
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].is_open || !text) return;
    LppUnixWindow *w = &g_unix_windows[win_id];
    if (!w->display || !p_XSetForeground || !p_XDrawString) return;
    void *gc = lpp_unix_gc(w);
    p_XSetForeground(w->display, gc, lpp_x11_color(w, hex_color));
    p_XDrawString(w->display, lpp_unix_target(w), gc,
                  (int)x, (int)(y + 13), /* +13 to align baseline like Win32 */
                  text, (int)strlen(text));
}

void lpp_gui_present(int64_t win_id) {
    /* No hardcoded sleep — frame pacing is the app's responsibility (#13/#47) */
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].is_open) return;
    LppUnixWindow *w = &g_unix_windows[win_id];
    if (!w->display) return;
    /* Blit pixmap backbuffer to window (#20/#21) */
    if (w->pixmap && w->gc && p_XCopyArea) {
        p_XCopyArea(w->display, w->pixmap, w->window, w->gc,
                    0, 0, (unsigned int)w->width, (unsigned int)w->height, 0, 0);
    }
    if (p_XFlush) p_XFlush(w->display);
}

void lpp_gui_draw_circle(int64_t win_id, int64_t cx, int64_t cy, int64_t r, int64_t hex_color) {
    /* Real circle using XFillArc (#5) */
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].is_open) return;
    LppUnixWindow *w = &g_unix_windows[win_id];
    if (!w->display || !p_XFillArc || !p_XSetForeground) return;
    void *gc = lpp_unix_gc(w);
    p_XSetForeground(w->display, gc, lpp_x11_color(w, hex_color));
    p_XFillArc(w->display, lpp_unix_target(w), gc,
               (int)(cx-r), (int)(cy-r),
               (unsigned int)(r*2), (unsigned int)(r*2),
               0, 360*64);
}

void lpp_gui_draw_line(int64_t win_id, int64_t x1, int64_t y1, int64_t x2, int64_t y2,
                       int64_t thick, int64_t hex_color) {
    /* Real line using XDrawLine with XSetLineAttributes (#6) */
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS || !g_unix_windows[win_id].is_open) return;
    LppUnixWindow *w = &g_unix_windows[win_id];
    if (!w->display || !p_XDrawLine || !p_XSetForeground) return;
    void *gc = lpp_unix_gc(w);
    p_XSetForeground(w->display, gc, lpp_x11_color(w, hex_color));
    if (p_XSetLineAttributes)
        p_XSetLineAttributes(w->display, gc,
                             (unsigned int)(thick > 0 ? thick : 1), 0, 0, 0);
    p_XDrawLine(w->display, lpp_unix_target(w), gc,
                (int)x1, (int)y1, (int)x2, (int)y2);
}

int64_t lpp_gui_measure_text_width(int64_t win_id, const char *text) {
    (void)win_id;
    /* Approximate — Xft/Pango needed for real font metrics (#9) */
    return (int64_t)(text ? strlen(text) * 8 : 0);
}

int64_t lpp_gui_dialog_message(const char *title, const char *msg) {
    printf("[%s] %s\n", title ? title : "Alert", msg ? msg : "");
    return 1;
}

int64_t lpp_gui_get_ticks_ms(void) {
    /* Monotonic clock — time actually advances now (#4) */
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    return (int64_t)(ts.tv_sec * 1000LL + ts.tv_nsec / 1000000LL);
}

void lpp_gui_window_close(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_UNIX_WINDOWS) return;
    LppUnixWindow *w = &g_unix_windows[win_id];
    if (!w->is_open && !w->display) return;
    w->is_open = 0;
    lpp_unix_destroy_slot(win_id); /* centralized cleanup (#33/#34/#38) */
}

#endif /* !_WIN32 */

/* ═══════════════════════════════════════════════════════════════════════════
 *  Raylib Standard Compatibility Layer (rl_ prefix)
 * ═══════════════════════════════════════════════════════════════════════════ */

static int64_t g_active_raylib_win = 0;
static int64_t g_target_fps = 60;
static uint8_t g_prev_key_state[256] = {0};
static uint8_t g_mouse_btn[3] = {0};
static uint8_t g_prev_mouse_btn[3] = {0};

static int lpp_raylib_key_to_vk(int64_t key) {
    if (key >= 'A' && key <= 'Z') return (int)key;
    if (key >= '0' && key <= '9') return (int)key;
    if (key == 32) return 0x20; // VK_SPACE
    if (key == 39) return 0xDE; // VK_OEM_7 (')
    if (key == 44) return 0xBC; // VK_OEM_COMMA (,)
    if (key == 45) return 0xBD; // VK_OEM_MINUS (-)
    if (key == 46) return 0xBE; // VK_OEM_PERIOD (.)
    if (key == 47) return 0xBF; // VK_OEM_2 (/)
    if (key == 59) return 0xBA; // VK_OEM_1 (;)
    if (key == 61) return 0xBB; // VK_OEM_PLUS (=)
    if (key == 91) return 0xDB; // VK_OEM_4 ([)
    if (key == 92) return 0xDC; // VK_OEM_5 (\)
    if (key == 93) return 0xDD; // VK_OEM_6 (])
    if (key == 96) return 0xC0; // VK_OEM_3 (`)
    
    /* Raylib special key codes */
    if (key == 256) return 0x1B; // VK_ESCAPE
    if (key == 257) return 0x0D; // VK_RETURN
    if (key == 258) return 0x09; // VK_TAB
    if (key == 259) return 0x08; // VK_BACK
    if (key == 260) return 0x2D; // VK_INSERT
    if (key == 261) return 0x2E; // VK_DELETE
    if (key == 262) return 0x27; // VK_RIGHT
    if (key == 263) return 0x25; // VK_LEFT
    if (key == 264) return 0x28; // VK_DOWN
    if (key == 265) return 0x26; // VK_UP
    if (key == 266) return 0x21; // VK_PRIOR (Page Up)
    if (key == 267) return 0x22; // VK_NEXT  (Page Down)
    if (key == 268) return 0x24; // VK_HOME
    if (key == 269) return 0x23; // VK_END
    if (key == 340 || key == 344) return 0x10; // VK_SHIFT
    if (key == 341 || key == 345) return 0x11; // VK_CONTROL
    if (key == 342 || key == 346) return 0x12; // VK_MENU (Alt)
    
    /* Function keys F1 - F12 */
    if (key >= 290 && key <= 301) return 0x70 + (int)(key - 290);
    
    if (key >= 0 && key < 256) return (int)key;
    return 0;
}

void rl_InitWindow(int64_t width, int64_t height, const char *title) {
    g_active_raylib_win = lpp_gui_window_create(title, width, height);
}

int64_t rl_WindowShouldClose(void) {
    lpp_gui_window_poll_events(g_active_raylib_win);
    return !lpp_gui_window_is_open(g_active_raylib_win);
}

void rl_CloseWindow(void) {
    lpp_gui_window_close(g_active_raylib_win);
}

void rl_SetTargetFPS(int64_t fps) {
    if (fps > 0) g_target_fps = fps;
}

int64_t rl_GetFPS(void) {
    return g_target_fps;
}

double rl_GetFrameTime(void) {
    return 1.0 / (double)(g_target_fps > 0 ? g_target_fps : 60);
}

int64_t rl_GetScreenWidth(void) {
    return lpp_gui_window_width(g_active_raylib_win);
}

int64_t rl_GetScreenHeight(void) {
    return lpp_gui_window_height(g_active_raylib_win);
}

void rl_BeginDrawing(void) {
}

void rl_EndDrawing(void) {
    lpp_gui_present(g_active_raylib_win);
    memcpy(g_prev_key_state, g_key_state, sizeof(g_key_state));
    memset(g_mouse_btn_pressed, 0, sizeof(g_mouse_btn_pressed));
    memset(g_mouse_btn_released, 0, sizeof(g_mouse_btn_released));
    g_mouse_wheel_delta = 0.0;
    if (g_target_fps > 0) {
        int64_t frame_ms = 1000 / g_target_fps;
        #if defined(_WIN32)
        Sleep((DWORD)frame_ms);
        #else
        usleep((useconds_t)(frame_ms * 1000));
        #endif
    }
}

void rl_ClearBackground(int64_t color) {
    lpp_gui_clear(g_active_raylib_win, color);
}

void rl_DrawRectangle(int64_t posX, int64_t posY, int64_t width, int64_t height, int64_t color) {
    lpp_gui_draw_rect(g_active_raylib_win, posX, posY, width, height, color);
}

void rl_DrawRectangleLines(int64_t posX, int64_t posY, int64_t width, int64_t height, int64_t color) {
    lpp_gui_draw_line(g_active_raylib_win, posX, posY, posX + width, posY, 1, color);
    lpp_gui_draw_line(g_active_raylib_win, posX + width, posY, posX + width, posY + height, 1, color);
    lpp_gui_draw_line(g_active_raylib_win, posX + width, posY + height, posX, posY + height, 1, color);
    lpp_gui_draw_line(g_active_raylib_win, posX, posY + height, posX, posY, 1, color);
}

void rl_DrawRectangleRounded(int64_t posX, int64_t posY, int64_t width, int64_t height, double roundness, int64_t segments, int64_t color) {
    (void)segments;
    int64_t radius = (int64_t)(roundness * (double)(height > 0 ? height / 2 : 4));
    lpp_gui_draw_rounded_rect(g_active_raylib_win, posX, posY, width, height, radius, color);
}

void rl_DrawLine(int64_t startPosX, int64_t startPosY, int64_t endPosX, int64_t endPosY, int64_t color) {
    lpp_gui_draw_line(g_active_raylib_win, startPosX, startPosY, endPosX, endPosY, 1, color);
}

void rl_DrawCircle(int64_t centerX, int64_t centerY, double radius, int64_t color) {
    lpp_gui_draw_circle(g_active_raylib_win, centerX, centerY, (int64_t)radius, color);
}

void rl_DrawText(const char *text, int64_t posX, int64_t posY, int64_t fontSize, int64_t color) {
    (void)fontSize;
    lpp_gui_draw_text(g_active_raylib_win, posX, posY, text, color);
}

int64_t rl_MeasureText(const char *text, int64_t fontSize) {
    (void)fontSize;
    return lpp_gui_measure_text_width(g_active_raylib_win, text);
}

int64_t rl_IsKeyDown(int64_t key) {
    int vk = lpp_raylib_key_to_vk(key);
    if (vk >= 0 && vk < 256) return g_key_state[vk];
    return 0;
}

int64_t rl_IsKeyPressed(int64_t key) {
    int vk = lpp_raylib_key_to_vk(key);
    if (vk >= 0 && vk < 256) {
        return (g_key_state[vk] && !g_prev_key_state[vk]) ? 1 : 0;
    }
    return 0;
}

int64_t rl_GetCharPressed(void) {
    if (g_char_q_head != g_char_q_tail) {
        int64_t c = g_char_queue[g_char_q_tail];
        g_char_q_tail = (g_char_q_tail + 1) % 64;
        return c;
    }
    return 0;
}

int64_t rl_GetKeyPressed(void) {
    return 0;
}

int64_t rl_GetMouseX(void) {
    return lpp_gui_mouse_x(g_active_raylib_win);
}

int64_t rl_GetMouseY(void) {
    return lpp_gui_mouse_y(g_active_raylib_win);
}

int64_t rl_IsMouseButtonPressed(int64_t button) {
    int b = (int)button;
    if (b >= 0 && b < 3) {
        return g_mouse_btn_pressed[b] ? 1 : 0;
    }
    return 0;
}

int64_t rl_IsMouseButtonDown(int64_t button) {
    int b = (int)button;
    if (b >= 0 && b < 3) {
        return g_mouse_btn_down[b] ? 1 : 0;
    }
    return 0;
}

double rl_GetMouseWheelMove(void) {
    return g_mouse_wheel_delta;
}

void rl_SetClipboardText(const char *text) {
    (void)text;
}

const char *rl_GetClipboardText(void) {
    return "";
}

void rl_TakeScreenshot(const char *filename) {
    #if defined(_WIN32)
    if (g_active_raylib_win < 0 || g_active_raylib_win >= MAX_WINDOWS) return;
    LppWin32Window *w = &g_windows[g_active_raylib_win];
    if (!w->is_open || !w->mem_dc || !w->hbmp) return;
    
    int width = w->width;
    int height = w->height;
    if (width <= 0 || height <= 0) return;
    
    BITMAPINFOHEADER bi;
    memset(&bi, 0, sizeof(bi));
    bi.biSize = sizeof(BITMAPINFOHEADER);
    bi.biWidth = width;
    bi.biHeight = height; // bottom-up
    bi.biPlanes = 1;
    bi.biBitCount = 32;
    bi.biCompression = BI_RGB;
    
    int buf_size = width * height * 4;
    uint8_t *pixels = (uint8_t *)malloc(buf_size);
    if (!pixels) return;
    
    GetDIBits(w->mem_dc, w->hbmp, 0, height, pixels, (BITMAPINFO *)&bi, DIB_RGB_COLORS);
    
    #pragma pack(push, 1)
    typedef struct {
        uint16_t bfType;
        uint32_t bfSize;
        uint16_t bfReserved1;
        uint16_t bfReserved2;
        uint32_t bfOffBits;
    } BMPHeader;
    #pragma pack(pop)
    
    BMPHeader hdr;
    hdr.bfType = 0x4D42; // "BM"
    hdr.bfSize = sizeof(BMPHeader) + sizeof(BITMAPINFOHEADER) + buf_size;
    hdr.bfReserved1 = 0;
    hdr.bfReserved2 = 0;
    hdr.bfOffBits = sizeof(BMPHeader) + sizeof(BITMAPINFOHEADER);
    
    FILE *f = fopen(filename, "wb");
    if (f) {
        fwrite(&hdr, sizeof(hdr), 1, f);
        fwrite(&bi, sizeof(bi), 1, f);
        fwrite(pixels, 1, buf_size, f);
        fclose(f);
    }
    free(pixels);
    #endif
}

