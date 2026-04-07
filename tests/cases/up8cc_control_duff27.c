// Upstream reference: rui314/8cc@b480958 test/control.c (Duff's-device style switch)
int _start(void) {
  int acc = 0;
  int count = 27;

  switch (count % 8) {
  case 0:
    do {
      acc++;
  case 7:
      acc++;
  case 6:
      acc++;
  case 5:
      acc++;
  case 4:
      acc++;
  case 3:
      acc++;
  case 2:
      acc++;
  case 1:
      acc++;
    } while ((count -= 8) > 0);
  }

  return acc;
}
