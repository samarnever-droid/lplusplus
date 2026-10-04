int sum_while(int limit) {
    int index = 0;
    int total = 0;
    while (index < limit) {
        total += index;
        index++;
    }
    return total;
}

int normalize_nonzero(int value) {
    int remaining = value;
    int count = 0;
    while (remaining) {
        remaining -= 1;
        count += 1;
    }
    return count;
}
