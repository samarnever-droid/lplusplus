int clamp_value(int n, int lo) {
    if (n < lo) n = lo;
    return n;
}

int adjust(int n) {
    if (n < 0) n += 10;
    return n;
}

int pick(int n) {
    if (n == 0) n = n < 0 ? 1 : 2;
    else n = 7;
    return n;
}

int bump(int *p) {
    if (*p > 0) *p += 3;
    return *p;
}
