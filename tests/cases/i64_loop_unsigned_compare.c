// i64_loop_unsigned_compare.c: an i64 loop counter stepped by add that wraps
// through zero, with exit and selects driven by unsigned 64-bit comparisons
// whose operands differ only in the high half or only in the low half, plus
// eq / ne / eqz.
static volatile unsigned long long start = 0xfffffffffffffffdull; // -3
static volatile unsigned long long stop = 4ull;
static volatile unsigned long long big = 0x0000000100000000ull;
static volatile unsigned long long small = 0x00000000ffffffffull;

long long _start(void) {
  unsigned long long acc = 0;
  for (unsigned long long i = start; i != stop; i++) {
    acc += (i > big) ? 1000ull : 0ull;
    acc += (i < small) ? 1ull : 0ull;
    acc ^= (i == 0) ? 0x5000000000ull : 0ull;
    acc += i & 0xffull;
    acc -= (i >= 0xfffffffe00000000ull) ? 0x100000000ull : 0ull;
  }
  acc += (big > small) ? 7ull : 0ull;
  acc += (big <= small) ? 70ull : 0ull;
  acc += (small + 1 == big) ? 700ull : 0ull;
  acc += (big != small) ? 7000ull : 0ull;
  acc += (big - small == 1) ? 70000ull : 0ull;
  acc += !acc ? 1ull : 0ull;
  return (long long)acc;
}
