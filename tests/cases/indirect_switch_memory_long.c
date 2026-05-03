// indirect_switch_memory_long.c
typedef int (*op_t)(int, int);

static volatile int seed = 0x01234567;

__attribute__((noinline)) static int op_add(int a, int b) { return a + b; }
__attribute__((noinline)) static int op_sub(int a, int b) { return a - b; }
__attribute__((noinline)) static int op_xor(int a, int b) { return a ^ b; }
__attribute__((noinline)) static int op_mix(int a, int b) {
  unsigned ua = (unsigned)a;
  unsigned ub = (unsigned)b;
  unsigned r = (ua << (ub & 7u)) | (ua >> ((8u - ub) & 7u));
  return (int)(r ^ (ua + ub));
}

int _start(void) {
  op_t table[4] = {op_add, op_sub, op_xor, op_mix};
  volatile int *mem = (volatile int *)(unsigned)0x300;
  int acc = seed;

  for (int i = 0; i < 14; i++) {
    int a = acc ^ (i * 29);
    int b = (i & 7) + 5;
    int v = table[(acc + i) & 3](a, b);

    mem[i & 3] = v ^ acc;
    int r = mem[i & 3];

    switch ((v ^ i) & 7) {
      case 0:
        acc += r;
        break;
      case 1:
        acc -= (r >> 1);
        break;
      case 2:
        acc ^= (r << 1);
        continue;
      case 3:
        acc += (r & 255);
        break;
      case 4:
        acc -= (r | 17);
        break;
      case 5:
        acc ^= (r >> 3);
        break;
      case 6:
        acc += (r ^ b);
        continue;
      default:
        acc -= (r ^ a);
        break;
    }

    acc = (acc << 1) ^ (int)((unsigned)acc >> 5) ^ i;
  }

  seed = acc;
  return acc;
}
