// putchar_cond_arms.c: putchar in one arm of a diamond, in either
// orientation, alongside arms that both print or do other work. Prints
// "x+-x+x+--x+-" followed by a newline and returns the number of set flags.
extern int putchar(int c);

static volatile unsigned char flags[] = {1, 0, 1, 1, 0, 0, 1, 0};

int _start(void) {
  int n = 0;
  for (int i = 0; i < 8; i++) {
    unsigned f = flags[i];
    if (f) {
      putchar('x');
    }
    if (!f) {
      putchar('-');
    } else {
      n++;
    }
    if (f == 0) {
    } else {
      putchar('+');
    }
  }
  putchar('\n');
  for (int i = 0; i < 8; i++) {
    if (flags[i]) {
      putchar('Y');
    } else {
      putchar('n');
    }
  }
  putchar('\n');
  return n;
}
