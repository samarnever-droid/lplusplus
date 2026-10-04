#include <stdio.h>

#include "../fixtures/pointer_places.c"

int main(void) {
    int values[4] = {7, 11, 13, 17};
    printf("%d\n", read_first(values));
    printf("%d\n", read_at(values, 2));
    printf("%d\n", sum_edges(values, 3));
    write_at(values, 1, 23);
    add_at(values, 2, 5);
    increment_first(values);
    printf("%d\n", values[0]);
    printf("%d\n", values[1]);
    printf("%d\n", values[2]);
    printf("%ld\n", (long)(address_at(values, 3) - values));
    printf("%d\n", local_shift(values));
    printf("%d\n", pointer_distance(values, 1, 3));
    printf("%d\n", pointer_same(values, 2));
    return 0;
}
