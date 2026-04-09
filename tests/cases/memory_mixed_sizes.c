__attribute__((optnone)) int _start(void) {
    volatile int *mem = (volatile int *)(unsigned int)0x200;
    volatile signed short *h = (volatile signed short *)(unsigned int)0x204;
    volatile signed char *b = (volatile signed char *)(unsigned int)0x208;

    mem[0] = 0x11223344;
    h[0] = 0xAABB;
    b[0] = 0xCC;

    int v1 = mem[0];
    signed short v2 = h[0];
    signed char v3 = b[0];

    mem[1] = v1 + v2 + v3;
    h[1] = (short)(v1 ^ v2);
    b[1] = (char)(v2 | v3);

    return mem[1];
}
