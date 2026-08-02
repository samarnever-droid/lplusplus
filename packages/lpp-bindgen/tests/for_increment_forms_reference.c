#include <stdio.h>
int f(int n){ int i,acc=0; for(i=0;i<n;i=i+1){ acc=acc+i; } return acc; }
int g(int n){ int i,acc=0; for(i=n;i>0;i--){ acc=acc+i; } return acc; }
int h(int n){ int i,acc=0; for(i=0;i<n;i+=2){ acc=acc+i; } return acc; }
int main(void){ printf("%d %d %d\n", f(5), g(5), h(5)); return 0; }
