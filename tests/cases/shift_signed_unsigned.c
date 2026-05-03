// shift_signed_unsigned.c
int _start(void) {
  int s = -16;
  unsigned int u = 0xf0000000u;

  int a = s >> 2;         // -4 (arithmetic shift)
  unsigned int b = u >> 28; // 15
  unsigned int c = 1u << 5; // 32

  return a + (int)b + (int)c; // 43
}

