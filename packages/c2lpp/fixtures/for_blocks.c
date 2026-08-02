int sum_odd_steps(int limit) {
    int total = 0;
    for (int index = 1; index <= limit; index += 2) {
        total += index;
    }
    return total;
}

int count_down(int limit) {
    int total = 0;
    int index = 0;
    for (index = limit; index > 0; index--) {
        total += index;
    }
    return total;
}

int sum_adjusted(int limit) {
    int total = 0;
    for (int index = 0; index < limit; index++) {
        if (index & 1) {
            total += index;
        } else {
            total += 2;
        }
    }
    return total;
}
