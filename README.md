# wss

wss is a transpiler from WebAssembly to CSS, plus an HTML/CSS runtime.

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

Use any WebAssembly-capable compiler. Export `_start` as the entry point and (optionally) import `putchar` and `getchar` for console I/O.

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
- `--js-clock <true|false>`: enable JS-based clock stepping. Default: `true`
- `--js-coprocessor <true|false>`: enable JS coprocessor for `div`/`rem` and bitwise builtins. Requires `--js-clock true`
- `--js-clock-debugger <true|false>`: enable the JS debugger popup. Requires `--js-clock true`
- `--max-phys-regs <N>`: register-allocation cap, including reserved `r0`-`r3`. Default: `256`

## What is supported

- **i32 arithmetic** — add, sub, mul, div, rem, and, or, xor, shl, shr, rotl, rotr, clz, ctz, popcnt, eqz
- **i32 comparisons** — eq, ne, lt, gt, le, ge (signed and unsigned)
- **i32 memory** — load and store with 8-, 16-, and 32-bit widths (signed and unsigned loads)
- **Control flow** — block, loop, if/else, br, br_if, br_table, return, unreachable
- **Function calls with TCO** — call, call_indirect, return_call, return_call_indirect
- **Locals and globals** — get, set, tee
- **Select** — typed and untyped select

## What is under development

- **i64 arithmetic**
- **Exception handling** — try/catch and related ops

## What will not be supported

- **Floats** — f32/f64 and all float ops
- **SIMD** — v128 and vector ops
- **Atomics** — all atomic load/store/rmw
- **memory.grow** — impossible to implement without JS
- **GC** - No
- **Threads** - No

## Credits

This project is inspired by [x86CSS](https://lyra.horse/x86css/).
