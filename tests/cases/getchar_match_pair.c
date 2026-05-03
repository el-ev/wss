// getchar_match_pair.c
extern int getchar(void);
extern int putchar(int c);

int _start(void) {
  int a = getchar();
  int b = getchar();

  if (a == 'o' && b == 'k') {
    putchar('Y');
    putchar('\n');
    return 0x123;
  }

  putchar('N');
  putchar('\n');
  return 0;
}
