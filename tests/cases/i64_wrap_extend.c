long long _start(void) {
    volatile long long big = 0xDEADBEEF12345678LL;
    volatile int neg = -1;
    volatile unsigned int pos = 0x80000000u;

    int wrapped = (int)big;
    long long ext_s = (long long)neg;
    long long ext_u = (long long)(unsigned long long)pos;
    return (long long)wrapped + ext_s + ext_u;
}
