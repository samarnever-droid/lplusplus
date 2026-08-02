#include <stdio.h>
#include "../fixtures/scalar_algorithms.c"

int main(void) {
    printf("%d\n", sum_to(10));
    printf("%d\n", sum_for(10));
    printf("%d\n", absolute_value(-7));
    printf("%d\n", clamp(50, 0, 10));
    printf("%d\n", array_score());
    return 0;
}
