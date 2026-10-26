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