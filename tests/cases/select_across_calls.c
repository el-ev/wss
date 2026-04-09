static volatile int glob = 100;

static int id(int x) {
    return x;
}

static int select_min(int a, int b) {
    return (a < b) ? a : b;
}

static int select_max(int a, int b) {
    return (a > b) ? a : b;
}

__attribute__((optnone)) int _start(void) {
    volatile int x = id(50);
    volatile int y = id(30);
    volatile int z = id(70);

    int m1 = select_min(x, y);
    int m2 = select_max(x, z);
    int m3 = select_min(y, z);

    int result = m1 + m2 + m3 + id(glob);
    return result;
}
