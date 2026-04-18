# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Common commands

- Build: `cargo build`
- Build optimized binary: `cargo build --release`
- Run transpiler: `cargo run -- <input.wasm> -o <output.html>`
- Format: `cargo fmt --all`
- Format check (CI style): `cargo fmt --all -- --check`
- Lint: `cargo clippy --all-targets --all-features`
- Rust tests: `cargo test`
- Run a single Rust test: `cargo test <test_name>`
  - Example: `cargo test js_clock_defaults_to_enabled`

### Blackbox / end-to-end tests

- Run blackbox suite: `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml --target-dir target --`
- Run specific blackbox cases: `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml --target-dir target -- <case-id>...`
  - Example: `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml --target-dir target -- bitwise_shift`
- Include lengthy-tagged cases in the default suite: `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml --target-dir target -- --include-lengthy`
- Run lengthy blackbox cases only: `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml --target-dir target -- --only-lengthy`

Notes:
- Runner lives in `tests/blackbox-runner/`.
- CI pins `CARGO_TARGET_DIR=target` with shared rust-cache key `blackbox` to reuse blackbox build artifacts across jobs.
- The runner rebuilds `target/release/wss` before running when `WSS_BIN` is unset, so the suite exercises the current source tree.
- With no per-case override, blackbox cases default to JS clock enabled (matching the CLI); `js_coprocessor` still forces JS clock on.
- With no case IDs, the harness skips `lengthy`, `broken`, and `unimplemented` cases unless included by flags.
- Blackbox tests require local toolchain dependencies (notably `clang` targeting wasm and Chrome, configurable via `WSS_CHROME`).

## Architecture overview

`wss` is a single Rust binary that compiles a constrained WebAssembly subset into a self-contained HTML/CSS runtime.

Compiler pipeline (wired in `src/main.rs`):

1. `module::decode_module_info` (`src/module/mod.rs`)
   - Reads wasm sections into `ModuleInfo` (types, funcs, globals, tables, memory pages, data segments, imports/exports).
2. `validate::validate` (`src/validate/mod.rs`)
   - Enforces the supported wasm subset before lowering.
3. `parse::parse_module` (`src/parse/mod.rs`)
   - Converts wasm operators to AST (`AstModule`, `AstFuncBody`, `ast::Node`).
4. `lower::lower_module` (`src/lower.rs`)
   - Lowers AST to CFG IR (`IrModule`, `BasicBlock`, `Terminator`, `IrNode` in `src/ir.rs`).
5. `lower8::lower8_module` (`src/lower8/mod.rs`)
   - Lowers IR to byte-oriented IR8 (`Ir8Program` in `src/ir8.rs`) with explicit 8-bit lanes/words and builtin call lowering.
6. `opt8::run` (`src/opt8/mod.rs`)
   - Fixed-point IR8 optimization passes (copy propagation/elimination, instcombine/simplify, DCE, CFG cleanup, etc.).
7. `regalloc::regalloc` (`src/regalloc.rs`)
   - Allocates/compacts physical register groups from virtual regs.
8. `schedule::schedule` (`src/schedule.rs`)
   - Packs IR8 ops into execution cycles under hazard/complexity limits and rewrites PCs.
9. `emit::emit_program` (`src/emit/mod.rs`)
   - Generates final HTML by injecting generated CSS logic/properties/support into `emit/base.html`.

### Important IR/data-model boundaries

- `ModuleInfo` (decoded wasm metadata) -> `AstModule` -> `IrModule` -> `Ir8Program` -> scheduled `cycles` -> emitted HTML/CSS.
- Keep stage boundaries clean: each stage transforms the previous representation and should not bypass intermediate abstractions.

### Runtime and feature constraints that affect edits

- The toolchain is currently i32-centric in validation/lowering paths (many explicit TODO(i64) markers).
- Entry contract: wasm module must export `_start`; imports are limited to runtime-style `putchar` / `getchar` signatures.
- Exception handling supports zero- or single-`i32`-payload tags, but `try_table`/`throw_ref` are unsupported and `delegate`/`rethrow` depth is currently limited to `0`.
- Emission uses template feature slicing in `emit/base.html` (KEEP markers): generated output relies on these markers remaining consistent with `emit/mod.rs` feature toggles.
- Scheduler and regalloc assumptions are tightly coupled to IR8 register/PC conventions (`VREG_START`, `PC_STRIDE` in `src/ir8.rs`).

## CI parity checks

CI runs:
- `cargo fmt --all -- --check`
- `cargo clippy --all-targets --all-features`
- `cargo build --verbose`
- `cargo test --verbose`
- `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml --target-dir target -- --retries=3 --jobs=3`
- `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml --target-dir target -- --only-lengthy --retries=3 --jobs=3`

## Coding Style & Naming Conventions
- `snake_case` for functions/modules/files, `CamelCase` for types, `UPPER_SNAKE_CASE` for constants.
- Prefer iterator and functional style over loop-heavy imperative style when it keeps code clear.
- Prefer stage-focused functions and explicit error context (`anyhow::Context`).
- Keep modules aligned with pipeline boundaries; avoid cross-stage coupling unless required.
- Keep static CSS functions in base.html and use guard to eagerly remove unused ones.
- Avoid introducing one-time trivial wrapprers around existing functions, keep one functionality only in one place.
- Open to add new dependencies if they simplify code.
