#include <stdio.h>
int clamp_value(int n, int lo){ if (n<lo) n=lo; return n; }
int adjust(int n){ if (n<0) n+=10; return n; }
int pick(int n){ if (n==0) n=n<0?1:2; else n=7; return n; }
int bump(int *p){ if (*p>0) *p+=3; return *p; }
int main(void){
    int x = 25;
    printf("%d %d %d %d %d %d %d\n",
        clamp_value(3,10), clamp_value(20,10),
        adjust(-5), adjust(4),
        pick(0), pick(1), bump(&x));
    return 0;
}
