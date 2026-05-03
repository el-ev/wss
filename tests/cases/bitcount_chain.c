// bitcount_chain.c
__attribute__((optnone)) int _start(void) {
    volatile unsigned int x = 0x12345678u;
    volatile unsigned int y = 0x87654321u;

    unsigned int rot = ((x << 3) | (x >> 29)) ^ ((y >> 5) | (y << 27));
    unsigned int clz = __builtin_clz(x);
    unsigned int ctz = __builtin_ctz(y);
    unsigned int pop = __builtin_popcount(x & y);

    unsigned int result = (rot & 0xFFu) + ((clz + ctz + pop) & 0xFFu);
    return (int)result;
}
