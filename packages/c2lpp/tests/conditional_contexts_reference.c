#include <stdio.h>

#include "../fixtures/conditional_contexts.c"

int main(void) {
    int value = 41;
    printf("%d\n", conditional_local(1, &value));
    printf("%d\n", conditional_local(0, 0));
    printf("%d\n", conditional_assignment(1, 7, 13));
    printf("%d\n", conditional_assignment(0, 7, 13));
    printf("%d\n", conditional_compound(1));
    printf("%d\n", conditional_compound(0));
    printf("%d\n", conditional_pointer_local(1, &value) == &value);
    printf("%d\n", conditional_pointer_local(0, &value) == 0);
    printf("%d\n", character_class('_'));
    printf("%d\n", character_class('x'));
    printf("%d\n", escape_total());
    return 0;
}
