int read_first(const int *values) {
    return *values;
}

int read_at(const int *values, int index) {
    return values[index];
}

int sum_edges(const int *values, int last) {
    return values[0] + values[last];
}

void write_at(int *values, int index, int value) {
    values[index] = value;
}

void add_at(int *values, int index, int value) {
    values[index] += value;
}

void increment_first(int *values) {
    (*values)++;
}

int *address_at(int *values, int index) {
    return &values[index];
}

int local_shift(int *values) {
    int *selected = values + 1;
    *selected += 2;
    return *selected;
}

int pointer_distance(int *values, int left, int right) {
    return &values[right] - &values[left];
}

int pointer_same(int *values, int index) {
    return (values + index) == &values[index];
}
