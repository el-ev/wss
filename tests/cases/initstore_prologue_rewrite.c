// initstore_prologue_rewrite.c: stores of a byte's initial value are no-ops
// only while the byte is provably untouched. Stores after a call, after a
// conditional write, or of a different value must survive.
static volatile int g1 = 5;
static volatile int g2 = 0;
static volatile int g3 = 77;
static volatile int g4 = 9;
static volatile unsigned char buf[8] = {1, 2, 3, 4, 5, 6, 7, 8};
static volatile int flag = 1;

__attribute__((noinline)) static void bump(void) {
  g1 += 10;
  buf[3] = 0x44;
}

int _start(void) {
  g1 = 5;     // same as initial image: droppable
  g2 = 0;     // same as initial image: droppable
  buf[0] = 1; // droppable
  g3 = 78;    // differs: must be kept
  if (flag) {
    g4 = 1234;
  }
  g4 = 9; // conditionally modified above: must be kept
  bump();
  g1 = 5;     // modified by bump(): must be kept
  buf[3] = 4; // modified by bump(): must be kept
  g2 = 0;     // untouched, still a no-op either way
  return g1 + g2 + g3 + g4 + buf[0] + buf[3]; // 5+0+78+9+1+4 = 97
}
