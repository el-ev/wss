// addsub_i32_stress.c
static volatile unsigned a0 = 0x00000000u;
static volatile unsigned b0 = 0x00000000u;
static volatile unsigned a1 = 0x00000001u;
static volatile unsigned b1 = 0xffffffffu;
static volatile unsigned a2 = 0xffffffffu;
static volatile unsigned b2 = 0x00000001u;
static volatile unsigned a3 = 0x7fffffffu;
static volatile unsigned b3 = 0x00000001u;
static volatile unsigned a4 = 0x80000000u;
static volatile unsigned b4 = 0x00000001u;
static volatile unsigned a5 = 0x80000000u;
static volatile unsigned b5 = 0x7fffffffu;
static volatile unsigned a6 = 0x12345678u;
static volatile unsigned b6 = 0x9abcdef0u;
static volatile unsigned a7 = 0x00ff00ffu;
static volatile unsigned b7 = 0xff00ff00u;
static volatile unsigned a8 = 0x01010101u;
static volatile unsigned b8 = 0xfefefefeu;
static volatile unsigned a9 = 0xdeadbeefu;
static volatile unsigned b9 = 0x10203040u;
static volatile unsigned a10 = 0x13579bdfu;
static volatile unsigned b10 = 0x2468ace0u;
static volatile unsigned a11 = 0xffff0000u;
static volatile unsigned b11 = 0x0001ffffu;

#define STEP(a, b, rot)                                                        \
  do {                                                                         \
    unsigned add = (a) + (b);                                                 \
    unsigned sub = (a) - (b);                                                 \
    acc = ((acc ^ add) + sub) ^ ((add >> (rot)) | (sub << (8 - (rot))));      \
  } while (0)

int _start(void) {
  unsigned acc = 0x6d2b79f5u;
  STEP(a0, b0, 1);
  STEP(a1, b1, 2);
  STEP(a2, b2, 3);
  STEP(a3, b3, 4);
  STEP(a4, b4, 5);
  STEP(a5, b5, 6);
  STEP(a6, b6, 7);
  STEP(a7, b7, 1);
  STEP(a8, b8, 2);
  STEP(a9, b9, 3);
  STEP(a10, b10, 4);
  STEP(a11, b11, 5);
  return (int)acc;
}

