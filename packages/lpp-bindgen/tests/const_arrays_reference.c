#include <stdio.h>

#include "../fixtures/const_arrays.c"

int main(void) {
    printf("%d\n", weight_at(0));
    printf("%d\n", weight_at(3));
    printf("%d\n", weight_at(4));
    printf("%d\n", inferred_at(2));
    printf("%d\n", weighted_sum(5, 3));
    return 0;
}
