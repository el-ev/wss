// meminc_counters.c: load; add/sub immediate; store on memory-resident
// counters. Each update is a candidate for the fused MemInc32 form, and the
// initial values are chosen so carries and borrows ripple across byte lanes.
static volatile unsigned a = 0xfeu;
static volatile unsigned b = 0xffffu;
static volatile unsigned c = 0xffffffffu;
static volatile unsigned d = 0u;
static volatile unsigned e = 0x00ffffffu;
static volatile unsigned f = 0x12345678u;
static volatile int g = -1;
static volatile unsigned cnt = 250u;

int _start(void) {
  for (int i = 0; i < 3; i++) {
    a += 1;
    b += 1;
    c += 1;
    d -= 1;
    e += 0x100;
    f += 0x01010101;
    g -= 0x7f;
  }
  while (cnt < 262u) {
    cnt += 1;
  }
  return (int)(a ^ b ^ c ^ d ^ e ^ f ^ (unsigned)g ^ (cnt << 20));
}
