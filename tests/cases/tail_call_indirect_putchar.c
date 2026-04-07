extern int putchar(int c);

typedef int (*putchar_fn_t)(int);

static volatile int g_idx = 0;

__attribute__((noinline)) static int dispatch_putchar(int idx, int ch) {
  putchar_fn_t table[1] = {putchar};
  return table[idx](ch);
}

int _start(void) {
  int idx = g_idx;
  return dispatch_putchar(idx, 'Z'); // 90 (0x5a)
}
