int for_control(int limit) {
    int total = 0;
    for (int index = 0; index < limit; index++) {
        if (index == 2) {
            continue;
        }
        if (index == 7) {
            break;
        }
        total += index;
    }
    return total;
}

int while_control(int limit) {
    int index = 0;
    int total = 0;
    while (index < limit) {
        index++;
        if ((index % 2) == 0) {
            continue;
        }
        if (index > 7) {
            break;
        }
        total += index;
    }
    return total;
}
