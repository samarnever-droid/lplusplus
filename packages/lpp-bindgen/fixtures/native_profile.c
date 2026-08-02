#include <stdlib.h>

typedef struct Item {
    int value;
    unsigned flags : 3;
} Item;

static int base = 7;
static int calls;
static int *seed_ptr = &base;

int process(int n) {
    Item *items = calloc(n, sizeof(Item));
    int total = *seed_ptr + 4;
    for (int i = 0; i < n; i++) {
        items[i].value = base + i * 2;
        items[i].flags = i & 7;
        total += items[i].value + items[i].flags;
    }
    Item *first = &items[0];
    (*first).value += 1;
    total += first->value;
    switch (n & 3) {
        case 0:
            total += 10;
            break;
        case 1:
            total += 20;
        case 2:
            total += 3;
            goto cleanup;
        default:
            total -= 1;
            break;
    }
cleanup:
    calls += 1;
    free(items);
    return total + calls;
}
