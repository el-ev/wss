// getchar_digits_parse.c: reads "20x7" from the console, parses the two
// numbers and operator, prints the decimal result "140" and returns it.
extern int getchar(void);
extern int putchar(int c);

int _start(void) {
  int a = 0;
  int c = getchar();
  a = a * 10 + (c - '0');
  c = getchar();
  a = a * 10 + (c - '0');
  int op = getchar();
  int b = getchar() - '0';
  int r = (op == 'x') ? a * b : a + b;
  if (r >= 100) {
    putchar('0' + r / 100);
  }
  putchar('0' + (r / 10) % 10);
  putchar('0' + r % 10);
  putchar('\n');
  return r;
}
