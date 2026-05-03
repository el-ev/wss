// memory_partial_store_merge.c
int _start(void) {
  volatile unsigned int *w = (volatile unsigned int *)(unsigned int)0x30;
  volatile unsigned short *h = (volatile unsigned short *)(unsigned int)0x32;

  w[0] = 0x11223344u;
  h[0] = (unsigned short)0xa1b2;

  return (int)w[0];
}
