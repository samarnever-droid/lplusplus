

/* ═══════════════════════════════════════════════════════════════════════════
 *  Raylib Standard Compatibility Layer (rl_ prefix)
 * ═══════════════════════════════════════════════════════════════════════════ */

static int64_t g_active_raylib_win = 0;
static int64_t g_target_fps = 60;
static uint8_t g_prev_key_state[256] = {0};

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
    (void)filename;
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

