// narrow_store_truncation.c: 8- and 16-bit stores of values with high bits
// set, read back through unsigned and sign-extending loads.
static volatile unsigned char b[4];
static volatile unsigned short h[4];
static volatile signed char sb[2];
static volatile short sh[2];
static volatile unsigned wide[] = {0x12345678u, 0xfffffff0u, 0x80u,  0x8000u,
                                   0x7fu,       0x7fffu,     0x1ffu, 0x1ffffu};

int _start(void) {
  b[0] = (unsigned char)wide[0];
  b[1] = (unsigned char)wide[1];
  b[2] = (unsigned char)wide[2];
  b[3] = (unsigned char)wide[6];
  h[0] = (unsigned short)wide[0];
  h[1] = (unsigned short)wide[1];
  h[2] = (unsigned short)wide[3];
  h[3] = (unsigned short)wide[7];
  sb[0] = (signed char)wide[2];
  sb[1] = (signed char)wide[4];
  sh[0] = (short)wide[3];
  sh[1] = (short)wide[5];
  int acc = b[0] + b[1] * 3 + b[2] * 5 + b[3] * 7;
  acc ^= h[0] + h[1] * 3 + h[2] * 5 + h[3] * 7;
  acc += sb[0] * 1000 + sb[1] * 1001;
  acc += sh[0] * 3 + sh[1] * 5;
  return acc;
}
