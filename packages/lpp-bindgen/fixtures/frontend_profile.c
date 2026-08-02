#define SCALE 3

typedef struct Node Node;
typedef int (*VisitFn)(Node *, int);

typedef struct Meta {
    unsigned kind : 3;
    unsigned ready : 1;
} Meta;

struct Node {
    int values[3];
    Meta meta;
};

static const char banner[] = "front";
static const int weights[3] = {2, 4, 6};

int visit(Node *node, int extra);
int log_values(const char *fmt, ...);

int run_graph(int n) {
    Node *nodes = calloc(n, sizeof(Node));
    int total = 0;
    for (int i = 0; i < n; i++) {
        nodes[i].values[0] = weights[0] + i;
        nodes[i].values[1] = weights[1] + i;
        nodes[i].values[2] = weights[2] + i;
        nodes[i].meta.kind = (i * SCALE) & 7;
        nodes[i].meta.ready = 1;
        total += nodes[i].values[0] + nodes[i].values[1] + nodes[i].values[2] + nodes[i].meta.kind + nodes[i].meta.ready;
    }
    Node *first = &nodes[0];
    (*first).values[1] += 1;
    total += first->values[1];
    switch (n & 3) {
        case 0:
            total += 10;
            break;
        case 1:
            total += 20;
        case 2:
            total += 3;
            goto done;
        default:
            total -= 1;
            break;
    }
done:
    free(nodes);
    return total + banner[0];
}
