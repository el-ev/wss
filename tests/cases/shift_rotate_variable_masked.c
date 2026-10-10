// shift_rotate_variable_masked.c: shifts and rotates by runtime amounts
// including 32, 33, 64, 65 and negative values, which wasm masks mod 32.
static volatile unsigned vals[] = {0x80000001u, 0x12345678u, 0xffffffffu, 1u};
static volatile int amts[] = {0, 1, 7, 8, 15, 16, 31, 32, 33, 40, 63, 64, 65, -1, -31, 100};

int _start(void) {
  unsigned acc = 0x9e3779b9u;
  for (int i = 0; i < 4; i++) {
    for (int j = 0; j < 16; j++) {
      unsigned x = vals[i];
      unsigned s = (unsigned)amts[j];
      acc = ((acc << 3) | (acc >> 29));
      acc ^= x << (s & 31u);
      acc += x >> (s & 31u);
      acc ^= (unsigned)((int)x >> (s & 31u));
      acc += __builtin_rotateleft32(x, s);
      acc ^= __builtin_rotateright32(x, s);
    }
  }
  return (int)acc;
}
