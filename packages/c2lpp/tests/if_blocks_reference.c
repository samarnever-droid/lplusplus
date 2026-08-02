#include <stdio.h>
#include "../fixtures/if_blocks.c"

int main(void) {
    printf("%d\n", adjust_value(-5));
    printf("%d\n", adjust_value(3));
    return 0;
}
