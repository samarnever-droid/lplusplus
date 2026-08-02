#include <stdio.h>
#include <stdlib.h>
#include <string.h>

static int sum_i32(const int *values, int count) {
    int total = 0;
    for (int i = 0; i < count; ++i) total += values[i];
    return total;
}

int main(void) {
    int *values = (int *)calloc(4, sizeof(int));
    if (!values) return 2;
    values[0] = 3;
    values[1] = 5;
    values[2] = 7;
    values[3] = 9;
    printf("%d\n", sum_i32(values, 4));

    int *grown = (int *)realloc(values, 6 * sizeof(int));
    if (!grown) { free(values); return 3; }
    values = grown;
    values[4] = 11;
    values[5] = 13;
    printf("%d\n", sum_i32(values, 6));
    printf("%td\n", (values + 6) - values);

    char *left = (char *)malloc(7);
    char *right = (char *)malloc(7);
    if (!left || !right) return 4;
    memcpy(left, "sqlite", 7);
    memcpy(right, "sqlite", 7);
    printf("%zu\n", strlen(left));
    printf("%d\n", strcmp(left, right));

    memmove((unsigned char *)values + 4, values, 20);
    printf("%d\n", sum_i32(values, 6));

    free(left);
    free(right);
    free(values);
    return 0;
}
