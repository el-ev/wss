// i64_sub_borrow.c
long long _start(void) {
    volatile long long a = 0x0000000100000000LL;
    volatile long long b = 0x0000000000000001LL;
    return a - b;
}
