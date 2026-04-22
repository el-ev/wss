static long long buf[2];

long long _start(void) {
    volatile long long a = 0xCAFEBABE01020304LL;
    volatile long long b = 0x0506070800000000LL;
    buf[0] = a;
    buf[1] = b;
    volatile long long la = buf[0];
    volatile long long lb = buf[1];
    return la + lb;
}
