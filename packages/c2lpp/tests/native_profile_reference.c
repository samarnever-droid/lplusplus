#include <stdio.h>
#include "../fixtures/native_profile.c"

int main(void) {
    printf("%d\n", process(4));
    printf("%d\n", process(3));
    printf("%d\n", process(1));
    return 0;
}
