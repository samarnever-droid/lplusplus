int conditional_local(int condition, int *value) {
    int selected = condition ? *value : 9;
    return selected;
}

int conditional_assignment(int condition, int when_true, int when_false) {
    int selected = 0;
    selected = condition ? when_true : when_false;
    return selected;
}

int conditional_compound(int condition) {
    int selected = 10;
    selected += condition ? 2 : 5;
    return selected;
}

int *conditional_pointer_local(int condition, int *value) {
    int *selected = condition ? value : 0;
    return selected;
}

int character_class(int value) {
    return value == '_' ? 'Y' : 'N';
}

int escape_total(void) {
    return '\n' + '\t' + '\\';
}
