typedef struct Opaque Opaque;

int pointer_nonnull(Opaque *value) {
    return value != 0;
}

Opaque *pointer_null(void) {
    return 0;
}
