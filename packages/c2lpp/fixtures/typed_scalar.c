int add_scaled(int a, int b) {
    int product = a * b;
    product += a;
    return product;
}

int bit_mix(int value) {
    int shifted = value << 2;
    return shifted ^ 3;
}

int call_chain(int value) {
    return add_scaled(value, 2) + bit_mix(value);
}
