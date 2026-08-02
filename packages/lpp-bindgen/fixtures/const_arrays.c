typedef unsigned char u8;

static const u8 weights[8] = {
    3, 0, 5, 0x10, 0, 9, 12, 1,
};

static const unsigned short inferred[] = {
    100, 0, 250, 7,
};

int weight_at(int index) {
    return weights[index];
}

int inferred_at(int index) {
    return inferred[index];
}

int weighted_sum(int left, int right) {
    return weights[left] + inferred[right];
}
