int sum_to(int n) {
    int total = 0;
    int i = 0;
    while (i < n) {
        total += i;
        i++;
    }
    return total;
}

int sum_for(int n) {
    int total = 0;
    for (int i = 0; i < n; i++) {
        total += i;
    }
    return total;
}

int absolute_value(int value) {
    if (value < 0) {
        return -value;
    }
    return value;
}

int clamp(int value, int low, int high) {
    if (value < low) {
        return low;
    }
    if (value > high) {
        return high;
    }
    return value;
}

int array_score(void) {
    int values[4] = {2, 4, 6, 8};
    values[2] = 10;
    return values[0] + values[1] + values[2] + values[3];
}
