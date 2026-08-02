int first_value(int *values) {
    return values[0];
}

int uses_goto(int value) {
    goto done;
done:
    return value;
}
