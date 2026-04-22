typedef unsigned long long u64;

__attribute__((optnone)) long long _start(void) {
    volatile u64 a = 0x8000000000000001ULL;
    volatile u64 b = 0x7FFFFFFFFFFFFFFFULL;
    volatile u64 c = 0x0000000100000000ULL;
    volatile u64 d = 0x0000000100000000ULL;
    volatile u64 z = 0;

    u64 acc = 0;
    acc = acc + acc + (u64)(a == b);
    acc = acc + acc + (u64)(a != b);
    acc = acc + acc + (u64)(a < b);
    acc = acc + acc + (u64)(a > b);
    acc = acc + acc + (u64)(a <= b);
    acc = acc + acc + (u64)(a >= b);
    acc = acc + acc + (u64)(c == d);
    acc = acc + acc + (u64)(c != d);
    acc = acc + acc + (u64)(!z);
    acc = acc + acc + (u64)(!a);
    return (long long)(acc + a);
}
