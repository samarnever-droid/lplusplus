#include <stdlib.h>
int balanced_path(int n) {
    int *v = calloc(n, 4);
    int total = 0;
    for (int i = 0; i < n; i++) {
        total += v[i];
    }
    free(v);
    return total;
}
int *escape_path(int n) {
    int *v = calloc(n, 4);
    return v;
}
int leak_path(int n) {
    int *v = calloc(n, 4);
    return n;
}
int double_free_path(int n) {
    int *v = calloc(n, 4);
    free(v);
    free(v);
    return n;
}
int divergent_path(int n) {
    int *v = calloc(n, 4);
    if (n > 0) {
        free(v);
        return 1;
    }
    return v == 0;
}
int goto_path(int n) {
    int *v = calloc(n, 4);
    if (v == 0) goto done;
    free(v);
    return n;
done:
    return -1;
}
int realloc_balanced(int n) {
    int *v = calloc(n, 4);
    v = realloc(v, (n + 1) * 4);
    free(v);
    return n;
}
