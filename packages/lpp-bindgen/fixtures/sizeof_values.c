typedef struct Pair {
    int first;
    long second;
} Pair;

int size_of_char(void) {
    return sizeof(char);
}

int size_of_int(void) {
    return sizeof(int);
}

int size_of_pointer(void) {
    return sizeof(void *);
}

int size_of_pair(void) {
    return sizeof(Pair);
}

int size_of_dereference(Pair *pair) {
    return sizeof(*pair);
}
