int step_value(int value) {
    return value + 1;
}

int comma_calls(int value) {
    step_value(value), step_value(value + 1);
    return step_value(value + 2);
}

int discard_two(int *left, int *right) {
    (void)left, (void)right;
    return 0;
}

void discard_three(int first, int second, int third) {
    (void)first, (void)second, (void)third;
}
