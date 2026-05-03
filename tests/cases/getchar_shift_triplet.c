// getchar_shift_triplet.c
extern int getchar(void);
extern int putchar(int c);

int _start(void) {
  int a = getchar();
  int b = getchar();
  int c = getchar();

  putchar(a + 1);
  putchar(b + 1);
  putchar(c + 1);
  putchar('\n');

  return (a << 16) ^ (b << 8) ^ c;
}
