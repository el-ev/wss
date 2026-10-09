// Conway's Game of Life on the memory visualizer.
//
// The board is a 10x10 square near the left edge of the visualizer: one
// byte per cell, 0 = empty (black), 255 = occupied (white). The green read
// highlight marks the cursor, which starts at the center. A gray box frames
// the board; to its right, a play/pause sign shows what Enter will do and a
// spinner steps once per generation. Everything else the program keeps
// lives in the spare parts of the same buffer, and shows there too.
//
// Keys: w/a/s/d move the cursor, Space flips the cell under it, g stamps a
// glider around it, Enter starts and stops the world.
//
// Build:
//   clang --target=wasm32 -Os -nostdlib -mno-simd128 -fno-exceptions \
//     -mno-bulk-memory -mno-multivalue \
//     -Wl,--gc-sections -Wl,--no-stack-first -Wl,--allow-undefined \
//     -Wl,-z,stack-size=16 -Wl,--compress-relocations -Wl,--strip-all \
//     -Wl,--global-base=4 -Wl,--export=_start \
//     -o life.wasm examples/life.c
//   cargo run --release -- life.wasm -o examples/life.html --memory-bytes 2048 \
//     --max-phys-regs 512
// The register limit lets each step's independent work run in parallel
// (see `step`); at the default the build falls back to a slower schedule.

extern int try_getchar(void);
extern int putchar(int c);

typedef unsigned char u8;
typedef unsigned u32;

// The visualizer draws 128 bytes per row, so a 128-byte stride puts the
// rows of `board` directly under each other; AT(row, col) is a pixel's
// offset in it. Left to right: a 1-pixel margin, the gray bounding box, the
// 10x10 board inside it, another margin, the sign and spinner, then the
// neighbor counts. The live lists and a few scalars sit in the margin rows
// above and below, right of the box, and a generation's changes in the row
// below those.
#define STRIDE 128u
#define AT(row, col) ((row) * STRIDE + (col))
#define SIDE 10u
#define RING (SIDE + 2u) // the board plus the box around it
#define BOTTOM (RING + 1u) // the margin row below the box
#define ROWS (RING + 3u)   // margin, box, margin, and the change list
#define FIRST 2u         // row and column of the board's first cell
#define LAST (FIRST + SIDE - 1u)
#define NO_KEY 0xffu // low byte of try_getchar() when no key is held

#define GRAY 0x80u
#define DIM 0x20u

// Cells are tracked by a packed index `(row << 4) | col` over the box and
// the board inside it. The box never comes alive, so neighbors never leave
// it. Live lists hold the index of a cell's top-left neighbor (CORNER below
// the cell itself): its 3x3 neighborhood is then at non-negative constant
// offsets from one base address.
#define PACK_ROW 16u
#define CORNER (PACK_ROW + 1u)
#define CELL(b) (AT(1u, 1u) + ((((b) & 0xf0u) << 3) | ((b) & 0x0fu)))

// Neighbor counts: a copy of the box's geometry CNT_SHIFT columns to the
// right, so a cell's count is at a fixed offset from its pixel. Each live
// cell adds 16 to its own count, so an entry is `16 * alive + neighbors`.
#define CNT_COL 24u
#define CNT_SHIFT (CNT_COL - 1u)

// The two live lists, current and next, alternate between the top and the
// bottom margin row. One entry of slack: the branchless push always writes
// one entry past the end of the list.
#define LIST_COL CNT_COL
#define LIST(which) (board + AT((which) ? BOTTOM : 0u, LIST_COL))

// The cells a generation flips, applied to the board in one burst once the
// whole generation is known. BURST - 1 entries of slack, for the padding
// the burst reads past the end.
#define BURST 8u
#define CHANGE_COL 16u // 100 cells + slack fit before the row ends
#define CHANGES (board + AT(BOTTOM + 1u, CHANGE_COL))

// Scalars, in the margin rows past the lists.
#define VAR(row, col) board[AT(row, col)]
#define NLIVE VAR(0u, LIST_COL + SIDE * SIDE + 1u)
#define WHICH VAR(0u, LIST_COL + SIDE * SIDE + 2u) // which list is current
#define SPIN_STEP VAR(0u, LIST_COL + SIDE * SIDE + 3u)
// A generation takes far longer than a key press, so `step` polls for Enter
// while it works and saves a stop request in STOP_PENDING; the world stops
// once the generation is complete. A request counts only once Enter has
// been seen released since the world started (STOP_ARMED), so the start
// press never stops it.
#define STOP_ARMED VAR(BOTTOM, LIST_COL + SIDE * SIDE + 1u)
#define STOP_PENDING VAR(BOTTOM, LIST_COL + SIDE * SIDE + 2u)
// A byte that branchless code writes to instead of skipping a store.
#define SCRATCH AT(BOTTOM, LIST_COL + SIDE * SIDE + 3u)

// 3x3 glyphs: one 3-bit row per 3 bits, bit c of a row lighting column c.
#define GLYPH(r0, r1, r2) ((r0) | (r1) << 3 | (r2) << 6)

// The start/stop sign at IND_ROW/IND_COL shows what Enter will do.
#define IND_ROW 2u
#define IND_COL 16u
#define PLAY GLYPH(0x1u, 0x3u, 0x1u)  // #..  ##.  #..  shown while stopped
#define PAUSE GLYPH(0x5u, 0x5u, 0x5u) // #.#  #.#  #.#  shown while running

// The spinner: a 3x3 ring at SPIN_ROW/SPIN_COL whose bright head steps
// clockwise once per generation, trailing a fading tail.
#define SPIN_ROW 9u
#define SPIN_COL 16u
#define SPIN(dr, dc) AT(SPIN_ROW + (dr), SPIN_COL + (dc))
static const unsigned short spin_at[8] = {
    SPIN(0u, 0u), SPIN(0u, 1u), SPIN(0u, 2u), SPIN(1u, 2u),
    SPIN(2u, 2u), SPIN(2u, 1u), SPIN(2u, 0u), SPIN(1u, 0u),
};
static const u8 spin_shade[4] = {255, 144, 64, DIM}; // head, tail..., rest

#define BOX_SIDES(r) [AT(r, 1u)] = GRAY, [AT(r, RING)] = GRAY,
#define PX(r, c) [AT(IND_ROW + (r), IND_COL + (c))] = 255,
static u8 board[ROWS * STRIDE] __attribute__((aligned(STRIDE))) = {
    [AT(1u, 1u)... AT(1u, RING)] = GRAY,
    BOX_SIDES(2u) BOX_SIDES(3u) BOX_SIDES(4u) BOX_SIDES(5u) BOX_SIDES(6u)
    BOX_SIDES(7u) BOX_SIDES(8u) BOX_SIDES(9u) BOX_SIDES(10u) BOX_SIDES(11u)
    [AT(RING, 1u)... AT(RING, RING)] = GRAY,
    // Stopped at start: the play glyph, and the spinner's head at step 0.
    PX(0u, 0u) PX(1u, 0u) PX(1u, 1u) PX(2u, 0u)
    [SPIN(0u, 0u)] = 255, [SPIN(1u, 0u)] = 144, [SPIN(2u, 0u)] = 64,
    [SPIN(0u, 1u)] = DIM, [SPIN(0u, 2u)] = DIM, [SPIN(1u, 2u)] = DIM,
    [SPIN(2u, 2u)] = DIM, [SPIN(2u, 1u)] = DIM,
};
#undef PX
#undef BOX_SIDES

static void put_str(const char *s) {
  while (*s)
    putchar(*s++);
}

// Glyph bits are consumed one at a time with constant shifts: a variable
// shift is a call into the runtime's shift helper.
// Unrolled: nine stores to fixed addresses, which the scheduler can issue
// together, and only constant shifts (a variable shift is a call into the
// runtime's shift helper). Inlined, so the program makes no calls at all:
// wss then emits no call stack, whose cells and visualizer pixels would be
// evaluated on every recalc.
__attribute__((always_inline)) static inline void draw_indicator(u32 glyph,
                                                                 u32 shade) {
#define PIXEL(k)                                                               \
  board[AT(IND_ROW + (k) / 3u, IND_COL + (k) % 3u)] =                          \
      (u8)(shade & (0u - ((glyph >> (k)) & 1u)));
  PIXEL(0u) PIXEL(1u) PIXEL(2u) PIXEL(3u) PIXEL(4u)
  PIXEL(5u) PIXEL(6u) PIXEL(7u) PIXEL(8u)
#undef PIXEL
}

// Polls for a stop request into the caller's `armed` and `pending`. It is
// branchless, so it shares cycles with the loop bodies; the gray sign goes
// up at the next phase boundary.
#define POLL_STOP()                                                            \
  do {                                                                         \
    u32 enter_ = (u8)try_getchar() == '\n';                                    \
    pending |= enter_ & armed;                                                 \
    armed |= enter_ ^ 1u;                                                      \
  } while (0)

// Rebuild the live list from the board after cells were flipped by hand.
// It is slow enough to poll for a stop request as well.
static void collect_live(void) {
  u32 armed = STOP_ARMED;
  u32 pending = STOP_PENDING;
  u8 *live = LIST(WHICH);
  u32 n = 0;
  for (u32 y = 0; y < SIDE; ++y) {
    POLL_STOP();
    // A row's ten pixels are constant offsets from one base, so they load
    // in one cycle (`volatile`: see `step`). The list entry of cell (x, y)
    // is the packed index of its top-left neighbor, `y * PACK_ROW + x`.
    volatile u8 *row = board + AT(FIRST + y, FIRST);
#define COLS(X) X(0) X(1) X(2) X(3) X(4) X(5) X(6) X(7) X(8) X(9)
#define LOAD(x) u32 v##x = row[x];
#define PUSH(x)                                                                \
  live[n] = (u8)(y * PACK_ROW + (x));                                          \
  n += v##x != 0;
    COLS(LOAD)
    COLS(PUSH)
#undef PUSH
#undef LOAD
#undef COLS
  }
  NLIVE = (u8)n;
  STOP_ARMED = (u8)armed;
  STOP_PENDING = (u8)pending;
  if (pending)
    draw_indicator(PAUSE, GRAY);
}

// The 3x3 neighborhood of a live list entry b0: X(k, packed offset from b0,
// pixel offset from CELL(b0)), with k == 4 the cell itself.
#define NEIGHBORS(X)                                                           \
  X(0, 0u, AT(0u, 0u)) X(1, 1u, AT(0u, 1u)) X(2, 2u, AT(0u, 2u))               \
  X(3, 16u, AT(1u, 0u)) X(4, 17u, AT(1u, 1u)) X(5, 18u, AT(1u, 2u))            \
  X(6, 32u, AT(2u, 0u)) X(7, 33u, AT(2u, 1u)) X(8, 34u, AT(2u, 2u))

// One generation, touching only live cells and their neighbors. First each
// live cell bumps the counts of its neighborhood; then each neighborhood is
// settled: a cell with a nonzero count gets its new state and its count is
// cleared, so a cell shared by several neighborhoods is settled once. The
// board itself is left alone until the end, when the cells that changed
// are flipped in one burst: it never shows a half-computed generation.
// Returns whether a stop was requested meanwhile; the request shows as a
// gray pause sign.
//
// All accesses to one neighborhood, pixels and counts, are constant offsets
// from one base address, so the scheduler sees they are disjoint and issues
// the loads, and the stores, together. `volatile` keeps clang from
// interleaving each load with its store, which would let the register
// allocator reuse one register for every loaded value and serialize them.
static u32 step(void) {
  u32 armed = STOP_ARMED;
  u32 pending = STOP_PENDING;
  u32 shown = pending;
#define SHOW_PENDING()                                                         \
  if (pending & (shown ^ 1u)) {                                                \
    shown = 1;                                                                 \
    draw_indicator(PAUSE, GRAY);                                               \
  }
  u32 which = WHICH;
  const u8 *live = LIST(which);
  u8 *next = LIST(which ^ 1u);
  u32 nlive = NLIVE;

  for (u32 i = 0; i < nlive; ++i) {
    POLL_STOP();
    volatile u8 *q = board + CELL(live[i]);
#define LOAD(k, poff, off) u32 v##k = q[CNT_SHIFT + (off)];
#define BUMP(k, poff, off) q[CNT_SHIFT + (off)] = (u8)(v##k + ((k) == 4 ? 16u : 1u));
    NEIGHBORS(LOAD)
    NEIGHBORS(BUMP)
#undef BUMP
#undef LOAD
  }

  SHOW_PENDING();

  u8 *changes = CHANGES;
  u32 n = 0, nf = 0;
  for (u32 i = 0; i < nlive; ++i) {
    POLL_STOP();
    u32 b0 = live[i];
    volatile u8 *q = board + CELL(b0);
    // Birth on 3 neighbors (v == 3), survival on 2 or 3 (v == 18, 19). The
    // box, the only gray pixels around, never comes alive. A zero count
    // means the cell was settled already. The board still holds the old
    // generation, so a cell flips when its new state differs from its pixel.
#define LOAD(k, poff, off)                                                     \
  u32 v##k = q[CNT_SHIFT + (off)];                                             \
  u32 o##k = q[off];
#define SETTLE(k, poff, off)                                                   \
  u32 in##k = o##k != GRAY;                                                    \
  u32 born##k = ((v##k == 3u) | ((v##k & 0xfeu) == 18u)) & in##k;              \
  u32 flip##k = (v##k != 0) & in##k & (born##k != (o##k == 255u));
#define STORE(k, poff, off) q[CNT_SHIFT + (off)] = 0;
#define PUSH(k, poff, off)                                                     \
  next[n] = (u8)(b0 + (poff) - CORNER);                                        \
  n += born##k;                                                                \
  changes[nf] = (u8)(b0 + (poff));                                             \
  nf += flip##k;
    NEIGHBORS(LOAD)
    NEIGHBORS(SETTLE)
    NEIGHBORS(STORE)
    NEIGHBORS(PUSH)
#undef PUSH
#undef STORE
#undef SETTLE
#undef LOAD
  }
  // The burst: BURST flips per iteration. Entries past the end are padding
  // that names the box's corner, whose pixel is written back unchanged, so
  // the addresses need no bounds check. All loads come before the stores
  // (`volatile` keeps that order; a cell is listed at most once): a load
  // after a store to a computed address would have to wait for it. The
  // stores still go one per cycle, since the scheduler cannot tell computed
  // addresses apart.
  for (u32 k = 0; k < BURST - 1u; ++k)
    changes[nf + k] = 0;
  for (u32 i = 0; i < nf; i += BURST) {
    volatile u8 *entry = changes + i;
    volatile u8 *pixels = board;
#define EACH(X) X(0u) X(1u) X(2u) X(3u) X(4u) X(5u) X(6u) X(7u)
#define ENTRY(k) u32 e##k = entry[k];
#define AT_(k) u32 at##k = CELL(e##k);
#define LOAD(k) u32 p##k = pixels[at##k];
#define STORE(k) pixels[at##k] = (u8)(i + (k) < nf ? p##k ^ 255u : p##k);
    EACH(ENTRY) EACH(AT_) EACH(LOAD) EACH(STORE)
#undef STORE
#undef LOAD
#undef AT_
#undef ENTRY
#undef EACH
  }
  WHICH = (u8)(which ^ 1u);
  NLIVE = (u8)n;

  // Advance the spinner: the new head, its fading tail, and the pixel the
  // tail just left.
  u32 h = ++SPIN_STEP;
  for (u32 i = 0; i < 4u; ++i)
    board[spin_at[(h - i) & 7u]] = spin_shade[i];

  POLL_STOP();
  SHOW_PENDING();
#undef SHOW_PENDING
  STOP_ARMED = (u8)armed;
  STOP_PENDING = (u8)pending;
  return pending;
}

// A glider heading down-right, 3x3 around the cursor and clipped to the
// board.
#define GLIDER GLYPH(0x2u, 0x4u, 0x7u) // .#.  ..#  ###

// Unrolled and branchless: a cell off the board is written to SCRATCH.
static void stamp_glider(u32 cur) {
  u32 y = cur / STRIDE, x = cur % STRIDE;
#define STAMP(k)                                                               \
  {                                                                            \
    u32 on_ = (y + (k) / 3u - 1u - FIRST < SIDE) &                             \
              (x + (k) % 3u - 1u - FIRST < SIDE);                              \
    board[on_ ? cur + AT((k) / 3u, (k) % 3u) - AT(1u, 1u) : SCRATCH] =         \
        (u8)(0u - ((GLIDER >> (k)) & 1u));                                     \
  }
  STAMP(0u) STAMP(1u) STAMP(2u) STAMP(3u) STAMP(4u)
  STAMP(5u) STAMP(6u) STAMP(7u) STAMP(8u)
#undef STAMP
}

int _start(void) {
  volatile u8 *vboard = board;
  static const char *const volatile banner =
      "Life: w/a/s/d move, space flips, g glider, enter starts/stops.\n";
  put_str(banner);

  u32 cur = AT(FIRST + SIDE / 2u, FIRST + SIDE / 2u);
  int running = 0;
  int dirty = 1;
  u8 prev = NO_KEY;
  for (;;) {
    // Reading the cursor cell drives the visualizer's read highlight; the
    // value is reused when Space flips the cell.
    u8 c;
    u8 k;
    if (running) {
      c = vboard[cur];
      k = (u8)try_getchar();
    } else {
      volatile u8 *cursor = vboard + cur;
      do
        c = *cursor;
      while ((u8)try_getchar() == prev);
      k = (u8)try_getchar();
    }
    if (k != prev) {
      prev = k;
      switch (k) {
      case 'w':
        if (cur >= AT(FIRST + 1u, 0u))
          cur -= STRIDE;
        break;
      case 's':
        if (cur < AT(LAST, 0u))
          cur += STRIDE;
        break;
      case 'a':
        if (cur % STRIDE > FIRST)
          cur -= 1u;
        break;
      case 'd':
        if (cur % STRIDE < LAST)
          cur += 1u;
        break;
      case ' ':
        vboard[cur] = (u8)(c ^ 255u);
        dirty = 1;
        break;
      case 'g':
        stamp_glider(cur);
        dirty = 1;
        break;
      case '\n':
        running = !running;
        STOP_ARMED = 0;
        draw_indicator(running ? PAUSE : PLAY, 255u);
        break;
      }
    }
    if (running) {
      if (dirty) {
        collect_live();
        dirty = 0;
      }
      if (step()) {
        running = 0;
        STOP_PENDING = 0;
        draw_indicator(PLAY, 255u);
        // Enter may still be held: take it as already seen.
        prev = (u8)try_getchar();
      }
    }
  }
}
