// rand_smoke.c

extern int rand(void);
extern int putchar(int c);

int _start(void) {
  volatile int sink = rand();
  sink ^= rand();
  putchar('o');
  putchar('k');
  putchar('\n');
  return 0;
}
