// try_getchar_drain.c
extern int try_getchar(void);
extern int putchar(int c);

// Blocking getchar as a polling loop over try_getchar.
static int getchar_blocking(void) {
  int c;
  while ((c = try_getchar()) < 0) {
  }
  return c;
}

int _start(void) {
  int (*volatile poll)(void) = try_getchar;
  int first = getchar_blocking();
  putchar(first);
  int n = 1;
  int c;
  while ((c = poll()) >= 0) {
    putchar(c);
    n++;
  }
  putchar('\n');
  return (n << 8) | first;
}
