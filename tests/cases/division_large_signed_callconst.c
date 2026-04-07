__attribute__((noinline)) int get_c(void) { return -2000000001; }
__attribute__((noinline)) int get_d(void) { return 34567; }
int _start(void) {
  int q = get_c() / get_d();
  return (int)(((unsigned int)q) ^ 0x013579bdu);
}
