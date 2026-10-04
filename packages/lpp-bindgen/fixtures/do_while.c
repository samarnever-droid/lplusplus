int sum_do(int limit) {
    int index = 0;
    int total = 0;
    do {
        total += index;
        index++;
    } while (index < limit);
    return total;
}

int break_do(int stop) {
    int index = 0;
    do {
        index++;
        if (index == stop) {
            break;
        }
    } while (index < 10);
    return index;
}

int once_do(int value) {
    int result = 0;
    do {
        result = value;
    } while (0);
    return result;
}
