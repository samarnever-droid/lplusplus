#include <stdio.h>
#include "../fixtures/while_blocks.c"

int main(void) {
    printf("%d\n", sum_while(10));
    printf("%d\n", normalize_nonzero(7));
    return 0;
}
