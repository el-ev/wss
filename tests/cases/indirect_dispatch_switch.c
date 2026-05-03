// indirect_dispatch_switch.c
typedef int (*op_t)(int, int);

__attribute__((noinline)) static int add2(int a, int b) { return a + b; }
__attribute__((noinline)) static int sub2(int a, int b) { return a - b; }
__attribute__((noinline)) static int mul2(int a, int b) { return a * b; }
__attribute__((noinline)) static int xor2(int a, int b) { return a ^ b; }

int _start(void) {
  op_t table[4] = {add2, sub2, mul2, xor2};
  int acc = 0;

  for (int i = 0; i < 8; i++) {
    int a = (i + 3) * 7;
    int b = (i & 3) + 1;
    int v = table[i & 3](a, b);

    switch (i & 3) {
      case 0:
        acc += v;
        break;
      case 1:
        acc -= v;
        break;
      case 2:
        acc ^= v;
        break;
      default:
        acc += (v << 1);
        break;
    }
  }

  return acc;
}
