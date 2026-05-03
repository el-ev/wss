// up8cc_type_pointer_size_lp64.c
// Upstream references:
// - rui314/8cc@b480958 test/type.c (pointer-size assumptions)
// - rui314/8cc@b480958 test/sizeof.c (LP64 long/pointer widths)
// This case intentionally encodes LP64 assumptions from upstream.
int _start(void) {
  return (int)sizeof(long) * 100 + (int)sizeof(void *);
}
