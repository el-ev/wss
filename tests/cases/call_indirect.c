typedef int (*binop_t)(int, int);

static int add(int a, int b) { return a + b; }
static int sub(int a, int b) { return a - b; }
static int mul(int a, int b) { return a * b; }

int _start(void) {
  binop_t table[3] = {add, sub, mul};

  int r = table[2](6, 7); // 42
  r += table[0](1, 2);    // 45
  r += table[1](9, 5);    // 49
  return r;               // 0x31
}

