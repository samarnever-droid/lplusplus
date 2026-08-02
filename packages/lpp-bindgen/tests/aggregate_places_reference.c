#include <stdio.h>

#include "../fixtures/aggregate_places.c"

int main(void) {
    Node node = {0};
    node.id = 7;
    node.total = 100;
    node.inner.delta = -4;
    node.inner.flags = 5;
    node.values[0] = 11;
    node.values[1] = 13;
    node.values[2] = 17;
    printf("%zu\n", sizeof(Inner));
    printf("%zu\n", sizeof(Node));
    printf("%d\n", inner_flags(&node.inner));
    printf("%d\n", node_pointer_present(&node));
    printf("%d\n", node_id(&node));
    printf("%d\n", node_preincrement(&node));
    printf("%ld\n", node_total(&node));
    printf("%d\n", node_nested(&node));
    printf("%d\n", node_value(&node, 2));
    node_set_id(&node, 9);
    node_add_total(&node, 23);
    node_set_delta(&node, -8);
    node_set_flags(&node, 3);
    node_set_value(&node, 1, 29);
    printf("%d\n", node.id);
    printf("%ld\n", node.total);
    printf("%d\n", node.inner.delta);
    printf("%u\n", node.inner.flags);
    printf("%d\n", node.values[1]);
    return 0;
}
