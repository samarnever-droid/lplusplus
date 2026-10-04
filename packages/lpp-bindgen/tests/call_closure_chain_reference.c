#include <stdio.h>
int c3(int x){ return x+1; }
int c2(int x){ return c3(x)+1; }
int c1(int x){ return c2(x)+1; }
int entry(int x){ return c1(x); }
int main(void){ printf("%d\n", entry(5)); return 0; }
