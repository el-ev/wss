// memory_rw.c
static int data[8];

int _start(void) {
  for (int i = 0; i < 8; i++) {
    data[i] = i * i + 3;
  }
  int r = data[0] + data[3] + data[7]; // 3 + 12 + 52 = 67
  data[4] = r;
  return data[4];
}

