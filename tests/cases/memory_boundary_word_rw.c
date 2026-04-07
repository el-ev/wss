int _start(void) {
  volatile unsigned int *p = (volatile unsigned int *)(unsigned int)1020;
  *p = 0x89abcdefu;
  return (int)(*p);
}
