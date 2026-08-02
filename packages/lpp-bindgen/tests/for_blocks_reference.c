#include <stdio.h>
#include "../fixtures/for_blocks.c"

int main(void) {
    printf("%d\n", sum_odd_steps(9));
    printf("%d\n", count_down(5));
    printf("%d\n", sum_adjusted(6));
    return 0;
}
