#include <stdio.h>
int f1(int a){ int x=a; x<<=2; x|=1; return x; }
int f6(int a){ int x=a; x>>=1; x&=3; x^=5; return x; }
int main(void){ printf("%d %d %d %d\n", f1(3), f1(5), f6(29), f6(8)); return 0; }
