# wss

wss is a transpiler from WebAssembly to CSS, plus an HTML/CSS runtime for the output.

The result runs entirely in pure HTML/CSS; optional JavaScript add-ons are available for quality-of-life features.

I should have a blog post about this: PLACEHOLDER

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
  -o a.wasm source.c
```

2. Run the transpiler:

```sh
cargo run --bin wss -- a.wasm
```

Optional runtime context flags:

```sh
cargo run --bin wss -- a.wasm \
  --output a.html \
  --memory-bytes 1024 \
  --stack-slots 256 \
  --js-clock true \
  --js-coprocessor false
```

* **`--memory-bytes 0`**: Uses global `0` (SP) as the runtime memory cap.
* **`--js-clock true`**: The JS clock is enabled by default. The pure CSS animation-based clock is functional but not as stable as the JS one. You may experience random resets when the JS clock is disabled, especially under heavy workloads.
* **`--js-coprocessor true`**: Offloads `div`/`rem` and bitwise built-in ops to a JS coprocessor channel (requires `--js-clock true`). If you have to do i32 division or remainder calculations in CSS, adding a JS coprocessor will drastically improve performance. However, both methods work.

3. Open the output HTML (default: `a.html`) in a recent Chromium-based browser.

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
