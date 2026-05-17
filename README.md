# wss

wss is an _optimizing_ transpiler from WebAssembly to CSS, plus an HTML/CSS runtime.

The result runs entirely in pure HTML/CSS; optional JavaScript add-ons are available for quality-of-life features.

## Examples

Several interactive outputs are checked into [`examples/`](examples/) and mirrored online:

- [Hanoi 4 (JS clock)](https://bucket.svc.moe/uploads/wss/hanoi4_js.html)
- [Hanoi 4 (CSS clock)](https://bucket.svc.moe/uploads/wss/hanoi4_nojs.html)
- [Horsle (JS debugger)](https://bucket.svc.moe/uploads/wss/horsle_js_step.html)
- [RPG (JS clock)](https://bucket.svc.moe/uploads/wss/rpg_js.html)

## Prerequisites

- Rust toolchain
- A compiler that can emit WebAssembly, such as `clang`
- A recent Chromium-based browser for running the generated HTML

## Usage

1. Prepare your WebAssembly file:

```c
// source.c
extern int putchar(int c);
extern int getchar();

int _start() {
    putchar('H');
    putchar('e');
    putchar('l');
    putchar('l');
    putchar('o');
    putchar('\n');
    return 0;
}
```

Use any WebAssembly-capable compiler. Export `_start` as the entry point and (optionally) import `putchar`, `getchar`, and `rand` for console I/O and randomness.

```sh
clang \
  --target=wasm32 -Os \
  -nostdlib \
  -mno-simd128 \
  -fno-exceptions \
  -mno-bulk-memory \
  -mno-multivalue \
  -Wfloat-conversion \
  -Wl,--gc-sections \
  -Wl,--no-stack-first \
  -Wl,--allow-undefined \
  -Wl,-z,stack-size=512 \
  -Wl,--compress-relocations \
  -Wl,--strip-all \
  -Wl,--global-base=4 \
  -Wl,--export=_start \
  -o a.wasm source.c
```

2. Run the transpiler:

```sh
cargo run --release -- a.wasm -o a.html
```

Options:

- `-o, --output <PATH>`: output HTML path. Default: `a.html`
- `--memory-bytes <N>`: runtime linear-memory cap in bytes. Default: `1024`
- `--stack-slots <N>`: runtime callstack cap in 16-bit slots. Default: `256`
- `--js-clock`: enable JS-based clock stepping. This is the default.
- `--no-js-clock`: disable JS-based clock stepping.
- `--js-coprocessor`: enable the JS coprocessor for `div`/`rem` and bitwise builtins. Conflicts with `--no-js-clock`.
- `--js-clock-debugger`: enable the JS debugger popup. Conflicts with `--no-js-clock`.
- `--no-visualizers`: omit the memory and callstack visualizers from the emitted runtime.
- `--no-indicators`: drop the PC / SP / G0 indicator panel.
- `--no-memory-trap`: omit the linear-memory bounds check. OOB reads return 0; OOB writes are dropped.
- `--no-callstack-trap`: omit the callstack-overflow bounds check. OOB reads return 0; OOB writes are dropped.
- `--max-phys-regs <N>`: register-allocation cap, including reserved `r0`-`r3`. Default: `256`
- `--no-embed-compile-command`: skip the leading `<!-- compile command: … -->` and `<!-- seeds: … -->` header comments.

### Obfuscation passes

Optional post-pipeline rewrites. All conflict with `--js-clock-debugger`. Seeded flags take a bare form (random seed) or `--flag=<SEED>` (reproducible); resolved seeds are echoed in a `<!-- seeds: … -->` header when `--no-embed-compile-command` is not set.

PC relabelling (mutually exclusive):

- `--randomize-pc[=SEED]`: shuffle PC labels and renumber 1..N.
- `--sparse-pc[=SEED]`: sample PC labels uniformly from the 16-bit space; also hides cycle count.

CSS rewrites:

- `--minify-vars[=SEED]`: rename custom properties to shortest idents; sort decls (shuffle `@property` bodies); strip comments; flatten `<style>` whitespace.
- `--split-pc[=SEED]`: split PC-keyed `if()` chains via `--__{N}` helpers.
- `--shuffle-arms[=SEED]`: permute mutually exclusive `if()` arms (LUTs).
- `--shuffle-ops[=SEED]`: reorder commutative operands (`Sum`, `Product`, `min`/`max`/`hypot`, `or` chains).
- `--shuffle-at-rules[=SEED]`: permute `@property` / `@function` positions.
- `--decoy-fallbacks[=SEED]`: add dead integer fallbacks to `var()` reads of `@property`-registered names.
- `--decoy-arms[=SEED]`: inject unreachable arms into `<integer>`-returning LUT `@function`s.
- `--minify-js`: strip comments and collapse whitespace inside `<script>`.

## Testing

- The blackbox runner rebuilds `target/release/wss` before running when `WSS_BIN` is unset, so the suite uses the current source tree by default.
- With no per-case override, blackbox cases default to JS clock enabled to match the CLI default. Cases can still opt out with `"js_clock": false`.

## What is supported

- **i32 arithmetic** — add, sub, mul, div, rem, and, or, xor, shl, shr, rotl, rotr, clz, ctz, popcnt, eqz
- **i32 comparisons** — eq, ne, lt, gt, le, ge (signed and unsigned)
- **i32 memory** — load and store with 8-, 16-, and 32-bit widths (signed and unsigned loads)
- **Bulk memory** — `memory.fill` and `memory.copy` lowered to loops
- **Control flow** — block, loop, if/else, br, br_if, br_table, return, unreachable
- **Function calls with TCO** — call, call_indirect, return_call, return_call_indirect
- **Locals and globals** — get, set, tee
- **Select** — typed and untyped select
- **rand()** — randomeness extracted from CSS keyframe animations
- **Exception handling** — `try`, `catch`, `catch_all`, `delegate`, `throw`, `rethrow` on exception tags with either no payload or a single `i32` payload. Doesn't support: multi-value or non-`i32` tag payloads, `try_table`, `throw_ref`, `delegate` depth > `0`, `rethrow` depth > `0`, and `rethrow` from `catch_all`.

## What is under development

- Full i64 integer support
- Some exception-handling features

## What will not be supported

- **Floats** — f32/f64 and all float ops
- **SIMD** — v128 and vector ops
- **Atomics** — all atomic load/store/rmw
- **memory.grow** — impossible to implement without JS
- **GC** - No
- **Threads** - No

## Credits

This project is inspired by [x86CSS](https://lyra.horse/x86css/).
