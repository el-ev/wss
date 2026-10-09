// try_getchar_none.c
extern int try_getchar(void);
extern int putchar(int c);

int _start(void) {
  int c = try_getchar();
  putchar(c < 0 ? '-' : c);
  putchar('\n');
  return c;
}
