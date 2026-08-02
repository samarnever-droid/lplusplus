#include <stdio.h>

#include "../fixtures/comma_statements.c"

int main(void) {
    int left = 1;
    int right = 2;
    printf("%d\n", comma_calls(7));
    printf("%d\n", discard_two(&left, &right));
    discard_three(3, 4, 5);
    printf("%d\n", left + right);
    return 0;
}
