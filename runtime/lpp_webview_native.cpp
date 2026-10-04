#define WEBVIEW_STATIC
#define WEBVIEW_BUILD
#define WEBVIEW_EDGE
#define WIN32_LEAN_AND_MEAN

#include <windows.h>
#include <string>
#include <vector>
#include <cstdint>
#include "webview.h"

extern "C" {

struct LppWebviewHandle {
    webview_t w;
};

static std::vector<webview_t> g_webviews;

int64_t lpp_webview_window_create(const char *title, int64_t width, int64_t height, int64_t debug) {
    webview_t w = webview_create(debug != 0, nullptr);
    if (!w) {
        return -1;
    }
    if (title && title[0]) {
        webview_set_title(w, title);
    } else {
        webview_set_title(w, "L++ Desktop Application");
    }
    
    int w_val = (int)(width > 0 ? width : 1280);
    int h_val = (int)(height > 0 ? height : 800);
    webview_set_size(w, w_val, h_val, WEBVIEW_HINT_NONE);

    g_webviews.push_back(w);
    return (int64_t)(g_webviews.size() - 1);
}

void lpp_webview_navigate(int64_t handle, const char *url) {
    if (handle < 0 || (size_t)handle >= g_webviews.size() || !g_webviews[handle] || !url) return;
    
    // Check if url is a local file relative path
    if (strstr(url, "://") == nullptr && strstr(url, "data:") == nullptr && strstr(url, "about:") == nullptr) {
        char full_path[MAX_PATH];
        if (GetFileAttributesA(url) != INVALID_FILE_ATTRIBUTES) {
            GetFullPathNameA(url, MAX_PATH, full_path, NULL);
        } else {
            char exe_dir[MAX_PATH];
            GetModuleFileNameA(NULL, exe_dir, MAX_PATH);
            char *slash = strrchr(exe_dir, '\\');
            if (slash) *slash = '\0';
            char candidate[MAX_PATH];
            wsprintfA(candidate, "%s\\%s", exe_dir, url);
            if (GetFileAttributesA(candidate) != INVALID_FILE_ATTRIBUTES) {
                GetFullPathNameA(candidate, MAX_PATH, full_path, NULL);
            } else {
                GetFullPathNameA(url, MAX_PATH, full_path, NULL);
            }
        }
        for (char *p = full_path; *p; p++) {
            if (*p == '\\') *p = '/';
        }
        char file_url[1024];
        wsprintfA(file_url, "file:///%s", full_path);
        webview_navigate(g_webviews[handle], file_url);
    } else {
        webview_navigate(g_webviews[handle], url);
    }
}

void lpp_webview_set_html(int64_t handle, const char *html) {
    if (handle < 0 || (size_t)handle >= g_webviews.size() || !g_webviews[handle] || !html) return;
    webview_set_html(g_webviews[handle], html);
}

void lpp_webview_run(int64_t handle) {
    if (handle < 0 || (size_t)handle >= g_webviews.size() || !g_webviews[handle]) return;
    webview_run(g_webviews[handle]);
}

void lpp_webview_terminate(int64_t handle) {
    if (handle < 0 || (size_t)handle >= g_webviews.size() || !g_webviews[handle]) return;
    webview_terminate(g_webviews[handle]);
}

void lpp_webview_destroy(int64_t handle) {
    if (handle < 0 || (size_t)handle >= g_webviews.size() || !g_webviews[handle]) return;
    webview_destroy(g_webviews[handle]);
    g_webviews[handle] = nullptr;
}

}
