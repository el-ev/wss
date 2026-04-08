extern int getchar(void);
extern int putchar(int);

int _start() {
  const int answer[5] = {'h', 'o', 'r', 's', 'e'};
  int guess[5];
  int attempt, i, c;

  const char *title = "HORSLE\nGuess the 5-letter word in 6 tries.\n";
  for (i = 0; title[i] != '\0'; ++i)
    putchar(title[i]);

  for (attempt = 0; attempt < 6; ++attempt) {
    const char *prompt = "> ";
    for (i = 0; prompt[i] != '\0'; ++i)
      putchar(prompt[i]);

    i = 0;
    while (i < 5) {
      c = getchar();
      if (c < 0)
        return 0;
      if (c == '\n' || c == '\r')
        continue;
      if (c >= 'A' && c <= 'Z')
        c = c - 'A' + 'a';
      guess[i++] = c;
      putchar(c);
    }

    while (1) {
      c = getchar();
      if (c < 0 || c == '\n')
        break;
    }

    putchar('\n');
    putchar(' ');
    putchar(' ');
    {
      int win = 1;
      for (i = 0; i < 5; ++i) {
        if (guess[i] == answer[i]) {
          putchar('G');
        } else {
          int j, found = 0;
          win = 0;
          for (j = 0; j < 5; ++j) {
            if (guess[i] == answer[j]) {
              found = 1;
              break;
            }
          }
          putchar(found ? 'Y' : '.');
        }
      }
      putchar('\n');

      if (win) {
        const char *msg = "WIN\n";
        for (i = 0; msg[i] != '\0'; ++i)
          putchar(msg[i]);
        return 0;
      }
    }
  }

  {
    const char *lose = "LOSE: HORSE\n";
    for (i = 0; lose[i] != '\0'; ++i)
      putchar(lose[i]);
  }

  return 0;
}