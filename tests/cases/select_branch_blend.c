// select_branch_blend.c
static volatile int sx = 9;
static volatile int sy = -14;

__attribute__((noinline)) static int choose(int cond, int a, int b) {
  return cond ? a : b;
}

int _start(void) {
  int x = sx;
  int y = sy;
  int acc = 0;

  for (int i = 0; i < 6; i++) {
    int t = (x + i) * (y - i);
    int u = choose((t & 4) != 0, t ^ (i << 3), t + (i * 7));
    acc += (u > 0) ? (u >> 2) : (u << 1);
    acc ^= choose((i & 1) != 0, x - i, y + i);
  }

  sx = x + 1;
  sy = y - 1;
  return acc;
}
