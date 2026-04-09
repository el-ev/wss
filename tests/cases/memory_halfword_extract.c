__attribute__((optnone)) int _start(void) {
    volatile unsigned int *p = (volatile unsigned int *)(unsigned int)0x80;

    p[0] = 0xABCD1234u;
    p[1] = 0x5678DCBAu;

    unsigned int val = p[0];
    unsigned int half = (val >> 16) & 0xFFFF;
    unsigned int byte1 = (val >> 8) & 0xFF;
    unsigned int byte2 = val & 0xFF;

    p[2] = byte1 | (byte2 << 8) | (half << 16);

    return (int)p[2];
}
