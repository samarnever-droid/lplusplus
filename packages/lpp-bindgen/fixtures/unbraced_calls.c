void set_value(int *value, int replacement) {
    *value = replacement;
}

void maybe_set(int condition, int *value, int replacement) {
    if (condition) set_value(value, replacement);
}

void either_set(int condition, int *value, int when_true, int when_false) {
    if (condition) set_value(value, when_true);
    else set_value(value, when_false);
}

int read_after_set(int condition, int *value) {
    if (condition) set_value(value, 31);
    return *value;
}
