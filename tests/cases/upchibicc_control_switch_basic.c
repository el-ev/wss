// Upstream reference: rui314/chibicc@90d1f7f test/control.c
static int pick_value(int x) {
  int out = 0;
  switch (x) {
  case 0:
    out = 5;
    break;
  case 1:
    out = 6;
    break;
  case 2:
    out = 7;
    break;
  default:
    break;
  }
  return out;
}

static int fallthrough_switch(void) {
  int out = 0;
  switch (1) {
  case 0:
    out = 9;
  case 1:
    out = 2;
  case 2:
    out = 4;
  }
  return out;
}

static int signed_case_switch(void) {
  int out = 0;
  switch (-1) {
  case 0xffffffff:
    out = 3;
    break;
  default:
    break;
  }
  return out;
}

int _start(void) {
  int score = 0;
  score += pick_value(0) * 1;
  score += pick_value(1) * 2;
  score += pick_value(2) * 3;
  score += pick_value(3) * 4;
  score += fallthrough_switch() * 5;
  score += signed_case_switch() * 6;
  return score;
}
