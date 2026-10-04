static int f1(int a){ int x = 1, y = 2; return x + y; }
static int f2(int a){ int x, y; x = a; y = a + 1; return x + y; }
static int f3(int a){ int x = a, y = a * 2; return x + y; }
static int f4(int a){ int x = 1, y = 2, z = 3; return x + y + z; }
static int f5(int *p, int n){ int *q = p, *r = p + 1; return *q + *r; }
