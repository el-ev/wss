extern int getchar(void);
extern int putchar(int c);

static int hp = 14;
static int max_hp = 14;
static int gold = 4;
static int potions = 1;
static int sword = 0;
static int wins = 0;
static int won = 0;

static void print_str(const char *s) {
  while (*s) {
    putchar(*s);
    s++;
  }
}

static void print_digit(int n) {
  switch (n) {
  case 0:
    putchar('0');
    break;
  case 1:
    putchar('1');
    break;
  case 2:
    putchar('2');
    break;
  case 3:
    putchar('3');
    break;
  case 4:
    putchar('4');
    break;
  case 5:
    putchar('5');
    break;
  case 6:
    putchar('6');
    break;
  case 7:
    putchar('7');
    break;
  case 8:
    putchar('8');
    break;
  default:
    putchar('9');
    break;
  }
}

static void print_num(int n) {
  if (n < 10) {
    print_digit(n);
  } else if (n < 20) {
    putchar('1');
    print_digit(n - 10);
  } else if (n < 30) {
    putchar('2');
    print_digit(n - 20);
  } else {
    putchar('3');
    print_digit(n - 30);
  }
}

static int read_key(void) {
  int ch = getchar();
  while (ch == '\n' || ch == '\r') {
    ch = getchar();
  }
  if (ch < 0) {
    return 'q';
  }
  return ch;
}

static void print_status(void) {
  print_str("\nHP ");
  print_num(hp);
  putchar('/');
  print_num(max_hp);
  print_str("  Gold ");
  print_num(gold);
  print_str("  Pots ");
  print_num(potions);
  print_str("  Sw ");
  print_num(sword);
  putchar('\n');
}

static void print_enemy_name(int kind) {
  if (kind == 0) {
    print_str("Slime");
  } else if (kind == 1) {
    print_str("Goblin");
  } else {
    print_str("Dragon");
  }
}

static int fight(int kind) {
  int enemy_hp;
  int enemy_atk;
  int reward;
  int turn = 0;

  if (kind == 0) {
    enemy_hp = 7;
    enemy_atk = 2;
    reward = 3;
  } else if (kind == 1) {
    enemy_hp = 11;
    enemy_atk = 3;
    reward = 5;
  } else {
    enemy_hp = 18;
    enemy_atk = 4;
    reward = 12;
  }

  print_str("\nA ");
  print_enemy_name(kind);
  print_str(" appears.\n");

  while (hp > 0 && enemy_hp > 0) {
    int ch;
    int damage;

    print_str("Enemy ");
    print_num(enemy_hp);
    print_str("  You ");
    print_num(hp);
    putchar('\n');
    print_str("A atk  P pot");
    if (kind != 2) {
      print_str("  R run");
    }
    putchar('\n');

    ch = read_key();
    switch (ch) {
    case 'A':
    case 'a':
      damage = 3 + sword + (turn & 1);
      enemy_hp -= damage;
      print_str("Hit ");
      print_num(damage);
      print_str(".\n");
      break;
    case 'P':
    case 'p':
      if (potions <= 0) {
        print_str("No potions.\n");
        continue;
      }
      potions -= 1;
      damage = 6;
      if (hp + damage > max_hp) {
        damage = max_hp - hp;
      }
      hp += damage;
      print_str("Heal ");
      print_num(damage);
      print_str(".\n");
      break;
    case 'R':
    case 'r':
      if (kind != 2) {
        print_str("You run.\n");
        return 0;
      }
      print_str("Choose A or P.\n");
      continue;
    default:
      if (kind != 2) {
        print_str("Choose A, P, or R.\n");
      } else {
        print_str("Choose A or P.\n");
      }
      continue;
    }

    if (enemy_hp <= 0) {
      break;
    }

    damage = enemy_atk - (turn & 1);
    if (kind == 2 && enemy_hp <= 8) {
      damage += 1;
    }
    if (damage < 1) {
      damage = 1;
    }
    hp -= damage;
    print_str("The ");
    print_enemy_name(kind);
    print_str(" hits ");
    print_num(damage);
    print_str(".\n");
    turn += 1;
  }

  if (hp <= 0) {
    print_str("You fall in battle.\n");
    return -1;
  }

  gold += reward;
  print_str("Win +");
  print_num(reward);
  print_str(" gold.\n");

  if (kind == 2) {
    won = 1;
  } else {
    wins += 1;
    if (wins == 2) {
      max_hp = 18;
      hp = max_hp;
      print_str("Dragon gate opens.\n");
      print_str("Max HP ");
      print_num(max_hp);
      print_str(".\n");
    }
  }

  return 1;
}

static void shop(void) {
  int ch;

  print_str("\nShop\n");
  print_str("B sword 7\n");
  print_str("P potion 3\n");
  print_str("L leave\n");
  ch = read_key();
  switch (ch) {
  case 'B':
  case 'b':
    if (sword) {
      print_str("You have a sword.\n");
    } else if (gold < 7) {
      print_str("Need more gold.\n");
    } else {
      gold -= 7;
      sword = 1;
      print_str("Bought sword.\n");
    }
    break;
  case 'P':
  case 'p':
    if (gold < 3) {
      print_str("Need more gold.\n");
    } else {
      gold -= 3;
      potions += 1;
      print_str("Bought potion.\n");
    }
    break;
  default:
    print_str("Leave shop.\n");
    break;
  }
}

int _start(void) {
  print_str("Tiny Crown RPG\n");
  print_str("Type a letter, then Enter.\n");
  print_str("Win 2 fights, then beat the dragon.\n");

  while (hp > 0 && !won) {
    int ch;

    print_status();
    print_str("F fight\n");
    print_str("S shop\n");
    print_str("I inn 2\n");
    print_str("D dragon\n");
    print_str("Q quit\n");

    ch = read_key();
    switch (ch) {
    case 'F':
    case 'f':
      if (wins == 0) {
        fight(0);
      } else {
        fight(1);
      }
      break;
    case 'S':
    case 's':
      shop();
      break;
    case 'I':
    case 'i':
      if (hp == max_hp) {
        print_str("Already rested.\n");
      } else if (gold < 2) {
        print_str("Need 2 gold.\n");
      } else {
        gold -= 2;
        hp = max_hp;
        print_str("You rest.\n");
      }
      break;
    case 'D':
    case 'd':
      if (wins < 2) {
        print_str("Dragon gate sealed.\n");
      } else {
        fight(2);
      }
      break;
    case 'Q':
    case 'q':
      print_str("You leave.\n");
      return 0;
    default:
      print_str("Choose F, S, I, D, or Q.\n");
      break;
    }
  }

  if (won) {
    print_str("You win the Tiny Crown.\n");
    return 0;
  }

  print_str("Another hero must try.\n");
  return 1;
}
