// loop_continue_nested.c
__attribute__((optnone)) int _start(void) {
    int result = 0;

    for (int i = 0; i < 5; i++) {
        for (int j = 0; j < 5; j++) {
            if (i == j) break;
            result += i * j;
        }
    }

    for (int i = 0; i < 4; i++) {
        int k = 0;
        while (k < 3) {
            k++;
            if (k == 2) continue;
            result += i + k;
        }
    }

    return result;
}
