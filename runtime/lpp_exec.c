/*
 * lpp_exec.c  —  L++ process/command execution builtins (cross-platform)
 *
 * Provides: command_exec, command_output, env_get, env_set
 *
 * Linux/macOS: posix_spawn + pipe   Windows: CreateProcess + pipe
 *
 * Build: cc -O2 -c runtime/lpp_exec.c -o lpp_exec.o
 *        cl /nologo /O2 /c runtime/lpp_exec.c /Fo:lpp_exec.obj
 */

#include <stdlib.h>
#include <string.h>
#include <stdint.h>
#include <stdio.h>

/* ── ARC helpers (defined in lpp_runtime.c) ───────────────────────────── */
extern void *lpp_arc_alloc(int64_t size);
extern void  lpp_arc_release(void *ptr);

#if defined(_WIN32)
/* ── Windows implementation ───────────────────────────────────────────── */
#include <windows.h>

int64_t lpp_command_exec(const char *cmdline) {
    if (!cmdline) return -1;
    STARTUPINFOA si = {sizeof(si)};
    PROCESS_INFORMATION pi = {0};
    si.dwFlags = STARTF_USESTDHANDLES;
    char *dup = _strdup(cmdline);
    if (!dup) return -1;
    BOOL ok = CreateProcessA(NULL, dup, NULL, NULL, FALSE,
                              CREATE_NO_WINDOW, NULL, NULL, &si, &pi);
    free(dup);
    if (!ok) return -1;
    WaitForSingleObject(pi.hProcess, INFINITE);
    DWORD code;
    GetExitCodeProcess(pi.hProcess, &code);
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);
    return (int64_t)(int)code;
}

char *lpp_command_output(const char *cmdline) {
    if (!cmdline) return (char *)"";
    HANDLE hRead, hWrite;
    SECURITY_ATTRIBUTES sa = {sizeof(sa), NULL, TRUE};
    if (!CreatePipe(&hRead, &hWrite, &sa, 0)) return (char *)"";

    STARTUPINFOA si = {sizeof(si)};
    PROCESS_INFORMATION pi = {0};
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdOutput = hWrite;
    si.hStdError  = hWrite;

    char *dup = _strdup(cmdline);
    BOOL ok = CreateProcessA(NULL, dup, NULL, NULL, TRUE,
                              CREATE_NO_WINDOW, NULL, NULL, &si, &pi);
    free(dup);
    CloseHandle(hWrite);
    if (!ok) { CloseHandle(hRead); return (char *)""; }

    WaitForSingleObject(pi.hProcess, INFINITE);
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);

    int cap = 4096, len = 0;
    char *buf = (char *)lpp_arc_alloc((int64_t)(cap + 1));
    if (!buf) { CloseHandle(hRead); return (char *)""; }
    for (;;) {
        if (len + 1024 >= cap) {
            int nc = cap * 2;
            char *nb = (char *)lpp_arc_alloc((int64_t)(nc + 1));
            if (!nb) break;
            memcpy(nb, buf, (size_t)len);
            lpp_arc_release(buf);
            buf = nb; cap = nc;
        }
        DWORD n;
        if (!ReadFile(hRead, buf + len, (DWORD)(cap - len), &n, NULL) || n == 0) break;
        len += (int)n;
    }
    CloseHandle(hRead);
    buf[len] = 0;
    return buf;
}

char *lpp_env_get(const char *name) {
    if (!name) return (char *)"";
    char val[4096];
    DWORD n = GetEnvironmentVariableA(name, val, sizeof(val));
    if (n == 0 || n >= sizeof(val)) return (char *)"";
    char *out = (char *)lpp_arc_alloc((int64_t)(n + 1));
    if (!out) return (char *)"";
    memcpy(out, val, n);
    out[n] = 0;
    return out;
}

int64_t lpp_env_set(const char *name, const char *value) {
    if (!name) return -1;
    return SetEnvironmentVariableA(name, value ? value : "") ? 0 : -1;
}

#else
/* ── Unix (Linux / macOS) implementation ───────────────────────────────── */
#include <sys/wait.h>
#include <unistd.h>
#include <spawn.h>
#include <signal.h>

extern char **environ;

int64_t lpp_command_exec(const char *cmdline) {
    if (!cmdline) return -1;
    pid_t pid;
    char *sh = "/bin/sh";
    char *argv[] = {sh, (char *)"-c", (char *)cmdline, NULL};
    int status = posix_spawn(&pid, sh, NULL, NULL, argv, environ);
    if (status != 0) return -1;
    waitpid(pid, &status, 0);
    return WIFEXITED(status) ? (int64_t)WEXITSTATUS(status) : -1;
}

char *lpp_command_output(const char *cmdline) {
    if (!cmdline) return (char *)"";
    int pipefd[2];
    if (pipe(pipefd) < 0) return (char *)"";

    pid_t pid = fork();
    if (pid < 0) { close(pipefd[0]); close(pipefd[1]); return (char *)""; }

    if (pid == 0) {
        /* child */
        close(pipefd[0]);
        dup2(pipefd[1], STDOUT_FILENO);
        dup2(pipefd[1], STDERR_FILENO);
        close(pipefd[1]);
        execl("/bin/sh", "sh", "-c", cmdline, (char *)NULL);
        _exit(127);
    }

    close(pipefd[1]);
    int cap = 4096, len = 0;
    char *buf = (char *)lpp_arc_alloc((int64_t)(cap + 1));
    if (!buf) { close(pipefd[0]); waitpid(pid, NULL, 0); return (char *)""; }

    for (;;) {
        if (len + 1024 >= cap) {
            int nc = cap * 2;
            char *nb = (char *)lpp_arc_alloc((int64_t)(nc + 1));
            if (!nb) break;
            memcpy(nb, buf, (size_t)len);
            lpp_arc_release(buf);
            buf = nb; cap = nc;
        }
        ssize_t n = read(pipefd[0], buf + len, (size_t)(cap - len));
        if (n <= 0) break;
        len += (int)n;
    }
    close(pipefd[0]);
    waitpid(pid, NULL, 0);
    buf[len] = 0;
    return buf;
}

char *lpp_env_get(const char *name) {
    if (!name) return (char *)"";
    const char *val = getenv(name);
    if (!val) return (char *)"";
    int64_t len = (int64_t)strlen(val);
    char *out = (char *)lpp_arc_alloc(len + 1);
    if (!out) return (char *)"";
    memcpy(out, val, (size_t)len);
    out[len] = 0;
    return out;
}

int64_t lpp_env_set(const char *name, const char *value) {
    if (!name) return -1;
    return setenv(name, value ? value : "", 1) == 0 ? 0 : -1;
}

#endif
