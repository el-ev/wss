// array_memory_interop.c
__attribute__((optnone)) int _start(void) {
    volatile int data[16];
    for (int i = 0; i < 16; i++) {
        data[i] = (i * 7) & 0xFF;
    }

    int sum = 0;
    for (int i = 0; i < 16; i++) {
        sum += data[i];
    }

    volatile int *m = (volatile int *)(unsigned int)0x100;
    m[0] = data[0] | (data[1] << 8) | (data[2] << 16) | (data[3] << 24);
    m[1] = data[4] | (data[5] << 8) | (data[6] << 16) | (data[7] << 24);
    m[2] = data[8] | (data[9] << 8) | (data[10] << 16) | (data[11] << 24);
    m[3] = data[12] | (data[13] << 8) | (data[14] << 16) | (data[15] << 24);

    return (m[0] + m[1] + m[2] + m[3]) & 0xFFFFFFFF;
}
