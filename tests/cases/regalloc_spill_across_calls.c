// regalloc_spill_across_calls.c: two dozen values loaded before a pair of
// calls and all consumed afterwards, so every one of them is live across
// the calls and must be preserved by the register allocator.
static volatile int src[24] = {1,  -2, 3,  -4, 5,  -6, 7,  -8, 9,  -10, 11, -12,
                               13, 14, 15, 16, 17, 18, 19, 20, 21, 22,  23, 24};

__attribute__((noinline)) static int touch(int x) { return x * 7 + 1; }

int _start(void) {
  int a0 = src[0], a1 = src[1], a2 = src[2], a3 = src[3];
  int a4 = src[4], a5 = src[5], a6 = src[6], a7 = src[7];
  int a8 = src[8], a9 = src[9], a10 = src[10], a11 = src[11];
  int a12 = src[12], a13 = src[13], a14 = src[14], a15 = src[15];
  int a16 = src[16], a17 = src[17], a18 = src[18], a19 = src[19];
  int a20 = src[20], a21 = src[21], a22 = src[22], a23 = src[23];
  int k = touch(a0 ^ a23);
  int m = touch(a5 + a17);
  int r = a0 - a1 + a2 * a3 - a4 + (a5 << 1) - (a6 >> 1) + a7;
  r ^= a8 + a9 * a10 - a11 + a12 - a13 + a14 * a15;
  r += (a16 - a17) * (a18 + a19) - a20 + a21 - a22 + a23;
  return r + k - m;
}
