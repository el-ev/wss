__attribute__((optnone)) int _start(void) {
    int result = 0;

    for (int i = 0; i < 10; i++) {
        int j = 0;
        while (j < 5) {
            if (i == 7 && j == 3) {
                result = i * 100 + j;
                goto done;
            }
            j++;
        }
    }
done:
    return result;
}
