__attribute__((noinline)) static int fact(int n) {
    if (n <= 1) return 1;
    return n * fact(n - 1);
}

__attribute__((optnone)) int _start(void) {
    int f5 = fact(5);
    int f6 = fact(6);

    return (f5 + f6) & 0xFFFF;
}
