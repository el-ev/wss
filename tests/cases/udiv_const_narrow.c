// udiv_const_narrow.c: unsigned div/rem by constants where the dividend is a
// zero-extended byte or halfword load, so only the low lanes carry
// information. Exercises the narrow-product and byte-compare simplifications.
typedef unsigned u32;

static volatile unsigned char bytes[] = {0, 9, 10, 127, 128, 255};
static volatile unsigned short halfs[] = {0, 255, 256, 999, 65535};

#define ROTMIX(acc, v) ((acc) = (((acc) << 5) | ((acc) >> 27)) ^ (u32)(v))

int _start(void) {
  u32 acc = 0;
  for (u32 i = 0; i < sizeof(bytes) / sizeof(bytes[0]); i++) {
    u32 b = bytes[i];
    ROTMIX(acc, b / 10u);
    ROTMIX(acc, (b % 10u) << 8);
    ROTMIX(acc, (b / 7u) << 16);
    ROTMIX(acc, (b % 3u) << 24);
  }
  for (u32 i = 0; i < sizeof(halfs) / sizeof(halfs[0]); i++) {
    u32 h = halfs[i];
    ROTMIX(acc, h / 10u);
    ROTMIX(acc, (h % 100u) << 12);
    ROTMIX(acc, (h / 255u) << 20);
    ROTMIX(acc, (h % 257u) << 24);
  }
  return (int)acc;
}
