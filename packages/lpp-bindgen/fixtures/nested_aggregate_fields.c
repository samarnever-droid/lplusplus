typedef struct { int a; int b; } Pair;
typedef struct { Pair *p; int n; } Box;
static int f(Box *b){
  if (b->p) {
    return b->p->a;
  }
  return -1;
}
static int f1(Box *b){
  if (b->n > 0) {
    return 1;
  }
  return 0;
}
