// up8cc_pointer_compare_rel.c
// Upstream reference: rui314/8cc@b480958 test/pointer.c (compare)
int _start(void) {
  char buf[8];
  char *p = buf + 2;
  int mask = 0;

  if (p == p)
    mask |= 1;
  if (p != p + 1)
    mask |= 2;
  if (p < p + 1)
    mask |= 4;
  if (p + 1 > p)
    mask |= 8;
  if (p >= p)
    mask |= 16;
  if (p <= p)
    mask |= 32;
  if (!(p >= p + 1))
    mask |= 64;
  if (!(p + 1 <= p))
    mask |= 128;

  return mask;
}
