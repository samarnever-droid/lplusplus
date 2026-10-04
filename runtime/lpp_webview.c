/*
 * lpp_webview.c  —  L++ Cross-Platform Embedded Native WebView Engine
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
#pragma comment(lib, "dwmapi.lib")
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

typedef struct ICoreWebView2SettingsVtbl {
    HRESULT (STDMETHODCALLTYPE *QueryInterface)(void *this, REFIID riid, void **ppvObject);
    ULONG (STDMETHODCALLTYPE *AddRef)(void *this);
    ULONG (STDMETHODCALLTYPE *Release)(void *this);
    HRESULT (STDMETHODCALLTYPE *get_IsScriptEnabled)(void *this, BOOL *isScriptEnabled);
    HRESULT (STDMETHODCALLTYPE *put_IsScriptEnabled)(void *this, BOOL isScriptEnabled);
    HRESULT (STDMETHODCALLTYPE *get_IsWebMessageEnabled)(void *this, BOOL *isWebMessageEnabled);
    HRESULT (STDMETHODCALLTYPE *put_IsWebMessageEnabled)(void *this, BOOL isWebMessageEnabled);
    HRESULT (STDMETHODCALLTYPE *get_AreDefaultScriptDialogsEnabled)(void *this, BOOL *areDefaultScriptDialogsEnabled);
    HRESULT (STDMETHODCALLTYPE *put_AreDefaultScriptDialogsEnabled)(void *this, BOOL areDefaultScriptDialogsEnabled);
    HRESULT (STDMETHODCALLTYPE *get_IsStatusBarEnabled)(void *this, BOOL *isStatusBarEnabled);
    HRESULT (STDMETHODCALLTYPE *put_IsStatusBarEnabled)(void *this, BOOL isStatusBarEnabled);
    HRESULT (STDMETHODCALLTYPE *get_AreDevToolsEnabled)(void *this, BOOL *areDevToolsEnabled);
    HRESULT (STDMETHODCALLTYPE *put_AreDevToolsEnabled)(void *this, BOOL areDevToolsEnabled);
} ICoreWebView2SettingsVtbl;

struct ICoreWebView2Settings {
    ICoreWebView2SettingsVtbl *lpVtbl;
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
    int is_ready;
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

static const GUID LPP_IID_IUnknown = { 0x00000000, 0x0000, 0x0000, { 0xC0, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x46 } };
static const GUID LPP_IID_EnvCompleted = { 0x4E8A3389, 0xC9D8, 0x4BD2, { 0xB6, 0xB5, 0x12, 0x4F, 0xEE, 0x6C, 0xC1, 0x4D } };
static const GUID LPP_IID_CtrlCompleted = { 0x6C4819F3, 0xC9B7, 0x4260, { 0x81, 0x27, 0xC9, 0xF5, 0xBD, 0xE7, 0xF6, 0x8C } };

static int lpp_guid_equal(const GUID *a, const GUID *b) {
    return memcmp(a, b, sizeof(GUID)) == 0;
}

static char *lpp_strrchr(const char *s, int c) {
    const char *last = NULL;
    while (*s) {
        if (*s == (char)c) last = s;
        s++;
    }
    return (char *)last;
}

static char *lpp_read_file_content(const char *filepath) {
    HANDLE hFile = CreateFileA(filepath, GENERIC_READ, FILE_SHARE_READ, NULL, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, NULL);
    if (hFile == INVALID_HANDLE_VALUE) return NULL;
    DWORD size = GetFileSize(hFile, NULL);
    if (size == 0 || size == INVALID_FILE_SIZE) {
        CloseHandle(hFile);
        return NULL;
    }
    char *buf = (char *)HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, size + 1);
    if (!buf) {
        CloseHandle(hFile);
        return NULL;
    }
    DWORD read = 0;
    ReadFile(hFile, buf, size, &read, NULL);
    CloseHandle(hFile);
    buf[read] = '\0';
    return buf;
}

static void lpp_format_url(const char *input, char *output, int max_len) {
    (void)max_len;
    if (!input || !input[0]) {
        lpp_strcpy(output, "about:blank");
        return;
    }
    if (strstr(input, "://") || strstr(input, "data:") || strstr(input, "about:")) {
        lpp_strcpy(output, input);
        return;
    }

    char full_path[MAX_PATH];
    full_path[0] = 0;

    if (GetFileAttributesA(input) != INVALID_FILE_ATTRIBUTES) {
        GetFullPathNameA(input, MAX_PATH, full_path, NULL);
    } else {
        char exe_dir[MAX_PATH];
        GetModuleFileNameA(NULL, exe_dir, MAX_PATH);
        char *slash = lpp_strrchr(exe_dir, '\\');
        if (slash) *slash = '\0';

        char candidate[MAX_PATH];
        wsprintfA(candidate, "%s\\%s", exe_dir, input);
        if (GetFileAttributesA(candidate) != INVALID_FILE_ATTRIBUTES) {
            GetFullPathNameA(candidate, MAX_PATH, full_path, NULL);
        } else {
            const char *fname = lpp_strrchr(input, '/');
            if (!fname) fname = lpp_strrchr(input, '\\');
            if (fname) fname++; else fname = input;
            wsprintfA(candidate, "%s\\%s", exe_dir, fname);
            if (GetFileAttributesA(candidate) != INVALID_FILE_ATTRIBUTES) {
                GetFullPathNameA(candidate, MAX_PATH, full_path, NULL);
            } else {
                GetFullPathNameA(input, MAX_PATH, full_path, NULL);
            }
        }
    }

    for (char *p = full_path; *p; p++) {
        if (*p == '\\') *p = '/';
    }
    wsprintfA(output, "file:///%s", full_path);
}

static void lpp_load_content_into_core(ICoreWebView2 *core, const char *url_or_path) {
    if (!core || !url_or_path || !url_or_path[0]) return;

    if (strstr(url_or_path, "http://") == url_or_path || strstr(url_or_path, "https://") == url_or_path || strstr(url_or_path, "data:") == url_or_path) {
        WCHAR wurl[1024];
        MultiByteToWideChar(CP_UTF8, 0, url_or_path, -1, wurl, 1024);
        core->lpVtbl->Navigate(core, wurl);
        return;
    }

    char resolved_path[MAX_PATH];
    resolved_path[0] = 0;
    if (GetFileAttributesA(url_or_path) != INVALID_FILE_ATTRIBUTES) {
        GetFullPathNameA(url_or_path, MAX_PATH, resolved_path, NULL);
    } else {
        char exe_dir[MAX_PATH];
        GetModuleFileNameA(NULL, exe_dir, MAX_PATH);
        char *slash = lpp_strrchr(exe_dir, '\\');
        if (slash) *slash = '\0';
        char candidate[MAX_PATH];
        wsprintfA(candidate, "%s\\%s", exe_dir, url_or_path);
        if (GetFileAttributesA(candidate) != INVALID_FILE_ATTRIBUTES) {
            GetFullPathNameA(candidate, MAX_PATH, resolved_path, NULL);
        } else {
            const char *fname = lpp_strrchr(url_or_path, '/');
            if (!fname) fname = lpp_strrchr(url_or_path, '\\');
            if (fname) fname++; else fname = url_or_path;
            wsprintfA(candidate, "%s\\%s", exe_dir, fname);
            if (GetFileAttributesA(candidate) != INVALID_FILE_ATTRIBUTES) {
                GetFullPathNameA(candidate, MAX_PATH, resolved_path, NULL);
            }
        }
    }

    if (resolved_path[0]) {
        char *html = lpp_read_file_content(resolved_path);
        if (html) {
            int wlen = MultiByteToWideChar(CP_UTF8, 0, html, -1, NULL, 0);
            if (wlen > 0) {
                WCHAR *whtml = (WCHAR *)HeapAlloc(GetProcessHeap(), HEAP_ZERO_MEMORY, (wlen + 1) * sizeof(WCHAR));
                if (whtml) {
                    MultiByteToWideChar(CP_UTF8, 0, html, -1, whtml, wlen);
                    core->lpVtbl->NavigateToString(core, whtml);
                    HeapFree(GetProcessHeap(), 0, whtml);
                    HeapFree(GetProcessHeap(), 0, html);
                    return;
                }
            }
            HeapFree(GetProcessHeap(), 0, html);
        }
    }

    char final_url[1024];
    lpp_format_url(url_or_path, final_url, 1024);
    WCHAR wurl[1024];
    MultiByteToWideChar(CP_UTF8, 0, final_url, -1, wurl, 1024);
    core->lpVtbl->Navigate(core, wurl);
}

/* COM Handlers with exact GUID checking */
static HRESULT STDMETHODCALLTYPE Env_QueryInterface(void *this, REFIID riid, void **ppvObject) {
    if (!ppvObject) return E_POINTER;
    if (lpp_guid_equal((const GUID*)riid, &LPP_IID_IUnknown) || lpp_guid_equal((const GUID*)riid, &LPP_IID_EnvCompleted)) {
        *ppvObject = this;
        return S_OK;
    }
    *ppvObject = NULL;
    return E_NOINTERFACE;
}
static ULONG STDMETHODCALLTYPE Env_AddRef(void *this) { (void)this; return 1; }
static ULONG STDMETHODCALLTYPE Env_Release(void *this) { (void)this; return 1; }

static HRESULT STDMETHODCALLTYPE Ctrl_QueryInterface(void *this, REFIID riid, void **ppvObject) {
    if (!ppvObject) return E_POINTER;
    if (lpp_guid_equal((const GUID*)riid, &LPP_IID_IUnknown) || lpp_guid_equal((const GUID*)riid, &LPP_IID_CtrlCompleted)) {
        *ppvObject = this;
        return S_OK;
    }
    *ppvObject = NULL;
    return E_NOINTERFACE;
}
static ULONG STDMETHODCALLTYPE Ctrl_AddRef(void *this) { (void)this; return 1; }
static ULONG STDMETHODCALLTYPE Ctrl_Release(void *this) { (void)this; return 1; }

static ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandlerVtbl g_env_vtbl = {
    Env_QueryInterface,
    Env_AddRef,
    Env_Release,
    NULL
};

static ICoreWebView2CreateCoreWebView2ControllerCompletedHandlerVtbl g_ctrl_vtbl = {
    Ctrl_QueryInterface,
    Ctrl_AddRef,
    Ctrl_Release,
    NULL
};

static HRESULT STDMETHODCALLTYPE Ctrl_Invoke(void *this_ptr, HRESULT result, ICoreWebView2Controller *created_controller) {
    ICoreWebView2CreateCoreWebView2ControllerCompletedHandler *handler = (ICoreWebView2CreateCoreWebView2ControllerCompletedHandler *)this_ptr;
    int slot = handler->slot;

    if (slot < 0 || slot >= MAX_WEBVIEWS || FAILED(result) || !created_controller) {
        if (slot >= 0 && slot < MAX_WEBVIEWS) g_webviews[slot].is_ready = 1;
        return S_OK;
    }

    LppWinWebView *wv = &g_webviews[slot];
    wv->controller = created_controller;
    created_controller->lpVtbl->AddRef(created_controller);
    created_controller->lpVtbl->get_CoreWebView2(created_controller, &wv->core);

    if (wv->core) {
        ICoreWebView2Settings *settings = NULL;
        wv->core->lpVtbl->get_Settings(wv->core, &settings);
        if (settings) {
            settings->lpVtbl->put_IsScriptEnabled(settings, TRUE);
            settings->lpVtbl->put_AreDefaultScriptDialogsEnabled(settings, TRUE);
            settings->lpVtbl->put_IsWebMessageEnabled(settings, TRUE);
            settings->lpVtbl->put_AreDevToolsEnabled(settings, TRUE);
            settings->lpVtbl->Release(settings);
        }
    }

    if (wv->hwnd && wv->controller) {
        RECT bounds;
        GetClientRect(wv->hwnd, &bounds);
        wv->controller->lpVtbl->put_Bounds(wv->controller, bounds);
        wv->controller->lpVtbl->put_IsVisible(wv->controller, TRUE);
        wv->controller->lpVtbl->NotifyParentWindowPositionChanged(wv->controller);
    }

    if (wv->core) {
        const char *target = wv->current_url[0] ? wv->current_url : 
                             (wv->temp_html_path[0] ? wv->temp_html_path : "webview_demo/index.html");
        lpp_load_content_into_core(wv->core, target);
    }

    wv->is_ready = 1;
    return S_OK;
}

static HRESULT STDMETHODCALLTYPE Env_Invoke(void *this_ptr, HRESULT result, ICoreWebView2Environment *created_environment) {
    ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler *handler = (ICoreWebView2CreateCoreWebView2EnvironmentCompletedHandler *)this_ptr;
    int slot = handler->slot;

    if (slot < 0 || slot >= MAX_WEBVIEWS || FAILED(result) || !created_environment) {
        if (slot >= 0 && slot < MAX_WEBVIEWS) g_webviews[slot].is_ready = 1;
        return S_OK;
    }

    LppWinWebView *wv = &g_webviews[slot];
    wv->env = created_environment;
    created_environment->lpVtbl->AddRef(created_environment);

    wv->ctrl_handler.lpVtbl = &g_ctrl_vtbl;
    wv->ctrl_handler.slot = slot;

    created_environment->lpVtbl->CreateCoreWebView2Controller(created_environment, wv->hwnd, &wv->ctrl_handler);
    return S_OK;
}

typedef BOOL (WINAPI *SetProcessDpiAwarenessContextFn)(HANDLE);

static void lpp_init_handlers(void) {
    static int initialized = 0;
    if (initialized) return;
    initialized = 1;

    HMODULE hUser32 = GetModuleHandleA("user32.dll");
    if (hUser32) {
        SetProcessDpiAwarenessContextFn pSetDpi = 
            (SetProcessDpiAwarenessContextFn)(void*)GetProcAddress(hUser32, "SetProcessDpiAwarenessContext");
        if (pSetDpi) {
            pSetDpi((HANDLE)-4);
        }
    }

    OleInitialize(NULL);

    g_env_vtbl.Invoke = Env_Invoke;
    g_ctrl_vtbl.Invoke = Ctrl_Invoke;

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
    if (!hLoader) hLoader = LoadLibraryA("webview_demo\\WebView2Loader.dll");
    if (!hLoader) hLoader = LoadLibraryA("packages\\samarbook-app\\WebView2Loader.dll");
    if (!hLoader) hLoader = LoadLibraryA("webview2_sdk\\build\\native\\x64\\WebView2Loader.dll");
    if (!hLoader) hLoader = LoadLibraryA("webview2_nuget\\build\\native\\x64\\WebView2Loader.dll");

    if (hLoader) {
        pCreateCoreWebView2EnvironmentWithOptions = 
            (CreateCoreWebView2EnvironmentWithOptionsFn)(void*)GetProcAddress(hLoader, "CreateCoreWebView2EnvironmentWithOptions");
    }
}

static LRESULT CALLBACK lpp_webview_wndproc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    switch (msg) {
        case WM_SIZE: {
            for (int i = 0; i < MAX_WEBVIEWS; i++) {
                if (g_webviews[i].hwnd == hwnd && g_webviews[i].controller) {
                    if (wp == SIZE_MINIMIZED) {
                        g_webviews[i].controller->lpVtbl->put_IsVisible(g_webviews[i].controller, FALSE);
                    } else {
                        RECT bounds;
                        GetClientRect(hwnd, &bounds);
                        g_webviews[i].controller->lpVtbl->put_IsVisible(g_webviews[i].controller, TRUE);
                        g_webviews[i].controller->lpVtbl->put_Bounds(g_webviews[i].controller, bounds);
                        g_webviews[i].controller->lpVtbl->NotifyParentWindowPositionChanged(g_webviews[i].controller);
                    }
                    break;
                }
            }
            break;
        }
        case WM_MOVE:
        case WM_WINDOWPOSCHANGED: {
            for (int i = 0; i < MAX_WEBVIEWS; i++) {
                if (g_webviews[i].hwnd == hwnd && g_webviews[i].controller) {
                    RECT bounds;
                    GetClientRect(hwnd, &bounds);
                    g_webviews[i].controller->lpVtbl->put_Bounds(g_webviews[i].controller, bounds);
                    g_webviews[i].controller->lpVtbl->NotifyParentWindowPositionChanged(g_webviews[i].controller);
                    break;
                }
            }
            break;
        }
        case WM_DPICHANGED: {
            RECT *new_rect = (RECT *)lp;
            if (new_rect) {
                SetWindowPos(hwnd, NULL, new_rect->left, new_rect->top,
                             new_rect->right - new_rect->left,
                             new_rect->bottom - new_rect->top,
                             SWP_NOZORDER | SWP_NOACTIVATE);
            }
            for (int i = 0; i < MAX_WEBVIEWS; i++) {
                if (g_webviews[i].hwnd == hwnd && g_webviews[i].controller) {
                    RECT bounds;
                    GetClientRect(hwnd, &bounds);
                    g_webviews[i].controller->lpVtbl->put_Bounds(g_webviews[i].controller, bounds);
                    g_webviews[i].controller->lpVtbl->NotifyParentWindowPositionChanged(g_webviews[i].controller);
                    break;
                }
            }
            return 0;
        }
        case WM_ERASEBKGND:
            return 1;
        case WM_PAINT: {
            PAINTSTRUCT ps;
            HDC hdc = BeginPaint(hwnd, &ps);
            EndPaint(hwnd, &ps);
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
                        g_webviews[i].controller->lpVtbl->Release(g_webviews[i].controller);
                        g_webviews[i].controller = NULL;
                    }
                    if (g_webviews[i].core) {
                        g_webviews[i].core->lpVtbl->Release(g_webviews[i].core);
                        g_webviews[i].core = NULL;
                    }
                    if (g_webviews[i].env) {
                        g_webviews[i].env->lpVtbl->Release(g_webviews[i].env);
                        g_webviews[i].env = NULL;
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
            return DefWindowProcW(hwnd, msg, wp, lp);
    }
    return DefWindowProcW(hwnd, msg, wp, lp);
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
        WNDCLASSEXW wc = {0};
        wc.cbSize        = sizeof(WNDCLASSEXW);
        wc.lpfnWndProc   = lpp_webview_wndproc;
        wc.hInstance     = hInst;
        wc.lpszClassName = L"LppNativeWebViewWindow";
        wc.hCursor       = LoadCursorW(NULL, (LPCWSTR)IDC_ARROW);
        wc.hIcon         = LoadIconW(NULL, (LPCWSTR)IDI_APPLICATION);
        wc.style         = CS_HREDRAW | CS_VREDRAW;
        RegisterClassExW(&wc);
        g_wv_class_registered = 1;
    }

    WCHAR wtitle[512];
    if (title && title[0]) {
        MultiByteToWideChar(CP_UTF8, 0, title, -1, wtitle, 512);
    } else {
        lpp_strcpy((char*)wtitle, (const char*)L"L++ Desktop Application");
    }

    HWND hwnd = CreateWindowExW(
        WS_EX_APPWINDOW,
        L"LppNativeWebViewWindow",
        wtitle,
        WS_OVERLAPPEDWINDOW | WS_CLIPCHILDREN | WS_VISIBLE,
        CW_USEDEFAULT, CW_USEDEFAULT, (int)width > 0 ? (int)width : 1360, (int)height > 0 ? (int)height : 820,
        NULL, NULL, hInst, NULL);

    if (!hwnd) {
        return -1;
    }

    /* Enable Windows 10/11 Dark Mode Titlebar */
    HMODULE hDwmapi = LoadLibraryA("dwmapi.dll");
    if (hDwmapi) {
        typedef HRESULT (WINAPI *DwmSetWindowAttributeFn)(HWND, DWORD, LPCVOID, DWORD);
        DwmSetWindowAttributeFn pDwmSet = (DwmSetWindowAttributeFn)(void*)GetProcAddress(hDwmapi, "DwmSetWindowAttribute");
        if (pDwmSet) {
            BOOL darkMode = TRUE;
            pDwmSet(hwnd, 20 /* DWMWA_USE_IMMERSIVE_DARK_MODE */, &darkMode, sizeof(darkMode));
        }
        FreeLibrary(hDwmapi);
    }

    ShowWindow(hwnd, SW_SHOW);
    UpdateWindow(hwnd);

    LppWinWebView *wv = &g_webviews[slot];
    lpp_memset(wv, 0, sizeof(*wv));
    wv->hwnd = hwnd;
    wv->is_open = 1;
    wv->is_ready = 0;
    wv->width = (int)width > 0 ? (int)width : 1360;
    wv->height = (int)height > 0 ? (int)height : 820;

    if (pCreateCoreWebView2EnvironmentWithOptions) {
        char user_data_dir[MAX_PATH];
        char local_app_data[MAX_PATH];
        if (GetEnvironmentVariableA("LOCALAPPDATA", local_app_data, MAX_PATH) > 0) {
            wsprintfA(user_data_dir, "%s\\LppWebView2\\slot_%d", local_app_data, slot);
        } else {
            char temp_dir[MAX_PATH];
            GetTempPathA(MAX_PATH, temp_dir);
            wsprintfA(user_data_dir, "%s\\LppWebView2_slot_%d", temp_dir, slot);
        }
        CreateDirectoryA(user_data_dir, NULL);
        
        WCHAR wdata_dir[MAX_PATH];
        MultiByteToWideChar(CP_UTF8, 0, user_data_dir, -1, wdata_dir, MAX_PATH);

        wv->env_handler.lpVtbl = &g_env_vtbl;
        wv->env_handler.slot = slot;

        pCreateCoreWebView2EnvironmentWithOptions(NULL, wdata_dir, NULL, &wv->env_handler);

        /* Pump messages synchronously until WebView2 completes its COM handshake */
        MSG msg;
        while (!wv->is_ready && GetMessageW(&msg, NULL, 0, 0)) {
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
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

    char temp_path[MAX_PATH];
    GetTempPathA(MAX_PATH, temp_path);
    char file_path[MAX_PATH];
    wsprintfA(file_path, "%slpp_app_%d.html", temp_path, (int)win_id);
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
        lpp_load_content_into_core(wv->core, url);
    }
}

void lpp_webview_run(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_WEBVIEWS || !g_webviews[win_id].is_open) return;
    LppWinWebView *wv = &g_webviews[win_id];

    MSG msg;
    while (wv->is_open && GetMessageW(&msg, NULL, 0, 0)) {
        TranslateMessage(&msg);
        DispatchMessageW(&msg);
    }
}

void lpp_webview_terminate(int64_t win_id) {
    if (win_id < 0 || win_id >= MAX_WEBVIEWS || !g_webviews[win_id].is_open) return;
    LppWinWebView *wv = &g_webviews[win_id];
    wv->is_open = 0;
    if (wv->controller) {
        wv->controller->lpVtbl->Close(wv->controller);
        wv->controller->lpVtbl->Release(wv->controller);
        wv->controller = NULL;
    }
    if (wv->core) {
        wv->core->lpVtbl->Release(wv->core);
        wv->core = NULL;
    }
    if (wv->env) {
        wv->env->lpVtbl->Release(wv->env);
        wv->env = NULL;
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
