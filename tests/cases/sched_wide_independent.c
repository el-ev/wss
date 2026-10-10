// sched_wide_independent.c: many independent dependency chains in one block
// followed by a reduction, so the scheduler can pack instructions wide
// without reordering across data dependencies incorrectly.
static volatile unsigned in[16] = {0x1u,     0x80u,  0xffu,   0x100u,
                                   0x1234u,  0x8000u, 0xffffu, 0x10000u,
                                   0xabcdefu, 0x800000u, 0xffffffu, 0x1000000u,
                                   0x7fffffffu, 0x80000000u, 0xfffffffeu, 0xffffffffu};

int _start(void) {
  unsigned c0 = (in[0] + 3u) ^ 0x55u, c1 = (in[1] - 7u) | 0x3u, c2 = in[2] * 9u,
           c3 = in[3] ^ 0xf0f0u;
  unsigned c4 = in[4] << 3, c5 = in[5] >> 2, c6 = (in[6] & 0xff0fu) + 1u,
           c7 = in[7] - 1u;
  unsigned c8 = ~in[8], c9 = in[9] + in[9], c10 = in[10] * 3u, c11 = in[11] | 1u;
  unsigned c12 = in[12] + 1u, c13 = in[13] - 1u, c14 = in[14] ^ in[15],
           c15 = in[15] >> 31;
  c0 = (c0 << 1) + 1u;
  c1 = c1 * 5u;
  c2 = c2 ^ (c2 >> 4);
  c3 = c3 + 0x0101u;
  c4 = c4 | (c4 >> 8);
  c5 = c5 - 0x11u;
  c6 = c6 << 4;
  c7 = c7 & 0xff00ffu;
  c8 = c8 >> 3;
  c9 = c9 ^ 0xdeadu;
  c10 = c10 + 0x100u;
  c11 = c11 << 2;
  c12 = c12 ^ (c12 << 1);
  c13 = c13 + 0x7u;
  c14 = c14 * 2u;
  c15 = c15 + 0x40u;
  unsigned s = c0 ^ c1 ^ c2 ^ c3;
  unsigned t = c4 + c5 + c6 + c7;
  unsigned u = c8 | c9 | c10 | c11;
  unsigned w = c12 - c13 - c14 - c15;
  return (int)((s + t) ^ (u - w));
}
