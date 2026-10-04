#include <stdio.h>

#include "../fixtures/ternary_returns.c"

int main(void) {
    int value = 37;
    printf("%d\n", choose_int(1, 5, 9));
    printf("%d\n", choose_int(0, 5, 9));
    printf("%d\n", choose_comparison(3, 7));
    printf("%d\n", choose_comparison(8, 9));
    printf("%.6f\n", choose_float(1, 2.5, 8.75));
    printf("%.6f\n", choose_float(0, 2.5, 8.75));
    printf("%d\n", choose_pointer(1, &value) == &value);
    printf("%d\n", choose_pointer(0, &value) == 0);
    printf("%d\n", load_or(&value, 11));
    printf("%d\n", load_or(0, 11));
    printf("%d\n", both_nonnull(&value, &value));
    printf("%d\n", both_nonnull(&value, 0));
    printf("%d\n", guarded_value(&value));
    printf("%d\n", guarded_value(0));
    return 0;
}
