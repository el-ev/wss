// rand_through_memory.c

extern int rand(void);
extern int putchar(int c);

static volatile int buf;

int _start(void) {
  int v = rand();
  buf = v;
  int w = buf;
  putchar(v == w ? 'o' : 'F');
  putchar('k');
  putchar('\n');
  return v == w ? 0 : 1;
}
