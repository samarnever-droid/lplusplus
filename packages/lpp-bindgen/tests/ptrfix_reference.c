#include <stdio.h>
int f(int n, int *base){ int *p; p = base; return p[n]; }
int g(int n, int *base){ int *p = 0; if (n>0) p = base; return p ? p[0] : -1; }
int main(void){
    int arr[4] = {10, 20, 30, 40};
    printf("%d %d %d %d\n", f(0,arr), f(2,arr), g(1,arr), g(0,arr));
    return 0;
}
