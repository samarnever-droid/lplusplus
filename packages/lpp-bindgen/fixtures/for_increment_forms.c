static int f(int n){
  int i;
  int acc = 0;
  for (i = 0; i < n; i = i + 1) {
    acc = acc + i;
  }
  return acc;
}
static int g(int n){
  int i;
  int acc = 0;
  for (i = n; i > 0; i--) {
    acc = acc + i;
  }
  return acc;
}
static int h(int n){
  int i;
  int acc = 0;
  for (i = 0; i < n; i += 2) {
    acc = acc + i;
  }
  return acc;
}
