#include <stdio.h>

int count_down(int n);
int clamp_to_zero(int n);

int main(void){
  printf("%d\n", count_down(5));
  printf("%d\n", count_down(-2));
  printf("%d\n", clamp_to_zero(-7));
  printf("%d\n", clamp_to_zero(9));
  return 0;
}
