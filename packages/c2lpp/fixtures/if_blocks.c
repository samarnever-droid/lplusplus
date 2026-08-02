int adjust_value(int value) {
    int total = value;
    if (value < 0) {
        total = -value;
    } else {
        total += 2;
    }
    return total;
}
