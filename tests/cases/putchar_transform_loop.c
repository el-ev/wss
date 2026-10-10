// putchar_transform_loop.c: print loops whose putchar argument is a
// transformed load (uppercasing with a range test, xor), so the unroller
// has to clone the dependent instructions per slot.
extern int putchar(int c);

static volatile char msg[] = "Hello, Wasm 42!";

int _start(void) {
  unsigned x = 0;
  for (int i = 0; msg[i]; i++) {
    char c = msg[i];
    int o = (c >= 'a' && c <= 'z') ? c - 32 : c;
    putchar(o);
    x = x * 31u + (unsigned)o;
  }
  putchar('\n');
  for (int i = 0; i < 15; i++) {
    putchar(msg[i] ^ 1);
  }
  putchar('\n');
  return (int)x;
}
