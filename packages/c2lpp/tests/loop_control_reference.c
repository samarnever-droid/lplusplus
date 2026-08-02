#include <stdio.h>
#include "../fixtures/loop_control.c"

int main(void) {
    printf("%d\n", for_control(10));
    printf("%d\n", while_control(12));
    return 0;
}
