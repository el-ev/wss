// carry_chain_edges.c: add/sub/mul/neg on operands sitting exactly at byte
// lane boundaries, so every carry and borrow has to propagate through all
// four lanes of the byte-sliced datapath.
static volatile unsigned vals[] = {0xffu,       0xffffu,     0xffffffu,   0xffffffffu,
                                   0x100u,      0x10000u,    0x1000000u,  0u,
                                   0x80000000u, 0x7fffffffu, 0xff00ff00u, 0x00ff00ffu};

int _start(void) {
  unsigned acc = 0;
  for (int i = 0; i < 12; i++) {
    unsigned x = vals[i];
    acc = ((acc << 7) | (acc >> 25)) ^ (x + 1u) ^ ((x - 1u) * 3u) ^ (x * x) ^
          (x + 0x01010101u) ^ (x - 0x10101010u) ^ (0u - x) ^ (x * 0xffffu) ^
          (x * 0x10001u);
  }
  return (int)acc;
}
