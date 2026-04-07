int _start(void) {
  volatile unsigned int a = 0x12345678u;
  volatile unsigned int b = 3u;

  unsigned int hi = (a / b) << 24;
  unsigned int rem = a % b;
  return (int)(hi ^ rem);
}
