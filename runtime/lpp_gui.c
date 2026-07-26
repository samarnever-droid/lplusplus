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

void lpp_gui_draw_rounded_rect(int64_t win_id, int64_t x, int64_t y, int64_t w, int64_t h, int64_t radius, int64_t hex_color) {
    if (win_id < 0 || win_id >= g_win_count || !g_windows[win_id].is_open) return;
    BYTE r_val = (BYTE)((hex_color >> 16) & 0xFF);
    BYTE g_val = (BYTE)((hex_color >> 8) & 0xFF);
    BYTE b_val = (BYTE)(hex_color & 0xFF);

    COLORREF col = RGB(r_val, g_val, b_val);
    HBRUSH brush = CreateSolidBrush(col);
    HBRUSH old_brush = (HBRUSH)SelectObject(g_windows[win_id].mem_dc, brush);
    HPEN pen = CreatePen(PS_NULL, 0, 0);
    HPEN old_pen = (HPEN)SelectObject(g_windows[win_id].mem_dc, pen);

    int r_diam = (int)(radius * 2);
    RoundRect(g_windows[win_id].mem_dc, (int)x, (int)y, (int)(x + w), (int)(y + h), r_diam, r_diam);

    SelectObject(g_windows[win_id].mem_dc, old_brush);
    SelectObject(g_windows[win_id].mem_dc, old_pen);
    DeleteObject(brush);
    DeleteObject(pen);
}

int64_t lpp_gui_mouse_x(int64_t win_id) {
    if (win_id < 0 || win_id >= g_win_count || !g_windows[win_id].is_open) return 0;
    POINT pt;
    GetCursorPos(&pt);
    ScreenToClient(g_windows[win_id].hwnd, &pt);
    return (int64_t)pt.x;
}

int64_t lpp_gui_mouse_y(int64_t win_id) {
    if (win_id < 0 || win_id >= g_win_count || !g_windows[win_id].is_open) return 0;
    POINT pt;
    GetCursorPos(&pt);
    ScreenToClient(g_windows[win_id].hwnd, &pt);
    return (int64_t)pt.y;
}

int64_t lpp_gui_mouse_down(int64_t win_id) {
    if (win_id < 0 || win_id >= g_win_count || !g_windows[win_id].is_open) return 0;
    return (GetAsyncKeyState(VK_LBUTTON) & 0x8000) ? 1 : 0;
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
/* ── Unix (Linux / macOS) X11 & Headless Implementation ───────────────── */
#include <dlfcn.h>
#include <unistd.h>

typedef struct {
    void *display;
    unsigned long window;
    unsigned long gc;
    int width;
    int height;
    int is_open;
} LppUnixWindow;

static LppUnixWindow g_unix_windows[8];
static int g_unix_win_count = 0;
static void *g_x11_lib = NULL;

/* X11 Function Pointers */
typedef void* (*fn_XOpenDisplay)(const char*);
typedef unsigned long (*fn_XCreateSimpleWindow)(void*, unsigned long, int, int, unsigned int, unsigned int, unsigned int, unsigned long, unsigned long);
typedef int (*fn_XSelectInput)(void*, unsigned long, long);
typedef int (*fn_XMapWindow)(void*, unsigned long);
typedef void* (*fn_XCreateGC)(void*, unsigned long, unsigned long, void*);
typedef int (*fn_XSetForeground)(void*, void*, unsigned long);
typedef int (*fn_XFillRectangle)(void*, unsigned long, void*, int, int, unsigned int, unsigned int);
typedef int (*fn_XDrawString)(void*, unsigned long, void*, int, int, const char*, int);
typedef int (*fn_XFlush)(void*);
typedef int (*fn_XPending)(void*);
typedef int (*fn_XNextEvent)(void*, void*);
typedef int (*fn_XQueryPointer)(void*, unsigned long, unsigned long*, unsigned long*, int*, int*, int*, int*, unsigned int*);
typedef int (*fn_XCloseDisplay)(void*);
typedef int (*fn_XDestroyWindow)(void*, unsigned long);

static fn_XOpenDisplay p_XOpenDisplay = NULL;
static fn_XCreateSimpleWindow p_XCreateSimpleWindow = NULL;
static fn_XSelectInput p_XSelectInput = NULL;
static fn_XMapWindow p_XMapWindow = NULL;
static fn_XCreateGC p_XCreateGC = NULL;
static fn_XSetForeground p_XSetForeground = NULL;
static fn_XFillRectangle p_XFillRectangle = NULL;
static fn_XDrawString p_XDrawString = NULL;
static fn_XFlush p_XFlush = NULL;
static fn_XPending p_XPending = NULL;
static fn_XNextEvent p_XNextEvent = NULL;
static fn_XQueryPointer p_XQueryPointer = NULL;
static fn_XCloseDisplay p_XCloseDisplay = NULL;
static fn_XDestroyWindow p_XDestroyWindow = NULL;

static int init_x11(void) {
    if (g_x11_lib) return 1;
    g_x11_lib = dlopen("libX11.so.6", RTLD_LAZY);
    if (!g_x11_lib) g_x11_lib = dlopen("libX11.so", RTLD_LAZY);
    if (!g_x11_lib) return 0;

    p_XOpenDisplay = (fn_XOpenDisplay)dlsym(g_x11_lib, "XOpenDisplay");
    p_XCreateSimpleWindow = (fn_XCreateSimpleWindow)dlsym(g_x11_lib, "XCreateSimpleWindow");
    p_XSelectInput = (fn_XSelectInput)dlsym(g_x11_lib, "XSelectInput");
    p_XMapWindow = (fn_XMapWindow)dlsym(g_x11_lib, "XMapWindow");
    p_XCreateGC = (fn_XCreateGC)dlsym(g_x11_lib, "XCreateGC");
    p_XSetForeground = (fn_XSetForeground)dlsym(g_x11_lib, "XSetForeground");
    p_XFillRectangle = (fn_XFillRectangle)dlsym(g_x11_lib, "XFillRectangle");
    p_XDrawString = (fn_XDrawString)dlsym(g_x11_lib, "XDrawString");
    p_XFlush = (fn_XFlush)dlsym(g_x11_lib, "XFlush");
    p_XPending = (fn_XPending)dlsym(g_x11_lib, "XPending");
    p_XNextEvent = (fn_XNextEvent)dlsym(g_x11_lib, "XNextEvent");
    p_XQueryPointer = (fn_XQueryPointer)dlsym(g_x11_lib, "XQueryPointer");
    p_XCloseDisplay = (fn_XCloseDisplay)dlsym(g_x11_lib, "XCloseDisplay");
    p_XDestroyWindow = (fn_XDestroyWindow)dlsym(g_x11_lib, "XDestroyWindow");

    return p_XOpenDisplay && p_XCreateSimpleWindow && p_XMapWindow;
}

int64_t lpp_gui_window_create(const char *title, int64_t width, int64_t height) {
    if (g_unix_win_count >= 8) return -1;

    if (init_x11()) {
        void *display = p_XOpenDisplay(NULL);
        if (display) {
            unsigned long root = 0; // DefaultRootWindow fallback
            unsigned long win = p_XCreateSimpleWindow(display, root, 10, 10, (unsigned int)width, (unsigned int)height, 1, 0, 0x121212);
            p_XSelectInput(display, win, 0x1L | 0x4L | 0x8L); // ExposureMask | ButtonPressMask | PointerMotionMask
            p_XMapWindow(display, win);
            void *gc = p_XCreateGC(display, win, 0, NULL);

            int id = g_unix_win_count++;
            g_unix_windows[id].display = display;
            g_unix_windows[id].window = win;
            g_unix_windows[id].gc = (unsigned long)(uintptr_t)gc;
            g_unix_windows[id].width = (int)width;
            g_unix_windows[id].height = (int)height;
            g_unix_windows[id].is_open = 1;
            return (int64_t)id;
        }
    }

    printf("[L++ GUI] Initialized Window '%s' (%lldx%lld)\n", title ? title : "L++ App", (long long)width, (long long)height);
    int id = g_unix_win_count++;
    g_unix_windows[id].is_open = 1;
    return (int64_t)id;
}

int64_t lpp_gui_window_is_open(int64_t win_id) {
    if (win_id < 0 || win_id >= g_unix_win_count) return 0;
    return g_unix_windows[win_id].is_open ? 1 : 0;
}

int64_t lpp_gui_window_poll_events(int64_t win_id) {
    if (win_id < 0 || win_id >= g_unix_win_count || !g_unix_windows[win_id].is_open) return 0;
    if (g_unix_windows[win_id].display && p_XPending) {
        char event_buf[256];
        while (p_XPending(g_unix_windows[win_id].display)) {
            p_XNextEvent(g_unix_windows[win_id].display, event_buf);
        }
    }
    return g_unix_windows[win_id].is_open ? 1 : 0;
}

void lpp_gui_clear(int64_t win_id, int64_t hex_color) {
    if (win_id < 0 || win_id >= g_unix_win_count || !g_unix_windows[win_id].is_open) return;
    if (g_unix_windows[win_id].display && p_XSetForeground && p_XFillRectangle) {
        void *gc = (void *)(uintptr_t)g_unix_windows[win_id].gc;
        p_XSetForeground(g_unix_windows[win_id].display, gc, (unsigned long)hex_color);
        p_XFillRectangle(g_unix_windows[win_id].display, g_unix_windows[win_id].window, gc, 0, 0, (unsigned int)g_unix_windows[win_id].width, (unsigned int)g_unix_windows[win_id].height);
    }
}

void lpp_gui_draw_rect(int64_t win_id, int64_t x, int64_t y, int64_t w, int64_t h, int64_t hex_color) {
    if (win_id < 0 || win_id >= g_unix_win_count || !g_unix_windows[win_id].is_open) return;
    if (g_unix_windows[win_id].display && p_XSetForeground && p_XFillRectangle) {
        void *gc = (void *)(uintptr_t)g_unix_windows[win_id].gc;
        p_XSetForeground(g_unix_windows[win_id].display, gc, (unsigned long)hex_color);
        p_XFillRectangle(g_unix_windows[win_id].display, g_unix_windows[win_id].window, gc, (int)x, (int)y, (unsigned int)w, (unsigned int)h);
    }
}

void lpp_gui_draw_rounded_rect(int64_t win_id, int64_t x, int64_t y, int64_t w, int64_t h, int64_t radius, int64_t hex_color) {
    (void)radius;
    lpp_gui_draw_rect(win_id, x, y, w, h, hex_color);
}

int64_t lpp_gui_mouse_x(int64_t win_id) {
    if (win_id < 0 || win_id >= g_unix_win_count || !g_unix_windows[win_id].is_open) return 0;
    if (g_unix_windows[win_id].display && p_XQueryPointer) {
        unsigned long root, child;
        int rx, ry, wx, wy;
        unsigned int mask;
        if (p_XQueryPointer(g_unix_windows[win_id].display, g_unix_windows[win_id].window, &root, &child, &rx, &ry, &wx, &wy, &mask)) {
            return (int64_t)wx;
        }
    }
    return 0;
}

int64_t lpp_gui_mouse_y(int64_t win_id) {
    if (win_id < 0 || win_id >= g_unix_win_count || !g_unix_windows[win_id].is_open) return 0;
    if (g_unix_windows[win_id].display && p_XQueryPointer) {
        unsigned long root, child;
        int rx, ry, wx, wy;
        unsigned int mask;
        if (p_XQueryPointer(g_unix_windows[win_id].display, g_unix_windows[win_id].window, &root, &child, &rx, &ry, &wx, &wy, &mask)) {
            return (int64_t)wy;
        }
    }
    return 0;
}

int64_t lpp_gui_mouse_down(int64_t win_id) {
    if (win_id < 0 || win_id >= g_unix_win_count || !g_unix_windows[win_id].is_open) return 0;
    if (g_unix_windows[win_id].display && p_XQueryPointer) {
        unsigned long root, child;
        int rx, ry, wx, wy;
        unsigned int mask;
        if (p_XQueryPointer(g_unix_windows[win_id].display, g_unix_windows[win_id].window, &root, &child, &rx, &ry, &wx, &wy, &mask)) {
            return (mask & (1 << 8)) ? 1 : 0; // Button1Mask (0x100)
        }
    }
    return 0;
}

void lpp_gui_draw_text(int64_t win_id, int64_t x, int64_t y, const char *text, int64_t hex_color) {
    if (win_id < 0 || win_id >= g_unix_win_count || !g_unix_windows[win_id].is_open || !text) return;
    if (g_unix_windows[win_id].display && p_XSetForeground && p_XDrawString) {
        void *gc = (void *)(uintptr_t)g_unix_windows[win_id].gc;
        p_XSetForeground(g_unix_windows[win_id].display, gc, (unsigned long)hex_color);
        p_XDrawString(g_unix_windows[win_id].display, g_unix_windows[win_id].window, gc, (int)x, (int)y, text, (int)strlen(text));
    }
}

void lpp_gui_present(int64_t win_id) {
    if (win_id < 0 || win_id >= g_unix_win_count || !g_unix_windows[win_id].is_open) return;
    if (g_unix_windows[win_id].display && p_XFlush) {
        p_XFlush(g_unix_windows[win_id].display);
    }
}

void lpp_gui_window_close(int64_t win_id) {
    if (win_id < 0 || win_id >= g_unix_win_count) return;
    if (g_unix_windows[win_id].is_open) {
        g_unix_windows[win_id].is_open = 0;
        if (g_unix_windows[win_id].display && p_XDestroyWindow && p_XCloseDisplay) {
            p_XDestroyWindow(g_unix_windows[win_id].display, g_unix_windows[win_id].window);
            p_XCloseDisplay(g_unix_windows[win_id].display);
        }
    }
}
#endif
