static int f(int n, int *base){
  int *p;
  p = base;
  return p[n];
}
static int g(int n, int *base){
  int *p = 0;
  if (n > 0) p = base;
  return p ? p[0] : -1;
}
