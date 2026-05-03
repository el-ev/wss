// division_large_unsigned_callconst.c
__attribute__((noinline)) unsigned int get_a(void) { return 0xf1234567u; }
__attribute__((noinline)) unsigned int get_b(void) { return 0x00fedcbau; }
int _start(void) {
  unsigned int q = get_a() / get_b();
  unsigned int r = get_a() % get_b();
  return (int)(((q << 24) ^ r ^ 0x00abcdefu));
}
