__attribute__((noinline)) long long plus16(long long x) {
    return x + 16;
}

long long _start(void) {
    return plus16(0x1122334455667788LL);
}
