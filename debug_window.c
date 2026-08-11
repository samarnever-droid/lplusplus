#include <windows.h>
#include <stdio.h>

LRESULT CALLBACK WndProc(HWND hwnd, UINT msg, WPARAM wp, LPARAM lp) {
    if (msg == WM_DESTROY) { PostQuitMessage(0); return 0; }
    return DefWindowProcA(hwnd, msg, wp, lp);
}

int main() {
    printf("Starting Win32 window test...\n");
    fflush(stdout);

    HINSTANCE hInst = GetModuleHandleA(NULL);
    printf("hInst = %p\n", (void*)hInst);
    fflush(stdout);

    WNDCLASSA wc = {0};
    wc.lpfnWndProc = WndProc;
    wc.hInstance = hInst;
    wc.lpszClassName = "TestWindowClass";
    wc.hbrBackground = (HBRUSH)(COLOR_WINDOW + 1);
    wc.hCursor = LoadCursorA(NULL, IDC_ARROW);
    ATOM atom = RegisterClassA(&wc);
    printf("RegisterClassA atom = %d\n", atom);
    fflush(stdout);

    HWND hwnd = CreateWindowExA(
        0, "TestWindowClass", "L++ GUI Test Window",
        WS_OVERLAPPEDWINDOW,
        200, 200, 800, 600,
        NULL, NULL, hInst, NULL
    );
    printf("hwnd = %p\n", (void*)hwnd);
    printf("GetLastError = %lu\n", GetLastError());
    fflush(stdout);

    if (!hwnd) {
        printf("FAILED to create window!\n");
        fflush(stdout);
        return 1;
    }

    ShowWindow(hwnd, SW_SHOW);
    UpdateWindow(hwnd);
    printf("Window shown. Running message loop...\n");
    fflush(stdout);

    MSG msg;
    while (GetMessageA(&msg, NULL, 0, 0)) {
        TranslateMessage(&msg);
        DispatchMessageA(&msg);
    }
    printf("Window closed.\n");
    return 0;
}
