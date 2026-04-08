__attribute((optnone))
int _start(void) {
  unsigned int a = 0xf0f0f0f0u;
  unsigned int b = 0x0ff00ff0u;

  unsigned int r = (a & b);  // 0x00f000f0
  r |= (a ^ b);              // 0xfff0fff0
  r = r >> 4;                // 0x0fff0fff
  r = r << 1;                // 0x1ffe1ffe

  return (int)r;
}

