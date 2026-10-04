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
extern char *lpp_empty_str(void);
extern void  lpp_arc_release(void *ptr);

#if defined(_WIN32)
/* ── Windows implementation ───────────────────────────────────────────── */
#include <windows.h>

#ifndef LPP_EXEC_EXCLUDE_BUILTINS
int64_t lpp_command_exec(const char *cmdline) {
    if (!cmdline) return -1;
    return (int64_t)system(cmdline);
}

char *lpp_command_output(const char *cmdline) {
    if (!cmdline) return lpp_empty_str();
    HANDLE hRead, hWrite;
    SECURITY_ATTRIBUTES sa = {sizeof(sa), NULL, TRUE};
    if (!CreatePipe(&hRead, &hWrite, &sa, 0)) return lpp_empty_str();

    STARTUPINFOA si = {sizeof(si)};
    PROCESS_INFORMATION pi = {0};
    si.dwFlags = STARTF_USESTDHANDLES;
    si.hStdOutput = hWrite;
    si.hStdError  = hWrite;

    char *dup = malloc(strlen(cmdline) + 1); if (dup) strcpy(dup, cmdline);
    BOOL ok = CreateProcessA(NULL, dup, NULL, NULL, TRUE,
                              CREATE_NO_WINDOW, NULL, NULL, &si, &pi);
    free(dup);
    CloseHandle(hWrite);
    if (!ok) { CloseHandle(hRead); return lpp_empty_str(); }

    WaitForSingleObject(pi.hProcess, INFINITE);
    CloseHandle(pi.hProcess);
    CloseHandle(pi.hThread);

    int cap = 4096, len = 0;
    char *buf = (char *)lpp_arc_alloc((int64_t)(cap + 1));
    if (!buf) { CloseHandle(hRead); return lpp_empty_str(); }
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
#endif

char *lpp_env_get(const char *name) {
    if (!name) return lpp_empty_str();
    char val[4096];
    DWORD n = GetEnvironmentVariableA(name, val, sizeof(val));
    if (n == 0 || n >= sizeof(val)) return lpp_empty_str();
    char *out = (char *)lpp_arc_alloc((int64_t)(n + 1));
    if (!out) return lpp_empty_str();
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

#ifndef LPP_EXEC_EXCLUDE_BUILTINS
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
    if (!cmdline) return lpp_empty_str();
    int pipefd[2];
    if (pipe(pipefd) < 0) return lpp_empty_str();

    pid_t pid = fork();
    if (pid < 0) { close(pipefd[0]); close(pipefd[1]); return lpp_empty_str(); }

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
    if (!buf) { close(pipefd[0]); waitpid(pid, NULL, 0); return lpp_empty_str(); }

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
#endif

char *lpp_env_get(const char *name) {
    if (!name) return lpp_empty_str();
    const char *val = getenv(name);
    if (!val) return lpp_empty_str();
    int64_t len = (int64_t)strlen(val);
    char *out = (char *)lpp_arc_alloc(len + 1);
    if (!out) return lpp_empty_str();
    memcpy(out, val, (size_t)len);
    out[len] = 0;
    return out;
}

int64_t lpp_env_set(const char *name, const char *value) {
    if (!name) return -1;
    return setenv(name, value ? value : "", 1) == 0 ? 0 : -1;
}

#endif
