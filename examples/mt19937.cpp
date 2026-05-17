// Build:
//   clang++ --target=wasm32 -Os -nostdlib -fno-exceptions -fno-rtti \
//     -mno-simd128 -mno-bulk-memory -mno-multivalue \
//     -Wl,--gc-sections -Wl,--no-stack-first -Wl,--allow-undefined \
//     -Wl,-z,stack-size=512 -Wl,--compress-relocations -Wl,--strip-all \
//     -Wl,--global-base=4 -Wl,--export=_start \
//     -o mt19937.wasm examples/mt19937.cpp
//   cargo run --release -- mt19937.wasm -o examples/mt19937.html --memory-bytes 3072

extern "C" int getchar(void);
extern "C" int putchar(int c);

using u32 = unsigned int;

namespace io {

void put_str(const char *s) {
  while (*s)
    putchar(*s++);
}

void put_uint(u32 n) {
  if (n >= 10u)
    put_uint(n / 10u);
  putchar(static_cast<int>('0' + (n % 10u)));
}

int hex_digit(u32 v) {
  return v < 10u ? static_cast<int>('0' + v)
                 : static_cast<int>('a' + v - 10u);
}

void put_hex32(u32 v) {
  for (int s = 28; s >= 0; s -= 4)
    putchar(hex_digit((v >> s) & 0xfu));
}

u32 read_seed() {
  u32 s = 0u;
  bool saw = false;
  for (;;) {
    int c = getchar();
    if (c < 0)
      break;
    if (c == '\n' || c == '\r') {
      if (saw)
        break;
      continue;
    }
    if (c < '0' || c > '9')
      continue;
    s = s * 10u + static_cast<u32>(c - '0');
    saw = true;
    putchar(c);
  }
  return s;
}

} // namespace io

class Mt19937 {
public:
  void seed(u32 s) {
    state_[0] = s;
    for (u32 i = 1u; i < N; ++i) {
      u32 prev = state_[i - 1u];
      state_[i] = 1812433253u * (prev ^ (prev >> 30)) + i;
    }
    idx_ = N;
  }

  u32 next() {
    if (idx_ >= N)
      generate();
    u32 y = state_[idx_++];
    y ^= y >> 11;
    y ^= (y << 7) & 0x9d2c5680u;
    y ^= (y << 15) & 0xefc60000u;
    y ^= y >> 18;
    return y;
  }

private:
  static constexpr u32 N = 624u;
  static constexpr u32 M = 397u;
  static constexpr u32 UPPER = 0x80000000u;
  static constexpr u32 LOWER = 0x7fffffffu;
  static constexpr u32 A = 0x9908b0dfu;

  void generate() {
    for (u32 i = 0u; i < N; ++i) {
      u32 y = (state_[i] & UPPER) | (state_[(i + 1u) % N] & LOWER);
      u32 next = state_[(i + M) % N] ^ (y >> 1);
      if (y & 1u)
        next ^= A;
      state_[i] = next;
    }
    idx_ = 0u;
  }

  u32 state_[N];
  u32 idx_;
};

static Mt19937 rng;

extern "C" int _start() {
  io::put_str("seed? ");
  u32 s = io::read_seed();
  putchar('\n');

  io::put_str("mt19937(");
  io::put_uint(s);
  io::put_str("):\n");

  rng.seed(s);
  for (int i = 0; i < 8; ++i) {
    io::put_hex32(rng.next());
    putchar('\n');
  }
  return 0;
}
