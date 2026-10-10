// bool_nary_chain.c: chains of && / || over comparisons, negations and
// parity tests, consumed both arithmetically and as select conditions.
static volatile int v[] = {3, -7, 12, 0, 255, -1, 64, 9};

int _start(void) {
  int acc = 0;
  for (int i = 0; i < 8; i++) {
    int a = v[i], b = v[(i + 1) & 7], c = v[(i + 3) & 7];
    int t1 = (a > 0 && b > 0 && c > 0);
    int t2 = (a == 0 || b < -5 || (c & 1));
    int t3 = ((a > b) && (b > c)) || ((a < b) && (b < c));
    int t4 = !(a == 0) && !(b == 0) && (c != 0);
    int t5 = ((unsigned)a < 10u) || ((unsigned)b < 10u);
    acc = acc * 3 + t1 + (t2 << 1) + (t3 << 2) + (t4 << 3) + (t5 << 4);
    acc += (t1 && t2) ? a : (t3 || t5) ? b : c;
  }
  return acc;
}
