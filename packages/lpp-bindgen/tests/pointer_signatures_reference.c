#include <stdio.h>
#include "../fixtures/pointer_signatures.c"

int main(void) {
    int storage = 1;
    printf("%d\n", pointer_nonnull((Opaque *)&storage));
    printf("%d\n", pointer_null() == 0);
    return 0;
}
