extern int getchar(void);
extern int putchar(int c);

int _start(void) {
  int ch = getchar();
  putchar(ch);
  putchar('\n');
  return ch;
}
