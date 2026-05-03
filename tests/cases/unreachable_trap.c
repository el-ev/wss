// unreachable_trap.c
__attribute__((noreturn)) static void boom(void) { __builtin_trap(); }

int _start(void) { boom(); }

