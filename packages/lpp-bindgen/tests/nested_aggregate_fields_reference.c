#include <stdio.h>
#include <stdlib.h>
typedef struct { int a; int b; } Pair;
typedef struct { Pair *p; int n; } Box;
int f(Box *b){ if (b->p) return b->p->a; return -1; }
int f1(Box *b){ if (b->n>0) return 1; return 0; }
int main(void){
    Box *b = calloc(1, sizeof(Box));
    Pair pr; pr.a = 42; pr.b = 7;
    b->p = &pr; b->n = 5;
    printf("%d %d\n", f(b), f1(b));
    free(b);
    return 0;
}
