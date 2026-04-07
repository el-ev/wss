// Upstream reference: rui314/8cc@b480958 test/control.c (test_if)
static volatile int g_one = 1;

static int if1(void) {
  if (g_one)
    return 'a';
  return 0;
}

static int if2(void) {
  if (0)
    return 0;
  return 'b';
}

static int if3(void) {
  if (1)
    return 'c';
  else
    return 0;
}

static int if4(void) {
  if (0)
    return 0;
  else
    return 'd';
}

static int if5(void) {
  if (g_one)
    return 'e';
  return 0;
}

static int if6(void) {
  if (0)
    return 0;
  return 'f';
}

static int if7(void) {
  if (1)
    return 'g';
  else
    return 0;
}

static int if8(void) {
  if (0)
    return 0;
  else
    return 'h';
}

static int if9(void) {
  if (0 + g_one)
    return 'i';
  return 0;
}

static int if10(void) {
  if (g_one - 1)
    return 0;
  return 'j';
}

int _start(void) {
  return if1() + if2() + if3() + if4() + if5() + if6() + if7() + if8() + if9() + if10();
}
