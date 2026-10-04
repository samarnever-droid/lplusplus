static int c3(int x){ return x + 1; }
static int c2(int x){ return c3(x) + 1; }
static int c1(int x){ return c2(x) + 1; }
static int entry(int x){ return c1(x); }
