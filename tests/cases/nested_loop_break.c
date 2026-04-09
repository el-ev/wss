static volatile int table[8] = {0, 1, 2, 3, 4, 5, 6, 7};

__attribute__((optnone)) static int fib(int n) {
    if (n < 2) return n;
    int a = 0, b = 1;
    for (int i = 2; i <= n; i++) {
        int t = a + b;
        a = b;
        b = t;
    }
    return b;
}

__attribute__((optnone)) int _start(void) {
    int sum = 0;
    for (int i = 0; i < 8; i++) {
        sum = sum * 10 + fib(table[i]);
    }
    return sum;
}
