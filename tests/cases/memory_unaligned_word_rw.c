// memory_unaligned_word_rw.c
int _start(void) {
  volatile unsigned int *p = (volatile unsigned int *)(unsigned int)1;
  *p = 0x12345678u;
  return (int)(*p);
}
