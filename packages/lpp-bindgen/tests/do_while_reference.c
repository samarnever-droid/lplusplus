#include <stdio.h>

#include "../fixtures/do_while.c"

int main(void) {
    printf("%d\n", sum_do(5));
    printf("%d\n", sum_do(0));
    printf("%d\n", break_do(4));
    printf("%d\n", break_do(20));
    printf("%d\n", once_do(27));
    return 0;
}
