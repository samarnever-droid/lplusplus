#include <stdio.h>
int f1(int a){ int x=1,y=2; return x+y; }
int f2(int a){ int x,y; x=a; y=a+1; return x+y; }
int f3(int a){ int x=a,y=a*2; return x+y; }
int f4(int a){ int x=1,y=2,z=3; return x+y+z; }
int f5(int *p,int n){ int *q=p,*r=p+1; return *q+*r; }
int main(void){
    int arr[4]={10,20,30,40};
    printf("%d %d %d %d %d\n", f1(0),f2(5),f3(5),f4(0),f5(arr,4));
    return 0;
}
