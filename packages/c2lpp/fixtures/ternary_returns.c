int choose_int(int condition, int when_true, int when_false) {
    return condition ? when_true : when_false;
}

int choose_comparison(int left, int right) {
    return left < right ? left == 3 : right == 9;
}

double choose_float(int condition, double when_true, double when_false) {
    return condition ? when_true : when_false;
}

int *choose_pointer(int condition, int *value) {
    return condition ? value : 0;
}

int load_or(int *value, int fallback) {
    return value ? *value : fallback;
}

int both_nonnull(int *left, int *right) {
    return left && right;
}

int guarded_value(int *value) {
    return value && *value == 37;
}
