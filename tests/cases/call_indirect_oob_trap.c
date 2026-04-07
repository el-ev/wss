int _start(void) {
  typedef int (*unop_t)(int);
  unop_t fn = (unop_t)(unsigned int)12345;
  return fn(7);
}
