#include <stdio.h>
int f1(int n){ int i=0; for(i=0;i<n;){ i=i+2; } return i; }
int f2(int n){ int i=0; while(1){ i=i+1; if(i>=n) break; } return i; }
int f3(int n){ int i; int acc=0; for(i=n;i>0;i=i-1){ acc=acc+i; } return acc; }
int main(void){ printf("%d %d %d\n", f1(5), f2(5), f3(5)); return 0; }
