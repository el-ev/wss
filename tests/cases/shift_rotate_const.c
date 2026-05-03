// shift_rotate_const.c
__attribute((optnone))
int _start(void) {
  volatile unsigned int x = 0x91a2b3c4u;
  volatile int sx = (int)x;

  unsigned int shl = x << 5;
  unsigned int shr_u = x >> 5;
  unsigned int shr_s = (unsigned int)(sx >> 5);
  unsigned int rotl = (x << 5) | (x >> 27);
  unsigned int rotr = (x >> 5) | (x << 27);
  unsigned int div_pow2 = x / 8u;

  return (int)(shl ^ shr_u ^ shr_s ^ rotl ^ rotr ^ div_pow2);
}
