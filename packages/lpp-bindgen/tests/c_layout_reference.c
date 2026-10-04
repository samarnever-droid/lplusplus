#include <stdio.h>

typedef struct Packet {
    unsigned tag : 3;
    unsigned mode : 5;
    int value;
    unsigned ready : 1;
    unsigned count : 7;
} Packet;

typedef union Number {
    unsigned short small;
    long long large;
} Number;

int main(void) {
    Packet packet = {0};
    packet.tag = 5;
    packet.mode = 17;
    packet.value = -9;
    packet.ready = 1;
    packet.count = 77;
    printf("%zu\n", sizeof(Packet));
    printf("%zu\n", _Alignof(Packet));
    printf("%u\n", packet.tag);
    printf("%u\n", packet.mode);
    printf("%d\n", packet.value);
    printf("%u\n", packet.ready);
    printf("%u\n", packet.count);
    printf("%zu\n", sizeof(Number));
    printf("%zu\n", _Alignof(Number));
    return 0;
}
