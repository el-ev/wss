// rand_indirect_call.c

extern int rand(void);
extern int putchar(int c);

typedef int (*rand_fn)(void);

int _start(void) {
  volatile rand_fn fp = rand;
  int v = fp();
  (void)v;
  putchar('o');
  putchar('k');
  putchar('\n');
  return 0;
}
