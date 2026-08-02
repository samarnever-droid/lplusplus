#include "common.h"
#include <stdint.h>

int clamp_shared(int value, int low, int high) {
    if (value < low) {
        return low;
    }
    if (value > high) {
        return high;
    }
    return value;
}

int sum_bytes(const uint8_t values[4]) {
    int total = 0;
    for (int i = 0; i < 4; ++i) {
        total += values[i];
    }
    return total;
}
