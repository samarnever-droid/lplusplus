#include <stdio.h>

int main(void) {
    int values[4] = {11, 22, 0, 0};
    int *slots[3] = {0, 0, 0};

    printf("%d\n", slots[0] == 0);
    slots[0] = &values[0];
    printf("%d\n", slots[0] == 0);
    printf("%td\n", (char *)slots[0] - (char *)&values[0]);
    printf("%zu\n", sizeof(int));
    printf("%d\n", *slots[0]);

    slots[1] = &values[1];
    printf("%td\n", (char *)slots[1] - (char *)&values[0]);
    printf("%d\n", *slots[1]);

    slots[2] = slots[0];
    printf("%td\n", (char *)slots[2] - (char *)&values[0]);
    int *temporary = slots[0];
    slots[0] = slots[1];
    slots[1] = temporary;
    printf("%td\n", (char *)slots[0] - (char *)&values[0]);
    printf("%td\n", (char *)slots[1] - (char *)&values[0]);
    slots[1] = 0;
    printf("%d\n", slots[1] == 0);
    printf("%d\n", &slots[0] == &slots[0]);
    printf("%d\n", &slots[0] == &slots[2]);
    return 0;
}
