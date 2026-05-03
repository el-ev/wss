// i64_bitwise.c
long long _start(void) {
    volatile unsigned long long a = 0xFF00FF00AA55AA55ULL;
    volatile unsigned long long b = 0x00FF00FF55AA55AAULL;
    unsigned long long r_and = a & b;
    unsigned long long r_or = a | b;
    unsigned long long r_xor = a ^ b;
    return (long long)(r_and + r_or + r_xor);
}
