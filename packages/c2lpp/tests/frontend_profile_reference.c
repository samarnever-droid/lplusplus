#include <stdio.h>
#include <stdlib.h>
#include "../fixtures/frontend_profile.c"

int main(void) {
    printf("%d\n", run_graph(4));
    printf("%d\n", run_graph(3));
    printf("%d\n", run_graph(1));
    return 0;
}
