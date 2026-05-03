// mixed_arith_bitwise.c
__attribute__((optnone)) int _start(void) {
    volatile int x = 5;
    volatile int y = 3;
    volatile int z = 7;

    int a = (x + y) * z;
    int b = (x ^ y) | z;
    int c = (x & y) ^ z;
    int d = (x << 2) + (y >> 1);
    int e = ~(x ^ y) & 0xFF;
    int f = ((x + 1) * 2) ^ ((y - 1) * 3);

    return (a + b + c + d + e + f) & 0xFFFFFFFF;
}
