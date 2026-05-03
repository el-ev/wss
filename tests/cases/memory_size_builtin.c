// memory_size_builtin.c
int _start(void) {
  int pages = __builtin_wasm_memory_size(0);
  return pages + 41;
}
