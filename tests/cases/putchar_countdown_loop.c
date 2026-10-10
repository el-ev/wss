// putchar_countdown_loop.c: print loops whose exit is an independent
// countdown (`while (n--)`), not a NUL test on the loaded byte and not an
// `i < n` compare on the pointer. Must print exactly n characters even
// though the string continues past them: "abc|efghi".
extern int putchar(int c);

static volatile char text[] = "abcdefghij";
static volatile int n0 = 3;

__attribute__((noinline)) static void put_countdown(const volatile char *p, int n) {
  while (n) {
    putchar(*p++);
    n--;
  }
}

int _start(void) {
  put_countdown(text, n0);
  putchar('|');
  put_countdown(text + 4, n0 + 2);
  putchar('\n');
  return n0;
}
