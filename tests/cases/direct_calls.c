// direct_calls.c
static int mul3(int x) { return x * 3; }
static int add5(int x) { return x + 5; }
static int pipeline(int x) { return add5(mul3(x)); }

int _start(void) { return pipeline(7); } // 26 (0x1a)

