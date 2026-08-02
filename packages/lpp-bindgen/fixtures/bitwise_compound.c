static int f1(int a){ int x = a; x <<= 2; x |= 1; return x; }
static int f6(int a){ int x = a; x >>= 1; x &= 3; x ^= 5; return x; }
