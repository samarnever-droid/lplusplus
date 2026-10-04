#include <stdio.h>
#include "../fixtures/casts.c"

int main(void) {
    printf("%d\n", cast_down(7.75));
    printf("%.6f\n", cast_up(9));
    printf("%d\n", cast_truth(-3));
    return 0;
}
