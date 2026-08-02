#include <stdio.h>
int f(int *arr, int i){ return arr[i]++; }
int g(int *arr){ return (*arr)++; }
int main(void){
    int a[3] = {10, 20, 30};
    int r1 = f(a,1);
    int n1 = a[1];
    int r2 = g(a);
    int n2 = a[0];
    printf("%d %d %d %d\n", r1, n1, r2, n2);
    return 0;
}
