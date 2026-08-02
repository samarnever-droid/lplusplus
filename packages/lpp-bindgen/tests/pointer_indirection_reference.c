#include <stdio.h>

#include "../fixtures/pointer_indirection.c"

int main(void) {
    int first = 11;
    int second = 23;
    int third = 37;
    int *values[2] = {&first, &second};
    int plain[4] = {3, 5, 7, 9};
    printf("%d\n", read_indirect(&values[0]));
    printf("%d\n", read_pointer_array(values, 1));
    write_pointer_array(values, 0, &third);
    printf("%d\n", read_indirect(&values[0]));
    write_indirect(&values[1], &first);
    printf("%d\n", read_pointer_array(values, 1));
    printf("%d\n", read_array_parameter(plain, 2));
    printf("%d\n", read_fixed_parameter(plain, 3));
    return 0;
}
