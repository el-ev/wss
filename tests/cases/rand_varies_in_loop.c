// rand_varies_in_loop.c

extern int rand(void);
extern int putchar(int c);

int _start(void) {
  int values[10];
  for (int i = 0; i < 10; i++) {
    values[i] = rand();
  }
  int all_same = 1;
  for (int i = 1; i < 10; i++) {
    if (values[i] != values[0]) {
      all_same = 0;
    }
  }
  if (all_same) {
    putchar('F');
    putchar('A');
    putchar('I');
    putchar('L');
  } else {
    putchar('v');
    putchar('a');
    putchar('r');
  }
  putchar('\n');
  return all_same;
}
