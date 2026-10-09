// Build:
//   clang++ --target=wasm32 -Os -nostdlib -fno-exceptions -fno-rtti \
//     -mno-simd128 -mno-bulk-memory -mno-multivalue \
//     -Wl,--gc-sections -Wl,--no-stack-first -Wl,--allow-undefined \
//     -Wl,-z,stack-size=512 -Wl,--compress-relocations -Wl,--strip-all \
//     -Wl,--global-base=4 -Wl,--export=_start \
//     -o mt19937.wasm examples/mt19937.cpp
//   cargo run --release -- mt19937.wasm -o examples/mt19937.html --memory-bytes 2560 --no-memory-trap

extern "C" int getchar(void);
extern "C" int putchar(int c);

using u32 = unsigned int;

namespace io {

__attribute__((noinline))
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
  for (int n = 0; n < 8; ++n) {
    putchar(hex_digit(v >> 28));
    v <<= 4;
  }
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
    fill_ = state_ + 1;
    cur_ = state_;
  }

  u32 next() {
    u32 *p = cur_;
    u32 *q = p + 1;
    if (q == state_ + N)
      q = state_;
    // Twisting word p reads the old words p+1 and p+M (mod N), so seed up to
    // the furthest one. Seeding finishes before any wrap-around read.
    u32 *r = p + M;
    u32 *need = r;
    if (r >= state_ + N) {
      r -= N;
      need = state_ + N - 1;
    }
    u32 *f = fill_;
    if (f <= need) {
      u32 i = static_cast<u32>(f - state_);
      u32 prev = f[-1];
      do {
        prev = 1812433253u * (prev ^ (prev >> 30)) + i++;
        *f = prev;
      } while (++f <= need);
      fill_ = f;
    }
    u32 y = (*p & UPPER) | (*q & LOWER);
    u32 x = *r ^ (y >> 1);
    if (y & 1u)
      x ^= A;
    *p = x;
    cur_ = q;

    x ^= x >> 11;
    x ^= (x << 7) & 0x9d2c5680u;
    x ^= (x << 15) & 0xefc60000u;
    x ^= x >> 18;
    return x;
  }

private:
  static constexpr u32 N = 624u;
  static constexpr u32 M = 397u;
  static constexpr u32 UPPER = 0x80000000u;
  static constexpr u32 LOWER = 0x7fffffffu;
  static constexpr u32 A = 0x9908b0dfu;

  u32 state_[N];
  u32 *fill_;
  u32 *cur_;
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
