#include <stdio.h>
#include "../fixtures/typed_scalar.c"

int main(void) {
    printf("%d\n", add_scaled(7, 3));
    printf("%d\n", bit_mix(9));
    printf("%d\n", call_chain(5));
    return 0;
}
