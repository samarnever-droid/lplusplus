int accepted_before(int value) {
    return value + 1;
}

int rejected_pointer(int ***value) {
    return ***value;
}

int rejected_bad_call(int value) {
    return accepted_before(value, 2);
}

int accepted_after(int value) {
    return accepted_before(value) * 2;
}
