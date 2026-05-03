// memory_byte_patterns.c
__attribute__((optnone)) int _start(void) {
    volatile unsigned char *p = (volatile unsigned char *)(unsigned int)0x100;

    p[0] = 0x12;
    p[1] = 0x34;
    p[2] = 0x56;
    p[3] = 0x78;

    int val = p[0] | (p[1] << 8) | (p[2] << 16) | (p[3] << 24);

    p[4] = 0xAA;
    p[5] = 0xBB;

    int val2 = p[4] | (p[5] << 8);

    return val + val2;
}
