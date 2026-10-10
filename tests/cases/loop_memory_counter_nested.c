// loop_memory_counter_nested.c: nested loops whose counters live in memory
// and are compared/incremented in place. Outer counter crosses the byte
// boundary at 256; total starts near the top of the u32 range and wraps.
static volatile unsigned cnt = 250u;
static volatile unsigned inner = 0u;
static volatile unsigned total = 0xfffffff0u;

int _start(void) {
  while (cnt < 262u) {
    cnt += 1;
    inner = 0;
    while (inner < 3u) {
      inner += 1;
      total += 1;
    }
  }
  // cnt = 262, inner = 3, total = 0xfffffff0 + 36 = 0x14 -> 262 ^ 3 ^ 0x14 = 273
  return (int)(cnt ^ inner ^ total);
}
