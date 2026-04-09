typedef unsigned (*binop_t)(unsigned, unsigned);

static volatile unsigned seed = 0x31415926u;

__attribute__((noinline)) static unsigned addop(unsigned a, unsigned b) { return a + b; }
__attribute__((noinline)) static unsigned subop(unsigned a, unsigned b) { return a - b; }
__attribute__((noinline)) static unsigned xorop(unsigned a, unsigned b) { return a ^ b; }
__attribute__((noinline)) static unsigned mulop(unsigned a, unsigned b) { return a * b; }

int _start(void) {
  binop_t table[4] = {addop, subop, xorop, mulop};
  unsigned acc = seed;

  for (unsigned i = 0; i < 12u; i++) {
    unsigned a = acc ^ (i * 37u);
    unsigned b = (i & 7u) + 3u;
    unsigned v = table[(i + acc) & 3u](a, b);

    switch (v & 3u) {
      case 0:
        acc += v;
        break;
      case 1:
        acc ^= (v >> 1);
        continue;
      case 2:
        acc -= (v << 1);
        break;
      default:
        acc += (v ^ i);
        break;
    }

    acc = (acc << 1) ^ (acc >> 3) ^ i;
  }

  seed = acc;
  return (int)acc;
}
