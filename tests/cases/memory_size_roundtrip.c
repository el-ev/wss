// memory_size_roundtrip.c
static volatile int sink;

int _start(void) {
  int pages = __builtin_wasm_memory_size(0);
  volatile unsigned char *p = (volatile unsigned char *)0;

  p[12] = (unsigned char)(pages + 3);
  p[13] = (unsigned char)(pages + 7);

  int x = (unsigned char)p[12];
  int y = (signed char)p[13];

  sink = x ^ y;
  return sink + pages + 44;
}
