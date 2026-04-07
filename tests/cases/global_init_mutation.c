static int g_seed = 9;
static int g_data[4] = {3, 1, 4, 1};

static int twist(int x) {
  g_seed = g_seed * 5 + x;
  return g_seed ^ (x << 2);
}

int _start(void) {
  int acc = 0;
  for (int i = 0; i < 4; i++) {
    g_data[i] = twist(g_data[i]);
    acc += g_data[i] & 0xff;
  }

  return acc + g_seed; // 628 + 6046 = 6674 (0x1a12)
}
