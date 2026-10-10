// i64_divrem_mixed.c: 64-bit signed/unsigned div and rem with runtime
// divisors, including values larger than 2^32 and negative operands.
static volatile unsigned long long un[] = {0xffffffffffffffffull, 0x123456789abcdefull, 1ull};
static volatile unsigned long long ud[] = {7ull, 0x100000000ull, 0xffffffffull};
static volatile long long sn[] = {-1000000000000ll, 0x7fffffffffffffffll, -7ll};
static volatile long long sd[] = {-3ll, 1000000007ll, 2ll};

long long _start(void) {
  unsigned long long acc = 0;
  for (int i = 0; i < 3; i++) {
    acc = ((acc << 5) | (acc >> 59)) ^ (un[i] / ud[i]) ^ ((un[i] % ud[i]) << 8);
    acc = ((acc << 5) | (acc >> 59)) ^ (unsigned long long)(sn[i] / sd[i]) ^
          ((unsigned long long)(sn[i] % sd[i]) << 8);
  }
  return (long long)acc;
}
