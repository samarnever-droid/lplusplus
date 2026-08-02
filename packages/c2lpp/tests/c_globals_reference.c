#include <stdio.h>

static int base = 7;
static int total;
static int counter;

static void initialize_translation_unit(void) {
    total = base * 3;
}

static int add_to_total(int value) {
    total += value;
    counter += 1;
    return total;
}

int main(void) {
    printf("%d\n", counter);
    initialize_translation_unit();
    printf("%d\n", base);
    printf("%d\n", total);
    printf("%d\n", add_to_total(5));
    printf("%d\n", add_to_total(-2));
    printf("%d\n", counter);
    return 0;
}
