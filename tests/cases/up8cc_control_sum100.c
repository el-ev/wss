// Upstream reference: rui314/8cc@b480958 test/control.c (test_while)
int _start(void) {
  int acc = 0;
  int i = 0;
  while (i <= 100)
    acc = acc + i++;
  return acc;
}
