static int f1(int n){
  int i = 0;
  for (i = 0; i < n; ) {
    i = i + 2;
  }
  return i;
}
static int f2(int n){
  int i = 0;
  while (1) {
    i = i + 1;
    if (i >= n) break;
  }
  return i;
}
static int f3(int n){
  int i;
  int acc = 0;
  for (i = n; i > 0; i = i - 1) {
    acc = acc + i;
  }
  return acc;
}
