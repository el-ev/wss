// Upstream reference: rui314/8cc@b480958 test/bitop.c
int _start(void) {
  int total = 0;

  total += (1 | 2);
  total += (2 | 5);
  total += (2 | 7);

  total += (1 & 2);
  total += (2 & 7);

  total += ~0;
  total += ~2;
  total += ~-1;

  total += (15 ^ 5);

  total += (1 << 4);
  total += (3 << 4);
  total += (15 >> 3);
  total += (8 >> 2);
  total += (((unsigned)-1) >> 31);

  return total;
}
