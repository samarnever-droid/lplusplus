#include <stdio.h>
#include "../fixtures/expression_statements.c"

int main(void) {
    printf("%d\n", call_wrapper(3));
    return 0;
}
