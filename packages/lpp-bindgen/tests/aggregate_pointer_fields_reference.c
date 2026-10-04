#include <stdio.h>

#include "../fixtures/aggregate_pointer_fields.c"

int main(void) {
    Link third = {29, 0};
    Link second = {13, &third};
    Link first = {7, &second};
    Link copy = {5, 0};
    printf("%d\n", link_has_next(&first));
    printf("%d\n", link_next_value(&first));
    printf("%d\n", link_get_next(&first) == &second);
    link_set_next(&first, &third);
    printf("%d\n", link_next_value(&first));
    link_copy_next(&copy, &second);
    printf("%d\n", link_next_value(&copy));
    link_clear_next(&first);
    printf("%d\n", link_has_next(&first));
    return 0;
}
