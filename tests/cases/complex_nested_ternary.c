// complex_nested_ternary.c
__attribute__((optnone)) int _start(void) {
    volatile int x = 1;
    volatile int y = 2;
    volatile int z = 3;

    int a = (x < y) ? ((y < z) ? y : ((x < z) ? z : x)) : ((x < z) ? x : ((y < z) ? z : y));
    int b = (x > y) ? ((y > z) ? y : ((x > z) ? z : x)) : ((x > z) ? x : ((y > z) ? z : y));

    int c = (x & y) | z;
    int d = (x | y) & z;
    int e = (x ^ y) ^ z;

    return a + b + c + d + e;
}
