
int64_t lpp_command_exec(int64_t cmd_ptr) {
    if (!cmd_ptr) return -1;
    const char *cmd = (const char *)(intptr_t)cmd_ptr;
    int res = system(cmd);
    return (int64_t)res;
}

int64_t lpp_command_output(int64_t cmd_ptr) {
    if (!cmd_ptr) {
        char *empty = (char *)lpp_arc_alloc(1);
        empty[0] = '\0';
        return (int64_t)(intptr_t)empty;
    }
    const char *cmd = (const char *)(intptr_t)cmd_ptr;
    
#if defined(_WIN32)
    FILE *fp = _popen(cmd, "r");
#else
    FILE *fp = popen(cmd, "r");
#endif

    if (!fp) {
        char *empty = (char *)lpp_arc_alloc(1);
        empty[0] = '\0';
        return (int64_t)(intptr_t)empty;
    }
    
    size_t cap = 1024;
    size_t len = 0;
    char *buf = (char *)lpp_arc_alloc((int64_t)cap);
    if (!buf) {
#if defined(_WIN32)
        _pclose(fp);
#else
        pclose(fp);
#endif
        return 0;
    }
    
    while (1) {
        if (len + 256 > cap) {
            cap *= 2;
            char *new_buf = (char *)lpp_arc_alloc((int64_t)cap);
            if (!new_buf) break;
            memcpy(new_buf, buf, len);
            lpp_arc_release(buf);
            buf = new_buf;
        }
        size_t n = fread(buf + len, 1, 256, fp);
        if (n == 0) break;
        len += n;
    }
    buf[len] = '\0';
    
#if defined(_WIN32)
    _pclose(fp);
#else
    pclose(fp);
#endif
    return (int64_t)(intptr_t)buf;
}
