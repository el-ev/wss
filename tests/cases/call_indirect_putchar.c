// call_indirect_putchar.c
extern int putchar(int c);

typedef int (*putchar_fn_t)(int);

__attribute__((noinline)) static int invoke(int idx, int ch) {
  putchar_fn_t table[1] = {putchar};
  int echoed = table[idx](ch);
  return echoed + 1;
}

int _start(void) { return invoke(0, 'Q'); }
