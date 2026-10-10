// loop_unsigned_wrap_counter.c: loop counters that wrap through zero
// (unsigned) and cross zero (signed), with != exit tests and mixed
// signed/unsigned comparisons in the body.
static volatile unsigned ustart = 0xfffffffcu;
static volatile int sstart = -3;

int _start(void) {
  unsigned acc = 0;
  for (unsigned i = ustart; i != 2u; i++) {
    acc = acc * 5u + (i & 0xfu) + (i < 0x10u ? 100u : 0u) + ((int)i < 0 ? 7u : 0u);
  }
  for (int j = sstart; j != 3; j++) {
    acc = acc * 3u + (unsigned)(j * j) + ((unsigned)j > 100u ? 1u : 0u);
  }
  return (int)acc;
}
