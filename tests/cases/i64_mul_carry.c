// i64_mul_carry.c: 64-bit products whose partial products carry across the
// 32-bit halves, plus signed products with negative operands.
static volatile unsigned long long a = 0xffffffffull;
static volatile unsigned long long b = 0x100000001ull;
static volatile unsigned long long c = 0xfedcba9876543210ull;
static volatile unsigned long long d = 0x8000000000000000ull;
static volatile long long s = -3;
static volatile long long t = 0x7fffffffffffffffll;

long long _start(void) {
  unsigned long long r = a * a;
  r ^= b * b;
  r ^= c * 7ull;
  r ^= d * 2ull;
  r += (unsigned long long)(s * s);
  r ^= (unsigned long long)(t * -1);
  return (long long)r;
}
