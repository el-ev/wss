// up8cc_pointer_ptrdiff_lp64.c
// Upstream reference: rui314/8cc@b480958 test/pointer.c (subtract)
// This case intentionally encodes LP64 assumptions from upstream.
int _start(void) {
  char *p = "abcdefg";
  char *q = p + 5;
  return (int)sizeof(q - p) * 100 + (int)(q - p);
}
