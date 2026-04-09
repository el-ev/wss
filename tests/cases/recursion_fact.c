__attribute__((optnone)) static int fact(int n) {
    if (n <= 1) return 1;
    return n * fact(n - 1);
}

__attribute__((optnone)) static int fib(int n) {
    if (n <= 1) return n;
    return fib(n - 1) + fib(n - 2);
}

__attribute__((optnone)) int _start(void) {
    int f5 = fact(5);
    int f6 = fact(6);
    int fi5 = fib(5);
    int fi6 = fib(6);

    return (f5 + f6 + fi5 + fi6) & 0xFFFF;
}
