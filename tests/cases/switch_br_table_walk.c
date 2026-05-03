// switch_br_table_walk.c
static volatile unsigned seed = 9;

__attribute__((optnone, noinline)) static int jumpy(unsigned x) {
  int acc = 100;
  switch (x) {
    case 0:
      acc += 1;
      break;
    case 1:
      acc += 3;
      break;
    case 2:
      acc += 5;
      break;
    case 3:
      acc += 7;
      break;
    case 4:
      acc += 11;
      break;
    case 5:
      acc += 13;
      break;
    case 6:
      acc += 17;
      break;
    case 7:
      acc += 19;
      break;
    case 8:
      acc += 23;
      break;
    case 9:
      acc += 29;
      break;
    case 10:
      acc += 31;
      break;
    case 11:
      acc += 37;
      break;
    case 12:
      acc += 41;
      break;
    case 13:
      acc += 43;
      break;
    case 14:
      acc += 47;
      break;
    case 15:
      acc += 53;
      break;
    default:
      acc += 97;
      break;
  }
  return acc;
}

int _start(void) {
  unsigned x = seed & 15u;
  seed = x + 1u;
  return jumpy(x);
}
