#include <stdio.h>

static int cfg_score(int value) {
    int total = 0;
start:
    switch (value) {
        case 0:
            total += 10;
            goto done;
        case 1:
            total += 20;
            /* intentional fallthrough */
        case 2:
            total += 3;
            break;
        default:
            total = -1;
            goto done;
    }
    if (total > 20) goto bonus;
    goto done;
bonus:
    total += 100;
done:
    return total;
}

int main(void) {
    printf("%d\n", cfg_score(0));
    printf("%d\n", cfg_score(1));
    printf("%d\n", cfg_score(2));
    printf("%d\n", cfg_score(5));
    return 0;
}
