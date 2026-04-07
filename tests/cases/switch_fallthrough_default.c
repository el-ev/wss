static int classify(int x) {
  int out = 0;
  switch (x) {
    case 0:
      out += 1;
    case 1:
      out += 4;
      break;
    case 2:
      out += 8;
    case 3:
      out += 16;
    default:
      out += 32;
      break;
  }
  return out;
}

int _start(void) {
  return classify(0) + classify(1) + classify(2) + classify(3) + classify(9);
  // 5 + 4 + 56 + 48 + 32 = 145 (0x91)
}
