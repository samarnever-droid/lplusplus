int later_add(int value);

int call_wrapper(int value) {
    later_add(value);
    (void)later_add(value + 1);
    return later_add(value + 1);
}

int later_add(int value) {
    return value + 1;
}
