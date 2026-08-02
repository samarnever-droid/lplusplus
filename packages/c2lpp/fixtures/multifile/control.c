#include "common.h"

typedef union C2Value {
    int integer;
    double real;
} C2Value;

int classify_value(int value) {
    int result = 0;
    switch (value) {
        case 0:
            result = 10;
            break;
        case 1:
            result = 20;
            goto done;
        default:
            result = (int)sizeof(C2Value);
            break;
    }
done:
    return result;
}

int format_probe(const char *format, ...) {
    return format == 0;
}
