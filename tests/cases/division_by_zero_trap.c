static volatile int zero = 0;

int _start(void) { return 123 / zero; }

