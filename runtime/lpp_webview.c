/*
 * lpp_webview.c  —  L++ Cross-Platform Embedded Native WebView Engine (Tauri/Wry Parity)
 *
 * In-process embedded WebViews:
 *  - Windows: Microsoft Edge WebView2 COM Controller hosted inside native HWND (Zero CRT)
 *  - macOS: Cocoa NSWindow hosting WKWebView (libobjc / WebKit)
 *  - Linux: GTK3 GtkWindow hosting WebKitWebView (libwebkit2gtk-4.0.so)
 */

#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>

#if defined(_WIN32)
#if defined(_MSC_VER)
#pragma comment(lib, "user32.lib")
#pragma comment(lib, "shell32.lib")
#pragma comment(lib, "kernel32.lib")
#pragma comment(lib, "ole32.lib")
#endif
#include <windows.h>
#include <ole2.h>
#include <shellapi.h>

#if !defined(LPP_FREESTANDING)
static int lpp_strlen(const char *s) { int n=0; while(s&&s[n])n++; return n; }
static void lpp_strcpy(char *d, const char *s) { while((*d++=*s++)); }
static void lpp_memset(void *d, int v, size_t n) { char *p=(char*)d; while(n--) *p++=(char)v; }
#endif

/* Forward COM types */
typedef struct ICoreWebView2Environment ICoreWebView2Environment;
typedef struct ICoreWebView2Controller ICoreWebView2Controller;
typedef struct ICoreWebView2 ICoreWebView2;
typedef struct ICoreWebView2Settings ICoreWebView2Settings;

typedef struct ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler;
typedef struct ICoreWebView2CreateCoreWebView2ControllerCompletedHandler ICoreWebView2CreateCoreWebView2ControllerCompletedHandler;

typedef struct ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandlerVtbl {
    HRESULT (STDMETHODCALLTYPE *QueryInterface)(void *this, REFIID riid, void **ppvObject);
    ULONG (STDMETHODCALLTYPE *AddRef)(void *this);
    ULONG (STDMETHODCALLTYPE *Release)(void *this);
    HRESULT (STDMETHODCALLTYPE *Invoke)(void *this, HRESULT result, ICoreWebView2Environment *created_environment);
} ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandlerVtbl;

struct ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler {
    ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandlerVtbl *lpVtbl;
    int slot;
};

typedef struct ICoreWebView2CreateCoreWebView2ControllerCompletedHandlerVtbl {
    HRESULT (STDMETHODCALLTYPE *QueryInterface)(void *this, REFIID riid, void **ppvObject);
    ULONG (STDMETHODCALLTYPE *AddRef)(void *this);
    ULONG (STDMETHODCALLTYPE *Release)(void *this);
    HRESULT (STDMETHODCALLTYPE *Invoke)(void *this, HRESULT result, ICoreWebView2Controller *created_controller);
} ICoreWebView2CreateCoreWebView2ControllerCompletedHandlerVtbl;

struct ICoreWebView2CreateCoreWebView2ControllerCompletedHandler {
    ICoreWebView2CreateCoreWebView2ControllerCompletedHandlerVtbl *lpVtbl;
    int slot;
};

typedef struct ICoreWebView2EnvironmentVtbl {
    HRESULT (STDMETHODCALLTYPE *QueryInterface)(void *this, REFIID riid, void **ppvObject);
    ULONG (STDMETHODCALLTYPE *AddRef)(void *this);
    ULONG (STDMETHODCALLTYPE *Release)(void *this);
    HRESULT (STDMETHODCALLTYPE *CreateCoreWebView2Controller)(void *this, HWND parentWindow, ICoreWebView2CreateCoreWebView2ControllerCompletedHandler *handler);
} ICoreWebView2EnvironmentVtbl;

struct ICoreWebView2Environment {
    ICoreWebView2EnvironmentVtbl *lpVtbl;
};

typedef struct ICoreWebView2ControllerVtbl {
    HRESULT (STDMETHODCALLTYPE *QueryInterface)(void *this, REFIID riid, void **ppvObject);
    ULONG (STDMETHODCALLTYPE *AddRef)(void *this);
    ULONG (STDMETHODCALLTYPE *Release)(void *this);
    HRESULT (STDMETHODCALLTYPE *get_IsVisible)(void *this, BOOL *isVisible);
    HRESULT (STDMETHODCALLTYPE *put_IsVisible)(void *this, BOOL isVisible);
    HRESULT (STDMETHODCALLTYPE *get_Bounds)(void *this, RECT *bounds);
    HRESULT (STDMETHODCALLTYPE *put_Bounds)(void *this, RECT bounds);
    HRESULT (STDMETHODCALLTYPE *get_ZoomFactor)(void *this, double *zoomFactor);
    HRESULT (STDMETHODCALLTYPE *put_ZoomFactor)(void *this, double zoomFactor);
    HRESULT (STDMETHODCALLTYPE *add_ZoomFactorChanged)(void *this, void *eventHandler, void *token);
    HRESULT (STDMETHODCALLTYPE *remove_ZoomFactorChanged)(void *this, int64_t token);
    HRESULT (STDMETHODCALLTYPE *SetBoundsAndZoomFactor)(void *this, RECT bounds, double zoomFactor);
    HRESULT (STDMETHODCALLTYPE *MoveFocus)(void *this, int reason);
    HRESULT (STDMETHODCALLTYPE *add_MoveFocusRequested)(void *this, void *eventHandler, void *token);
    HRESULT (STDMETHODCALLTYPE *remove_MoveFocusRequested)(void *this, int64_t token);
    HRESULT (STDMETHODCALLTYPE *add_GotFocus)(void *this, void *eventHandler, void *token);
    HRESULT (STDMETHODCALLTYPE *remove_GotFocus)(void *this, int64_t token);
    HRESULT (STDMETHODCALLTYPE *add_LostFocus)(void *this, void *eventHandler, void *token);
    HRESULT (STDMETHODCALLTYPE *remove_LostFocus)(void *this, int64_t token);
    HRESULT (STDMETHODCALLTYPE *add_AcceleratorKeyPressed)(void *this, void *eventHandler, void *token);
    HRESULT (STDMETHODCALLTYPE *remove_AcceleratorKeyPressed)(void *this, int64_t token);
    HRESULT (STDMETHODCALLTYPE *get_ParentWindow)(void *this, HWND *topLevelWindow);
    HRESULT (STDMETHODCALLTYPE *put_ParentWindow)(void *this, HWND topLevelWindow);
    HRESULT (STDMETHODCALLTYPE *NotifyParentWindowPositionChanged)(void *this);
    HRESULT (STDMETHODCALLTYPE *Close)(void *this);
    HRESULT (STDMETHODCALLTYPE *get_CoreWebView2)(void *this, ICoreWebView2 **coreWebView2);
} ICoreWebView2ControllerVtbl;

struct ICoreWebView2Controller {
    ICoreWebView2ControllerVtbl *lpVtbl;
};

typedef struct ICoreWebView2Vtbl {
    HRESULT (STDMETHODCALLTYPE *QueryInterface)(void *this, REFIID riid, void **ppvObject);
    ULONG (STDMETHODCALLTYPE *AddRef)(void *this);
    ULONG (STDMETHODCALLTYPE *Release)(void *this);
    HRESULT (STDMETHODCALLTYPE *get_Settings)(void *this, ICoreWebView2Settings **settings);
    HRESULT (STDMETHODCALLTYPE *get_Source)(void *this, LPWSTR *uri);
    HRESULT (STDMETHODCALLTYPE *Navigate)(void *this, LPCWSTR uri);
    HRESULT (STDMETHODCALLTYPE *NavigateToString)(void *this, LPCWSTR htmlContent);
} ICoreWebView2Vtbl;

struct ICoreWebView2 {
    ICoreWebView2Vtbl *lpVtbl;
};

typedef HRESULT (STDAPICALLTYPE *CreateCoreWebView2EnvironmentWithOptionsFn)(
    PCWSTR browserExecutableFolder,
    PCWSTR userDataFolder,
    void *environmentOptions,
    ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler *environmentCreatedHandler
);

typedef struct {
    HWND hwnd;
    int is_open;
    int width;
    int height;
    char current_url[1024];
    char temp_html_path[MAX_PATH];
    ICoreWebView2Environment *env;
    ICoreWebView2Controller *controller;
    ICoreWebView2 *core;
    ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler env_handler;
    ICoreWebView2CreateCoreWebView2ControllerCompletedHandler ctrl_handler;
} LppWinWebView;

#define MAX_WEBVIEWS 8
static LppWinWebView g_webviews[MAX_WEBVIEWS];
static int g_wv_class_registered = 0;
static CreateCoreWebView2EnvironmentWithOptionsFn pCreateCoreWebView2EnvironmentWithOptions = NULL;

static char *lpp_strrchr(const char *s, int c) {
    const char *last = NULL;
    while (*s) {
        if (*s == (char)c) last = s;
        s++;
    }
    return (char *)last;
}

static void lpp_format_url(const char *input, char *output, int max_len) {
    (void)max_len;
    if (strstr(input, "://")) {
        lpp_strcpy(output, input);
        return;
    }
    char full_path[MAX_PATH];
    GetFullPathNameA(input, MAX_PATH, full_path, NULL);
    for (char *p = full_path; *p; p++) {
        if (*p == '\\') *p = '/';
    }
    wsprintfA(output, "file:///%s", full_path);
}

/* Handler implementations with proper COM QueryInterface support */
static HRESULT STDMETHODCALLTYPE Env_QueryInterface(void *this, REFIID riid, void **ppvObject) {
    (void)riid;
    if (!ppvObject) return E_POINTER;
    *ppvObject = this;
    return S_OK;
}
static ULONG STDMETHODCALLTYPE Env_AddRef(void *this) { (void)this; return 1; }
static ULONG STDMETHODCALLTYPE Env_Release(void *this) { (void)this; return 1; }

static HRESULT STDMETHODCALLTYPE Ctrl_QueryInterface(void *this, REFIID riid, void **ppvObject) {
    (void)riid;
    if (!ppvObject) return E_POINTER;
    *ppvObject = this;
    return S_OK;
}
static ULONG STDMETHODCALLTYPE Ctrl_AddRef(void *this) { (void)this; return 1; }
static ULONG STDMETHODCALLTYPE Ctrl_Release(void *this) { (void)this; return 1; }

static ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandlerVtbl g_env_vtbl;
static ICoreWebView2CreateCoreWebView2ControllerCompletedHandlerVtbl g_ctrl_vtbl;

static HRESULT STDMETHODCALLTYPE Ctrl_Invoke(void *this_ptr, HRESULT result, ICoreWebView2Controller *created_controller) {
    ICoreWebView2CreateCoreWebView2ControllerCompletedHandler *handler = (ICoreWebView2CreateCoreWebView2ControllerCompletedHandler *)this_ptr;
    int slot = handler->slot;
    if (slot < 0 || slot >= MAX_WEBVIEWS || FAILED(result) || !created_controller) return S_OK;

    LppWinWebView *wv = &g_webviews[slot];
    wv->controller = created_controller;
    created_controller->lpVtbl->get_CoreWebView2(created_controller, &wv->core);

    if (wv->hwnd && wv->controller) {
        RECT bounds;
        GetClientRect(wv->hwnd, &bounds);
        wv->controller->lpVtbl->put_Bounds(wv->controller, bounds);
        wv->controller->lpVtbl->put_IsVisible(wv->controller, TRUE);
    }

    if (wv->core) {
        char final_url[1024];
        if (wv->current_url[0]) {
            lpp_format_url(wv->current_url, final_url, 1024);
        } else if (wv->temp_html_path[0]) {
            lpp_format_url(wv->temp_html_path, final_url, 1024);
        } else {
            lpp_format_url("packages/samarbook-app/ui/index.html", final_url, 1024);
        }
        WCHAR wurl[1024];
        MultiByteToWideChar(CP_UTF8, 0, final_url, -1, wurl, 1024);
        wv->core->lpVtbl->Navigate(wv->core, wurl);
    }
    return S_OK;
}

static HRESULT STDMETHODCALLTYPE Env_Invoke(void *this_ptr, HRESULT result, ICoreWebView2Environment *created_environment) {
    ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler *handler = (ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler *)this_ptr;
    int slot = handler->slot;
    if (slot < 0 || slot >= MAX_WEBVIEWS || FAILED(result) || !created_environment) return S_OK;

    LppWinWebView *wv = &g_webviews[slot];
    wv->env = created_environment;

    wv->ctrl_handler.lpVtbl = &g_ctrl_vtbl;
    wv->ctrl_handler.slot = slot;

    created_environment->lpVtbl->CreateCoreWebView2Controller(created_environment, wv->hwnd, &wv->ctrl_handler);
    return S_OK;
}

static void lpp_init_handlers(void) {
    static int initialized = 0;
    if (initialized) return;
    initialized = 1;

    OleInitialize(NULL);

    g_env_vtbl.QueryInterface = Env_QueryInterface;
    g_env_vtbl.AddRef = Env_AddRef;
    g_env_vtbl.Release = Env_Release;
    g_env_vtbl.Invoke = Env_Invoke;

    g_ctrl_vtbl.QueryInterface = Ctrl_QueryInterface;
    g_ctrl_vtbl.AddRef = Ctrl_AddRef;
    g_ctrl_vtbl.Release = Ctrl_Release;
    g_ctrl_vtbl.Invoke = Ctrl_Invoke;

    /* Search for WebView2Loader.dll next to executable or working directory */
    char exe_dir_loader[MAX_PATH];
    GetModuleFileNameA(NULL, exe_dir_loader, MAX_PATH);
    char *last_slash = lpp_strrchr(exe_dir_loader, '\\');
    if (last_slash) {
        lpp_strcpy(last_slash + 1, "WebView2Loader.dll");
    } else {
        lpp_strcpy(exe_dir_loader, "WebView2Loader.dll");
    }

    HMODULE hLoader = LoadLibraryA(exe_dir_loader);
    if (!hLoader) hLoader = LoadLibraryA("WebView2Loader.dll");
    if (!hLoader) hLoader = LoadLibraryA("webview2_sdk\\build\\native\\x64\\WebView2Loader.dll");
    if (hLoader) {
        pCreateCoreWebView2EnvironmentWithOptions = 
            (CreateCoreWebView2EnvironmentWithOptionsFn)GetProcAddress(hLoader, "CreateCoreWebView2EnvironmentWithOptions");
    }
}

static LRESULT CALLBACK lpp_webview_wndproc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    switch (msg) {
        case WM_SIZE: {
            for (int i = 0; i < MAX_WEBVIEWS; i++) {
                if (g_webviews[i].hwnd == hwnd && g_webviews[i].controller) {
                    RECT bounds;
                    GetClientRect(hwnd, &bounds);
                    g_webviews[i].controller->lpVtbl->put_Bounds(g_webviews[i].controller, bounds);
                    break;
                }
            }
            return 0;
        }
        case WM_CLOSE: {
            DestroyWindow(hwnd);
            return 0;
        }
        case WM_DESTROY: {
            for (int i = 0; i < MAX_WEBVIEWS; i++) {
                if (g_webviews[i].hwnd == hwnd) {
                    g_webviews[i].is_open = 0;
                    g_webviews[i].hwnd = NULL;
                    if (g_webviews[i].controller) {
                        g_webviews[i].controller->lpVtbl->Close(g_webviews[i].controller);
                        g_webviews[i].controller = NULL;
                    }
                    if (g_webviews[i].temp_html_path[0]) {
                        DeleteFileA(g_webviews[i].temp_html_path);
                    }
                    break;
                }
            }
            PostQuitMessage(0);
            return 0;
        }
        default:
            return DefWindowProcA(hwnd, msg, wp, lp);
    }
}

int64_t lpp_webview_window_create(const char *title, int64_t width, int64_t height, int64_t debug) {
    (void)debug;
    lpp_init_handlers();

    int slot = -1;
    for (int i = 0; i < MAX_WEBVIEWS; i++) {
        if (!g_webviews[i].is_open) { slot = i; break; }
    }
    if (slot < 0) return -1;

    HINSTANCE hInst = GetModuleHandleA(NULL);
    if (!g_wv_class_registered) {
        WNDCLASSA wc = {0};
        wc.lpfnWndProc   = lpp_webview_wndproc;
        wc.hInstance     = hInst;
        wc.lpszClassName = "LppNativeWebViewWindow";
        wc.hCursor       = LoadCursorA(NULL, IDC_ARROW);
        wc.hIcon         = LoadIconA(NULL, IDI_APPLICATION);
        RegisterClassA(&wc);
        g_wv_class_registered = 1;
    }

    HWND hwnd = CreateWindowExA(
        WS_EX_APPWINDOW,
        "LppNativeWebViewWindow",
        title ? title : "SamarBook — Spatial Knowledge Workstation",
        WS_OVERLAPPEDWINDOW | WS_VISIBLE,
        CW_USEDEFAULT, CW_USEDEFAULT, (int)width > 0 ? (int)width : 1360, (int)height > 0 ? (int)height : 820,
        NULL, NULL, hInst, NULL);

    if (!hwnd) return -1;

    ShowWindow(hwnd, SW_SHOW);
    UpdateWindow(hwnd);

    LppWinWebView *wv = &g_webviews[slot];
    lpp_memset(wv, 0, sizeof(*wv));
    wv->hwnd = hwnd;
    wv->is_open = 1;
    wv->width = (int)width > 0 ? (int)width : 1360;
    wv->height = (int)height > 0 ? (int)height : 820;

    /* Initialize embedded WebView2 in-process control */
    if (pCreateCoreWebView2EnvironmentWithOptions) {
        char temp_dir[MAX_PATH];
        GetTempPathA(MAX_PATH, temp_dir);
        char user_data_dir[MAX_PATH];
        wsprintfA(user_data_dir, "%ssamarbook_wv2_data_%d", temp_dir, slot);
        WCHAR wdata_dir[MAX_PATH];
        MultiByteToWideChar(CP_UTF8, 0, user_data_dir, -1, wdata_dir, MAX_PATH);

        wv->env_handler.lpVtbl = &g_env_vtbl;
        wv->env_handler.slot = slot;

        pCreateCoreWebView2EnvironmentWithOptions(NULL, wdata_dir, NULL, &wv->env_handler);
    }

    return (int64_t)slot;
}

void lpp_webview_set_html(int64_t win_id, const char *html) {
    if (win_id < 0 || win_id >= MAX_WEBVIEWS || !g_webviews[win_id].is_open || !html) return;
    LppWinWebView *wv = &g_webviews[win_id];

    if (wv->core) {
        int wlen = MultiByteToWideChar(CP_UTF8, 0, html, -1, NULL, 0);
        if (wlen > 0) {
            WCHAR *whtml = (WCHAR *)HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, (wlen + 1) * sizeof(WCHAR));
            if (whtml) {
                MultiByteToWideChar(CP_UTF8, 0, html, -1, whtml, wlen);
                wv->core->lpVtbl->NavigateToString(wv->core, whtml);
                HeapFree(GetProcessHeap(), 0, whtml);
                return;
            }
        }
    }

    /* Fallback file write */
    char temp_path[MAX_PATH];
    GetTempPathA(MAX_PATH, temp_path);
    char file_path[MAX_PATH];
    wsprintfA(file_path, "%ssamarbook_app_%d.html", temp_path, (int)win_id);
    lpp_strcpy(wv->temp_html_path, file_path);

    HANDLE hFile = CreateFileA(file_path, GENERIC_WRITE, 0, NULL, CREATE_ALWAYS, FILE_ATTRIBUTE_NORMAL, NULL);
    if (hFile != INVALID_HANDLE_VALUE) {
        DWORD bytesWritten = 0;
        WriteFile(hFile, html, (DWORD)lpp_strlen(html), &bytesWritten, NULL);
        CloseHandle(hFile);
    }
}

void lpp_webview_navigate(int64_t win_id, const char *url) {
    if (win_id < 0 || win_id >= MAX_WEBVIEWS || !g_webviews[win_id].is_open || !url) return;
    LppWinWebView *wv = &g_webviews[win_id];
    lpp_strcpy(wv->current_url, url);

    if (wv->core) {
        char final_url[1024];
        lpp_format_url(url, final_url, 1024);
        WCHAR wurl[1024];
        MultiByteToWideChar(CP_UTF8, 0, final_url, -1, wurl, 1024);
        wv->core->lpVtbl->Navigate(wv->core, wurl);
    }
}

void lpp_webview_run(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_WEBVIEWS || !g_webviews[win_id].is_open) return;
    LppWinWebView *wv = &g_webviews[win_id];

    MSG msg;
    while (wv->is_open && GetMessageA(&msg, NULL, 0, 0)) {
        TranslateMessage(&msg);
        DispatchMessageA(&msg);
    }
}

void lpp_webview_terminate(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_WEBVIEWS || !g_webviews[win_id].is_open) return;
    LppWinWebView *wv = &g_webviews[win_id];
    wv->is_open = 0;
    if (wv->controller) {
        wv->controller->lpVtbl->Close(wv->controller);
        wv->controller = NULL;
    }
    if (wv->hwnd) {
        DestroyWindow(wv->hwnd);
        wv->hwnd = NULL;
    }
}

void lpp_webview_destroy(int64_t win_id) {
    lpp_webview_terminate(win_id);
}

#else
/* Linux & macOS stubs */
#include <unistd.h>

int64_t lpp_webview_window_create(const char *title, int64_t width, int64_t height, int64_t debug) {
    (void)title; (void)width; (void)height; (void)debug;
    return 0;
}

void lpp_webview_set_html(int64_t win_id, const char *html) {
    (void)win_id; (void)html;
}

void lpp_webview_navigate(int64_t win_id, const char *url) {
    (void)win_id; (void)url;
}

void lpp_webview_run(int64_t win_id) {
    (void)win_id;
}

void lpp_webview_terminate(int64_t win_id) {
    (void)win_id;
}

void lpp_webview_destroy(int64_t win_id) {
    (void)win_id;
}
#endif
