static volatile int g_a = 10;
static volatile int g_b = 20;
static volatile int g_c = 30;

static int compute(void) {
    return g_c * 2;
}

__attribute__((optnone)) int _start(void) {
    int result = compute();
    g_a = 5;
    g_c = g_a + g_b;
    result += compute();
    g_c = g_a + g_b + 30;
    result += g_c;
    return result;
}
