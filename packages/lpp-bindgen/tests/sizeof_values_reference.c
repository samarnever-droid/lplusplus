#include <stdio.h>

#include "../fixtures/sizeof_values.c"

int main(void) {
    Pair pair = {1, 2};
    printf("%d\n", size_of_char());
    printf("%d\n", size_of_int());
    printf("%d\n", size_of_pointer());
    printf("%d\n", size_of_pair());
    printf("%d\n", size_of_dereference(&pair));
    return 0;
}
