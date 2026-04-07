static int g_counter = 0;

static int bump(int v) {
  g_counter = g_counter * 10 + v;
  return v & 1;
}

int _start(void) {
  int score = 0;

  if (0 && bump(1)) {
    score += 100;
  }
  if (1 || bump(2)) {
    score += 3;
  }
  if (bump(3) && bump(4)) {
    score += 10;
  }
  if (bump(5) || bump(6)) {
    score += 20;
  }

  return score + g_counter; // 23 + 345 = 368 (0x170)
}
