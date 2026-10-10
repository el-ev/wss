// putchar_string_tails.c: NUL-terminated and bounded print loops over
// strings whose lengths cover every residue mod 4 (0, 1, 3, 4, 5, 9), so
// the unrolled print loop must gate its tail slots correctly.
extern int putchar(int c);

static const char s0[] = "";
static const char s1[] = "a";
static const char s3[] = "bcd";
static const char s4[] = "efgh";
static const char s5[] = "ijklm";
static const char s9[] = "nopqrstuv";

__attribute__((noinline)) static int puts_nul(const char *p) {
  int n = 0;
  for (; *p; p++) {
    putchar(*p);
    n++;
  }
  return n;
}

__attribute__((noinline)) static int puts_n(const char *p, int n) {
  for (int i = 0; i < n; i++) {
    putchar(p[i]);
  }
  return n;
}

int _start(void) {
  int total = 0;
  total += puts_nul(s0);
  total += puts_nul(s1);
  putchar('|');
  total += puts_nul(s3);
  putchar('|');
  total += puts_nul(s4);
  putchar('|');
  total += puts_nul(s5);
  putchar('|');
  total += puts_nul(s9);
  putchar('\n');
  total += puts_n(s9, 0);
  total += puts_n(s9, 4);
  putchar('|');
  total += puts_n(s9 + 2, 7);
  putchar('\n');
  return total; // 1+3+4+5+9+0+4+7 = 33
}
