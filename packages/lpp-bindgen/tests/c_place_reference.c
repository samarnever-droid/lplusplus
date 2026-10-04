#include <stdio.h>

typedef struct Inner {
    unsigned flags : 3;
    int score;
} Inner;

typedef struct Record {
    int values[4];
    Inner inner;
} Record;

int main(void) {
    Record record = {0};
    int *value0 = &record.values[0];
    int *value1 = &record.values[1];
    int *value2 = &record.values[2];

    printf("%d\n", *value0);
    printf("%d\n", *value0 = 10);
    printf("%d\n", *value0 += 5);
    printf("%d\n", (*value0)++);
    printf("%d\n", *value0);
    printf("%d\n", --(*value0));
    printf("%d\n", *value0 *= 2);
    printf("%d\n", *value0 /= 3);
    printf("%d\n", *value0 %= 6);
    printf("%d\n", *value0 |= 8);
    printf("%d\n", *value0 &= 10);
    printf("%d\n", *value0 ^= 3);
    printf("%d\n", *value0 <<= 1);
    printf("%d\n", *value0 >>= 1);

    printf("%u\n", record.inner.flags = 5);
    record.inner.flags += 5;
    printf("%u\n", record.inner.flags);
    printf("%d\n", record.inner.score = -7);
    printf("%d\n", *(&record.values[0]));
    printf("%d\n", value0 == &record.values[0]);

    *value1 = 41;
    *value2 = *value1;
    printf("%d\n", *value2);
    int temporary = *value0;
    *value0 = *value1;
    *value1 = temporary;
    printf("%d\n", *value0);
    printf("%d\n", *value1);
    printf("%d\n", (*value1)--);
    printf("%d\n", *value1);
    return 0;
}
