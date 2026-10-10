// memory_high_addresses.c: a 3000-byte array (memory_bytes raised to 4096)
// touched at a prime stride, so addresses above 1024 are exercised through
// the 16-bit address path.
static volatile unsigned char big[3000];

int _start(void) {
  for (unsigned i = 0; i < 3000u; i += 251u) {
    big[i] = (unsigned char)(i ^ (i >> 8));
  }
  *(volatile unsigned *)(big + 2996) = 0xc0ffee42u;
  unsigned acc = 0;
  for (unsigned i = 0; i < 3000u; i += 251u) {
    acc = acc * 33u + big[i];
  }
  return (int)(acc ^ *(volatile unsigned *)(big + 2996) ^ big[2761]);
}
