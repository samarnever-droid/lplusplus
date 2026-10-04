#include <stdlib.h>

typedef int (*Transform)(int value);

static int plus_one(int value) {
    return value + 1;
}

int graph_analyze(int count, Transform transform) {
    int *values = (int *)calloc(count, sizeof(int));
    int total = 0;
    if (values == 0) {
        goto failed;
    }
    for (int i = 0; i < count; i++) {
        values[i] = i + 1;
        if (transform != 0) {
            values[i] = transform(values[i]);
        }
        total += values[i];
    }
    switch (count & 3) {
        case 0:
            total += 10;
            break;
        case 1:
            total += 20;
        case 2:
            total += 3;
            break;
        default:
            total -= 1;
            break;
    }
    values = (int *)realloc(values, (count + 1) * sizeof(int));
    free(values);
    return total;
failed:
    free(values);
    return -1;
}

int graph_entry(int count) {
    return graph_analyze(count, plus_one);
}
