__attribute((optnone)) int _start(void) {
  int a = 7;
  int b = 5;
  int c = 3;

  int r = a + b; // 12
  r = r * 4;     // 48
  r = r - 9;     // 39
  r = r + c - c; // keep `c` live
  return r;      // 0x00000027
}
