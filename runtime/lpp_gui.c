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

#if defined(_WIN32)
#include "lpp_gui/win32.inc"
#else
#include "lpp_gui/x11.inc"
#endif

#include "lpp_gui/raylib_compat.inc"
