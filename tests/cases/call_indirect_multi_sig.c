// call_indirect_multi_sig.c: call_indirect through tables of three different
// signatures (unary, binary, void), with a callee that mutates a global the
// other callees read.
typedef int (*un_fn)(int);
typedef int (*bin_fn)(int, int);
typedef void (*nul_fn)(void);

static volatile int g = 1;

static int inc(int x) { return x + g; }
static int dbl(int x) { return x * 2; }
static int neg(int x) { return -x; }
static int add(int a, int b) { return a + b; }
static int sub(int a, int b) { return a - b; }
static int mulg(int a, int b) { return a * b * g; }
static void bump(void) { g += 3; }

static un_fn volatile U[3] = {inc, dbl, neg};
static bin_fn volatile B[3] = {add, sub, mulg};
static nul_fn volatile N[1] = {bump};

int _start(void) {
  int acc = 7;
  for (int i = 0; i < 3; i++) {
    acc = B[i](U[i](acc), U[(i + 1) % 3](acc));
    N[0]();
  }
  return acc; // 22 -> 66 -> -66*73*7 = -33726
}
