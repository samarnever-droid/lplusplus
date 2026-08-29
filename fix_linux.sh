sed -i '/int64_t lpp_webview_window_create(const char \*title, int64_t width, int64_t height, int64_t debug) {/,+2d' runtime/linux_x86_64_min.c
sed -i '/int64_t lpp_webview_navigate(int64_t w, const char \*url) {/,+2d' runtime/linux_x86_64_min.c
