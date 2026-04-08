extern int getchar(void);
extern int putchar(int c);

static volatile int pegs[3][4];
static volatile int heights[3];

static void print_str(const char *s) {
  while (*s) {
    putchar(*s);
    s++;
  }
}

static void print_uint(unsigned n) {
  if (n >= 10u)
    print_uint(n / 10u);
  putchar((int)('0' + (n % 10u)));
}

static void newline(void) { putchar('\n'); }

static void print_help(void) {
  print_str("H4 13=1->3 Q=q");
  newline();
}

static void reset_game(void) {
  heights[0] = 4;
  heights[1] = 0;
  heights[2] = 0;
  pegs[0][0] = 4;
  pegs[0][1] = 3;
  pegs[0][2] = 2;
  pegs[0][3] = 1;
}

static void print_peg(int peg) {
  int i;
  if (heights[peg] == 0) {
    putchar('.');
    return;
  }
  for (i = 0; i < heights[peg]; i++)
    putchar('0' + pegs[peg][i]);
}

static void print_board(void) {
  print_peg(0);
  putchar('|');
  print_peg(1);
  putchar('|');
  print_peg(2);
  newline();
}

static int solved(void) { return heights[2] == 4; }

static int read_peg(void) {
  for (;;) {
    int ch = getchar();
    if (ch <= 0 || ch == 'q' || ch == 'Q')
      return -1;
    if (ch >= '1' && ch <= '3')
      return ch - '1';
  }
}

static int try_move(int from, int to) {
  int disk;

  if (from == to || heights[from] == 0)
    return 0;

  disk = pegs[from][heights[from] - 1];
  if (heights[to] != 0 && pegs[to][heights[to] - 1] < disk)
    return 0;

  heights[from]--;
  pegs[to][heights[to]] = disk;
  heights[to]++;
  return 1;
}

int _start(void) {
  int moves = 0;
  int faults = 0;

  reset_game();
  print_help();
  print_board();

  while (moves < 31) {
    int from;
    int to;

    from = read_peg();
    if (from < 0) {
      print_str("Q");
      newline();
      return 0x100 + moves;
    }
    putchar('1' + from);
    print_str(" -> ");

    to = read_peg();
    if (to < 0) {
      print_str("Q");
      newline();
      return 0x100 + moves;
    }
    putchar('1' + to);
    newline();

    if (!try_move(from, to)) {
      faults++;
      print_str("X");
      newline();
      if (faults >= 6) {
        print_str("L");
        newline();
        return 0x200 + faults;
      }
      continue;
    }

    moves++;
    print_board();

    if (solved()) {
      print_str("WIN ");
      print_uint((unsigned)moves);
      newline();
      return 0x400 + moves;
    }
  }

  print_str("T");
  newline();
  return 0x300 + moves;
}
