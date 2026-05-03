// division_by_zero_trap.c
static volatile int zero = 0;

int _start(void) { return 123 / zero; }

