int _start(void) {
  int a = 12;
  int b = 34;
  int x = (a < b) ? a : b;
  int y = (a > b) ? a : b;
  int z = (x ^ y) ? 1 : 0;
  return x + y + z; // 47 (0x2f)
}

