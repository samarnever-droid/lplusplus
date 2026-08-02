int read_indirect(int **value) {
    return **value;
}

int read_pointer_array(int **values, int index) {
    return *values[index];
}

void write_pointer_array(int **values, int index, int *value) {
    values[index] = value;
}

void write_indirect(int **value, int *replacement) {
    *value = replacement;
}

int read_array_parameter(int values[], int index) {
    return values[index];
}

int read_fixed_parameter(int values[4], int index) {
    return values[index];
}
