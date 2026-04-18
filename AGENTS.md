# Repository Guidelines

## High-Signal Repo Facts
- `wss` is a single Rust binary; pipeline order in `src/main.rs` is fixed: `validate -> parse -> lower -> lower8 -> opt8 -> regalloc -> schedule -> emit`.
- Template wiring is strict: `src/emit/mod.rs` expects placeholders/KEEP markers in `src/emit/base.html`; keep both sides in sync.
- Validator guardrails (`src/validate/mod.rs`) are easy to forget: only `i32` value types, `_start` export is required and must return `i32`, max two imports (`getchar`, `putchar`), and table elements must be `funcref` with <= 256 entries.
- Exception handling supports zero- or single-`i32`-payload tags end-to-end; `try_table`/`throw_ref` are still unsupported, and `delegate`/`rethrow` only support depth `0`.
- CSS runtime limitation: nested custom-function calls evaluate to `0` (for example `--sel(x,--sel(y,z,w),v)`).

## Commands That Match CI/Scripts
- Rust CI order: `cargo fmt --all -- --check && cargo clippy --all-targets --all-features && cargo build --verbose && cargo test --verbose`.
- Transpile a wasm file: `cargo run --release -- <input.wasm> -o <output.html>`.
- Run one Rust test: `cargo test <test_name>`.
- Blackbox default run: `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml --target-dir target --`.
- Blackbox targeted run: `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml --target-dir target -- <case-id>...`.
- Lengthy CI-style blackbox pass: `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml --target-dir target -- --only-lengthy --retries=3 --jobs=3`.

## Blackbox Prereqs and Quirks
- Requires `clang` + Chrome/Chromium; CI uses clang 22.
- If Chrome is not at the default path, set `WSS_CHROME`.
- Runner lives in a separate Cargo project at `tests/blackbox-runner/`.
- CI pins `CARGO_TARGET_DIR=target` and a shared rust-cache key (`blackbox`) so blackbox jobs reuse artifacts.
- Harness rebuilds `target/release/wss` before running when `WSS_BIN` is unset, so blackbox runs exercise the current source tree.
- With no per-case override, blackbox cases default to JS clock enabled (matching the CLI); `js_coprocessor` still forces JS clock on.
- With no explicit case IDs, `lengthy`, `broken`, and `unimplemented` cases are skipped unless included by flags.
- Generated blackbox artifacts land in `tests/out/` (gitignored).

## Change-Scoped Validation
- Parser/lowering/opt changes: add unit tests in the touched module and targeted blackbox cases in `tests/cases/` + `tests/cases.json`.
- New blackbox C cases should use `volatile` or `__attribute__((optnone))` to avoid constant-folded false positives.
- If `src/schedule.rs` changes, additionally run:
  `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml --target-dir target -- division_large_unsigned division_large_signed division_hazard_small_u24_remonly division_large_unsigned_divonly division_large_unsigned_remonly division_large_signed_divonly division_large_signed_remonly`.

## Documentation Sync Rule
- When commands, CI behavior, or test workflow changes, update `AGENTS.md`, `CLAUDE.md`, and `README.md` together.
