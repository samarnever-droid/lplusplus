#include <stdio.h>

#include "../fixtures/unbraced_calls.c"

int main(void) {
    int value = 5;
    maybe_set(0, &value, 11);
    printf("%d\n", value);
    maybe_set(1, &value, 13);
    printf("%d\n", value);
    either_set(1, &value, 17, 19);
    printf("%d\n", value);
    either_set(0, &value, 17, 19);
    printf("%d\n", value);
    printf("%d\n", read_after_set(1, &value));
    return 0;
}
