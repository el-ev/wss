// stlf_overlap_widths.c: store-to-load forwarding and dead-store elimination
// across mixed widths and unaligned overlaps inside one block.
static volatile unsigned char arena[32] __attribute__((aligned(4)));

int _start(void) {
  volatile unsigned *w = (volatile unsigned *)arena;
  volatile unsigned short *h = (volatile unsigned short *)arena;

  w[0] = 0xdeadbeefu; // fully overwritten below: dead
  w[0] = 0x01020304u;
  h[1] = 0xaabbu;         // high half of w[0]
  unsigned r1 = w[0];     // 0xaabb0304

  arena[5] = 0x80;
  w[1] = 0x11223344u; // overwrites arena[5]
  arena[6] = 0xfe;
  unsigned r2 = w[1]; // 0x11fe3344

  int r3 = *(volatile signed char *)(arena + 6); // -2
  int r4 = *(volatile short *)(arena + 6);       // 0x11fe

  *(volatile unsigned *)(arena + 9) = 0xcafebabeu;          // unaligned
  unsigned r5 = *(volatile unsigned *)(arena + 8);          // 0xfebabe00
  unsigned r6 = *(volatile unsigned short *)(arena + 11);   // 0xcafe

  return (int)(r1 ^ (r2 * 3u) ^ (unsigned)r3 ^ ((unsigned)r4 << 8) ^ r5 ^ r6);
}
