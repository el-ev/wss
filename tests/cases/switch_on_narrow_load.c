// switch_on_narrow_load.c: switch indices come from signed and unsigned
// byte loads, so -1 vs 255 and -128 vs 128 must dispatch differently.
static volatile signed char keys[] = {0, 1, 2, -1, 3, 127, -128, 4};
static volatile unsigned char ukeys[] = {0, 1, 2, 255, 3, 127, 128, 4};

__attribute__((noinline)) static int f(int k) {
  switch (k) {
  case 0:
    return k + 100;
  case 1:
    return k * 50;
  case 2:
    return k ^ 0x33;
  case 3:
    return k - 7;
  case 4:
    return k << 3;
  default:
    return k * 2 + 1;
  }
}

int _start(void) {
  int s = 0;
  for (int i = 0; i < 8; i++) {
    s = s * 5 + f(keys[i]) - f(ukeys[i]);
  }
  return s;
}
