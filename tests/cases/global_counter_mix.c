// global_counter_mix.c
static volatile int g_counter = 0;

__attribute__((optnone)) static int count(int delta) {
    g_counter += delta;
    return g_counter;
}

__attribute__((optnone)) int _start(void) {
    int a = count(1);
    int b = count(2);
    int c = count(3);

    int x = (a < b) ? count(10) : count(20);
    int y = (b > c) ? count(30) : count(40);

    int z = count(a + b + c);

    return (g_counter * 1000) + (x * 100) + (y * 10) + z;
}
