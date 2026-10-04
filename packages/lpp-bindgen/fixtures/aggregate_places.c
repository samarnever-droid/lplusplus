typedef unsigned char u8;

typedef struct Inner {
    int delta;
    unsigned flags : 3;
} Inner;

typedef struct Node {
    int id;
    void *opaque;
    long total;
    Inner inner;
    int values[3];
} Node;

int inner_flags(Inner *inner) {
    return inner->flags;
}

int node_id(Node *node) {
    return node->id;
}

int node_preincrement(Node *node) {
    return ++node->id;
}

long node_total(Node *node) {
    return node->total;
}

int node_nested(Node *node) {
    return node->inner.delta + node->inner.flags;
}

int node_value(Node *node, int index) {
    return node->values[index];
}

int node_pointer_present(Node *node) {
    return node->opaque != 0;
}

void node_set_id(Node *node, int value) {
    node->id = value;
}

void node_add_total(Node *node, long value) {
    node->total += value;
}

void node_set_delta(Node *node, int delta) {
    node->inner.delta = delta;
}

void node_set_flags(Node *node, int flags) {
    node->inner.flags = flags;
}

void node_set_value(Node *node, int index, int value) {
    node->values[index] = value;
}
