int count_down(int n){
  int count = 0;
  while( n > 0 ){
    n -= 1;
    count += 1;
  }
  return count;
}

int clamp_to_zero(int n){
  if( n < 0 ){
    n = 0;
  }
  return n;
}
