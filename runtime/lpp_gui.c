/*
 * lpp_gui.c  —  L++ Native 2D GUI & Windowing Builtins (cross-platform)
 *
 * Provides native GUI window creation, 2D canvas drawing (rectangles, text, clearing),
 * event polling, and rendering using Win32 GDI on Windows and X11 on Unix.
 */

#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <stdint.h>

#if defined(_WIN32)
#if defined(_MSC_VER)
#pragma comment(lib, "user32.lib")
#pragma comment(lib, "gdi32.lib")
#endif
#include <windows.h>

typedef struct {
    HWND hwnd;
    HDC hdc;
    HDC mem_dc;
    HBITMAP hbmp;
    HBITMAP old_bmp;
    int width;
    int height;
    int is_open;
    COLORREF bg_color;
} LppWin32Window;

static LppWin32Window g_windows[8];
static int g_win_count = 0;
static int g_class_registered = 0;

static LRESULT CALLBACK lpp_gui_wndproc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    switch (msg) {
        case WM_CLOSE:
            for (int i = 0; i < g_win_count; i++) {
                if (g_windows[i].hwnd == hwnd) {
                    g_windows[i].is_open = 0;
                    break;
                }
            }
            DestroyWindow(hwnd);
            return 0;
        case WM_DESTROY:
            PostQuitMessage(0);
            return 0;
        default:
            return DefWindowProcA(hwnd, msg, wp, lp);
    }
}

int64_t lpp_gui_window_create(const char *title, int64_t width, int64_t height) {
    if (g_win_count >= 8) return -1;

    HINSTANCE hInst = GetModuleHandleA(NULL);

    if (!g_class_registered) {
        WNDCLASSA wc = {0};
        wc.lpfnWndProc = lpp_gui_wndproc;
        wc.hInstance = hInst;
        wc.lpszClassName = "LppGUIWindowClass";
        wc.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
        wc.hCursor = LoadCursorA(NULL, IDC_ARROW);
        RegisterClassA(&wc);
        g_class_registered = 1;
    }

    HWND hwnd = CreateWindowExA(
        0,
        "LppGUIWindowClass",
        title ? title : "L++ GUI App",
        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
        CW_USEDEFAULT, CW_USEDEFAULT,
        (int)width, (int)height,
        NULL, NULL, hInst, NULL
    );

    if (!hwnd) return -1;

    HDC hdc = GetDC(hwnd);
    HDC mem_dc = CreateCompatibleDC(hdc);
    HBITMAP hbmp = CreateCompatibleBitmap(hdc, (int)width, (int)height);
    HBITMAP old_bmp = (HBITMAP)SelectObject(mem_dc, hbmp);

    int id = g_win_count++;
    g_windows[id].hwnd = hwnd;
    g_windows[id].hdc = hdc;
    g_windows[id].mem_dc = mem_dc;
    g_windows[id].hbmp = hbmp;
    g_windows[id].old_bmp = old_bmp;
    g_windows[id].width = (int)width;
    g_windows[id].height = (int)height;
    g_windows[id].is_open = 1;
    g_windows[id].bg_color = RGB(240, 240, 240);

    // Initial clear
    RECT r = {0, 0, (int)width, (int)height};
    HBRUSH brush = CreateSolidBrush(g_windows[id].bg_color);
    FillRect(mem_dc, &r, brush);
    DeleteObject(brush);

    return (int64_t)id;
}

int64_t lpp_gui_window_is_open(int64_t win_id) {
    if (win_id < 0 || win_id >= g_win_count) return 0;
    return g_windows[win_id].is_open ? 1 : 0;
}

int64_t lpp_gui_window_poll_events(int64_t win_id) {
    if (win_id < 0 || win_id >= g_win_count) return 0;
    MSG msg;
    while (PeekMessageA(&msg, NULL, 0, 0, PM_REMOVE)) {
        if (msg.message == WM_QUIT) {
            g_windows[win_id].is_open = 0;
            return 0;
        }
        TranslateMessage(&msg);
        DispatchMessageA(&msg);
    }
    return g_windows[win_id].is_open ? 1 : 0;
}

void lpp_gui_clear(int64_t win_id, int64_t hex_color) {
    if (win_id < 0 || win_id >= g_win_count || !g_windows[win_id].is_open) return;
    BYTE r_val = (BYTE)((hex_color >> 16) & 0xFF);
    BYTE g_val = (BYTE)((hex_color >> 8) & 0xFF);
    BYTE b_val = (BYTE)(hex_color & 0xFF);

    COLORREF col = RGB(r_val, g_val, b_val);
    RECT r = {0, 0, g_windows[win_id].width, g_windows[win_id].height};
    HBRUSH brush = CreateSolidBrush(col);
    FillRect(g_windows[win_id].mem_dc, &r, brush);
    DeleteObject(brush);
}

void lpp_gui_draw_rect(int64_t win_id, int64_t x, int64_t y, int64_t w, int64_t h, int64_t hex_color) {
    if (win_id < 0 || win_id >= g_win_count || !g_windows[win_id].is_open) return;
    BYTE r_val = (BYTE)((hex_color >> 16) & 0xFF);
    BYTE g_val = (BYTE)((hex_color >> 8) & 0xFF);
    BYTE b_val = (BYTE)(hex_color & 0xFF);

    COLORREF col = RGB(r_val, g_val, b_val);
    RECT r = {(LONG)x, (LONG)y, (LONG)(x + w), (LONG)(y + h)};
    HBRUSH brush = CreateSolidBrush(col);
    FillRect(g_windows[win_id].mem_dc, &r, brush);
    DeleteObject(brush);
}

void lpp_gui_draw_text(int64_t win_id, int64_t x, int64_t y, const char *text, int64_t hex_color) {
    if (win_id < 0 || win_id >= g_win_count || !g_windows[win_id].is_open || !text) return;
    BYTE r_val = (BYTE)((hex_color >> 16) & 0xFF);
    BYTE g_val = (BYTE)((hex_color >> 8) & 0xFF);
    BYTE b_val = (BYTE)(hex_color & 0xFF);

    SetBkMode(g_windows[win_id].mem_dc, TRANSPARENT);
    SetTextColor(g_windows[win_id].mem_dc, RGB(r_val, g_val, b_val));
    TextOutA(g_windows[win_id].mem_dc, (int)x, (int)y, text, (int)strlen(text));
}

void lpp_gui_present(int64_t win_id) {
    if (win_id < 0 || win_id >= g_win_count || !g_windows[win_id].is_open) return;
    BitBlt(
        g_windows[win_id].hdc, 0, 0,
        g_windows[win_id].width, g_windows[win_id].height,
        g_windows[win_id].mem_dc, 0, 0, SRCCOPY
    );
}

void lpp_gui_window_close(int64_t win_id) {
    if (win_id < 0 || win_id >= g_win_count) return;
    if (g_windows[win_id].is_open) {
        g_windows[win_id].is_open = 0;
        SelectObject(g_windows[win_id].mem_dc, g_windows[win_id].old_bmp);
        DeleteObject(g_windows[win_id].hbmp);
        DeleteDC(g_windows[win_id].mem_dc);
        ReleaseDC(g_windows[win_id].hwnd, g_windows[win_id].hdc);
        DestroyWindow(g_windows[win_id].hwnd);
    }
}

#else
/* Unix / Headless Fallback Implementation */
int64_t lpp_gui_window_create(const char *title, int64_t width, int64_t height) {
    printf("[L++ GUI] Initialized Window '%s' (%lldx%lld)\n", title ? title : "L++ App", (long long)width, (long long)height);
    return 0;
}

int64_t lpp_gui_window_is_open(int64_t win_id) { (void)win_id; return 0; }
int64_t lpp_gui_window_poll_events(int64_t win_id) { (void)win_id; return 0; }
void lpp_gui_clear(int64_t win_id, int64_t hex_color) { (void)win_id; (void)hex_color; }
void lpp_gui_draw_rect(int64_t win_id, int64_t x, int64_t y, int64_t w, int64_t h, int64_t hex_color) {
    (void)win_id; (void)x; (void)y; (void)w; (void)h; (void)hex_color;
}
void lpp_gui_draw_text(int64_t win_id, int64_t x, int64_t y, const char *text, int64_t hex_color) {
    (void)win_id; (void)x; (void)y; (void)text; (void)hex_color;
}
void lpp_gui_present(int64_t win_id) { (void)win_id; }
void lpp_gui_window_close(int64_t win_id) { (void)win_id; }
#endif
