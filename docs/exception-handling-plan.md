# Exception Handling End-to-End Implementation Plan

## Goal

Implement WebAssembly exception handling end-to-end in `wss` so modules using EH can be validated, parsed, lowered, scheduled, emitted, and run correctly in the HTML/CSS runtime.

## Current Constraints (from codebase)

- EH operators are currently rejected (`src/validate/operators.rs`).
- AST/IR do not model EH constructs (`src/ast.rs`, `src/ir.rs`).
- Runtime has trap model only (`src/ir8.rs`, `src/emit/support.rs`).
- Blackbox harness assumes terminal PCs are only `-1..-5` (`tests/blackbox-runner/src/probe.template.js`, `src/emit/base.html`).
- Project is i32-only and single-result oriented.

## Recommended Scope

### V1 (recommended)

- Support: `try`, `catch`, `catch_all`, `throw`, `rethrow`, `delegate`.
- Keep unsupported (explicit validator error): `try_table`, `throw_ref`, exnref/reference-typed payloads.
- Tag payloads: start with zero-payload tags for control-flow correctness.
- Introduce uncaught-exception terminal status code `-6`.

### V1.1 (immediately after V1)

- Extend payload support to one `i32` payload.
- Keep multi-value/reference payloads unsupported.

## Core Architecture Choice

Use an explicit exception state channel (flag + tag + optional payload) propagated through normal call/return paths, instead of implementing runtime stack scanning/unwinding logic.

Why:

- Fits current callstack model (`CsStore/CsLoad/CsAlloc/CsFree`).
- Requires fewer invasive runtime semantics changes.
- Keeps scheduler/regalloc complexity bounded.

## Phase Plan

## Phase 0 - Design Freeze and Invariants

1. Define EH invariants:
   - Non-exceptional path keeps `exc_flag == 0`.
   - Throw sets `exc_flag = 1`, `exc_tag = tag`, optional payload.
   - Catch entry clears `exc_flag`.
2. Add new trap code for uncaught exception (`-6`) and error UI copy.
3. Disable or gate tail-call fusion when EH is active (to avoid bypassing exception checks).

Acceptance:

- Design doc approved.
- Explicit invariants documented in code comments and tests.

## Phase 1 - Module Metadata and Validation

Files: `src/module/mod.rs`, `src/validate/mod.rs`, `src/validate/operators.rs`

1. Add tag metadata:
   - `TagInfo` in `ModuleInfo`.
   - Decode `TagSection`.
   - Decode imported tags from `ImportSection`.
2. Validator:
   - Remove blanket EH rejection.
   - Validate tag indices for `throw`/`catch`.
   - Validate allowed EH subset.
   - Validate tag signature constraints per scope (V1: no payloads; V1.1: <=1 i32 payload).
3. Keep unsupported EH ops rejected with precise messages.

Acceptance:

- Validator accepts selected EH subset and rejects out-of-scope ops with deterministic errors.

## Phase 2 - AST and Parser Support

Files: `src/ast.rs`, `src/parse/frame.rs`, `src/parse/mod.rs`, `src/print.rs`

1. Add AST nodes:
   - `Try { body, catches, catch_all, delegate }`
   - `Throw { tag, args }`
   - `Rethrow(depth)`
   - `Delegate(depth)`
2. Extend block-frame model for try/catch transitions.
3. Parse EH operators and preserve existing stack materialization behavior (temp locals at block boundaries).
4. Ensure parser's reachability handling works with throw/rethrow/delegate.
5. Add printer support for new AST forms.

Acceptance:

- Parser can ingest EH wasm without fallback errors.
- AST dump reflects EH structure accurately.

## Phase 3 - Lowering to CFG with Exceptional Edges

Files: `src/lower.rs`, `src/ir.rs`, `src/lower8/analysis.rs` (if IR uses new forms), `src/print.rs`

1. Lower try/catch to explicit CFG blocks:
   - try body block
   - dispatch block
   - catch block(s)
   - merge block
2. Model exception state updates in lowered IR:
   - throw sets state + jumps to nearest handler dispatch (or uncaught path).
3. Add post-call exception checks:
   - After `call`/`call_indirect`, branch on `exc_flag`.
   - If set: jump to nearest handler dispatch or propagate.
4. Handle `rethrow`/`delegate` by forwarding to outer handler target.

Acceptance:

- IR CFG has explicit exceptional control paths.
- No uncaught exception can silently continue normal execution.

## Phase 4 - Lower8 Integration

Files: `src/lower8/mod.rs`, `src/lower8/ops.rs`, `src/lower8/calls.rs`, `src/ir8.rs`

1. Add runtime representation for exception state:
   - either hidden globals appended at lower8 time, or explicit IR8 ops.
2. Ensure call/return paths preserve and propagate exception state.
3. Add `TrapCode::UncaughtException = -6`.
4. Ensure entry function uncaught path terminates with `Trap(-6)`.

Acceptance:

- IR8 contains valid state transitions for throw/catch/propagation.
- Scheduled program reaches `-6` for uncaught exceptions.

## Phase 5 - Emitter and Runtime Surface

Files: `src/emit/logic.rs`, `src/emit/support.rs`, `src/emit/base.html`, `src/emit/tests.rs`

1. Emit logic for new trap code and EH state handling.
2. Add UI text for uncaught exception (`[Trap: uncaught exception]`).
3. Include `-6` in runtime halt sets.
4. Keep existing trap behavior (`-2..-5`) unchanged.

Acceptance:

- HTML runtime halts and displays correct status on uncaught exception.
- Existing trap UI tests still pass.

## Phase 6 - Optimizer, Allocator, Scheduler, Printers

Files: `src/opt8/mod.rs`, `src/regalloc.rs`, `src/schedule.rs`, `src/print.rs`

1. Update exhaustive matches for any new terminators/op kinds.
2. Mark EH state-mutating ops as side-effecting where needed (DCE safety).
3. Ensure goto-threading/coalescing do not remove exceptional control paths.

Acceptance:

- `cargo clippy --all-targets --all-features` clean.
- No optimizer-induced EH miscompilations in regression tests.

## Phase 7 - Test Strategy

Files: `src/*` tests + `tests/cases/*` + `tests/cases.json` + `tests/blackbox-runner/src/*`

1. Unit tests:
   - Validator acceptance/rejection matrix for EH subset.
   - Parser EH structure tests.
   - Lowering tests for call-propagated exceptions.
   - Emitter trap/UI tests for `-6`.
2. Blackbox tests (new case IDs):
   - `exception_local_try_catch`
   - `exception_cross_function_catch`
   - `exception_rethrow_outer_catch`
   - `exception_delegate_to_outer`
   - `exception_uncaught_trap`
3. Harness updates:
   - Add EH-aware compile config per case (or wasm fixture path).
   - Extend terminal PC sets to include `-6`.

Acceptance:

- New EH cases pass.
- Existing non-EH suite remains green.

## Phase 8 - Documentation and Rollout

Files: `README.md`, `docs/exception-handling-plan.md` (this doc), optional changelog notes

1. Move EH status from "under development" to "supported subset".
2. Document unsupported EH ops clearly.
3. Add migration notes for test-case config (`exceptions: true` or equivalent).

Acceptance:

- README and docs match implemented behavior exactly.

## Validation Commands (completion gate)

- `cargo test`
- `cargo clippy --all-targets --all-features`
- `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml -- <new-exception-case-ids>`
- plus representative existing trap regressions:
  - `call_indirect_oob_trap`
  - `unreachable_trap`
  - `invalid_memory_load_trap`
  - `division_by_zero_trap`
  - `callstack_overflow_trap`

## Risks and Mitigations

1. Tail-call optimization bypassing EH checks
- Mitigation: disable tail-call fusion under EH scope first; re-enable later with explicit proof tests.

2. Optimizer removing EH-critical state ops
- Mitigation: classify EH ops as side effects and add targeted DCE tests.

3. Harness/compiler friction for generating EH wasm
- Mitigation: support per-case EH compile flags and/or prebuilt wasm fixture mode.

4. Scope creep (`try_table`, `throw_ref`, payload arity)
- Mitigation: strict V1 subset with explicit validator errors and follow-up milestone.

## PR-by-PR Execution Plan

The phase plan above is split here into review-friendly, mergeable PR slices.
Each PR should keep `main` green and avoid mixing unrelated cleanup.

### PR 1 - Uncaught exception status plumbing

Goal: reserve runtime status code `-6` without enabling EH ops yet.

Scope:

- Add `TrapCode::UncaughtException = -6` in `src/ir8.rs`.
- Update trap printers in `src/print.rs`.
- Add support text and paused-state handling for `-6` in `src/emit/support.rs` and `src/emit/base.html`.
- Extend blackbox probe terminal PC sets in `tests/blackbox-runner/src/probe.template.js`.

Out of scope:

- Parsing or lowering EH operators.

Merge gate:

- `cargo test`
- `cargo clippy --all-targets --all-features`
- `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml -- call_indirect_oob_trap unreachable_trap`

### PR 2 - Tag metadata and validator subset

Goal: decode tags and validate the supported EH subset.

Scope:

- Add tag/imported-tag metadata in `src/module/mod.rs`.
- Allow EH subset in `src/validate/operators.rs` (`try`, `catch`, `catch_all`, `throw`, `rethrow`, `delegate`).
- Keep unsupported EH ops rejected with precise messages (`try_table`, `throw_ref`, reference payloads).
- Add validator tests for acceptance/rejection matrix.

Out of scope:

- Parser/lowering behavior changes.

Merge gate:

- `cargo test`
- `cargo clippy --all-targets --all-features`

### PR 3 - AST + parser + printer support

Goal: parse EH into structured AST.

Scope:

- Add EH AST nodes in `src/ast.rs`.
- Extend parser frames and control-flow handling in `src/parse/frame.rs` and `src/parse/mod.rs`.
- Add AST printer coverage in `src/print.rs`.
- Add focused parser tests (including reachability around throw/rethrow/delegate).

Out of scope:

- IR and backend EH execution semantics.

Merge gate:

- `cargo test`
- `cargo clippy --all-targets --all-features`

### PR 4 - IR model and lowering semantics

Goal: lower EH AST into CFG with explicit exceptional edges.

Scope:

- Add EH-relevant IR forms in `src/ir.rs`.
- Lower try/catch/throw/rethrow/delegate in `src/lower.rs`.
- Add post-call exception checks and propagation edges.
- Add lowering unit tests for handler selection and uncaught propagation.

Out of scope:

- Final IR8 runtime materialization.

Merge gate:

- `cargo test`
- `cargo clippy --all-targets --all-features`

### PR 5 - Lower8/backend integration

Goal: materialize EH semantics in IR8 and backend passes.

Scope:

- Implement exception state channel lowering in `src/lower8/mod.rs`, `src/lower8/ops.rs`, and `src/lower8/calls.rs`.
- Route uncaught exception to `Trap(-6)`.
- Update `src/regalloc.rs`, `src/schedule.rs`, and `src/opt8/mod.rs` for new ops/terminators and side-effect constraints.
- Add unit coverage for IR8 generation and pass compatibility.

Out of scope:

- New blackbox EH cases.

Merge gate:

- `cargo test`
- `cargo clippy --all-targets --all-features`

### PR 6 - End-to-end runtime + blackbox coverage

Goal: verify EH behavior from wasm input to rendered runtime output.

Scope:

- Finalize emitter/runtime behavior in `src/emit/logic.rs`, `src/emit/support.rs`, and `src/emit/tests.rs`.
- Add blackbox cases and expected outcomes in `tests/cases/*` and `tests/cases.json`:
  - `exception_local_try_catch`
  - `exception_cross_function_catch`
  - `exception_rethrow_outer_catch`
  - `exception_delegate_to_outer`
  - `exception_uncaught_trap`
- Add any needed case-level compile knobs in `tests/blackbox-runner/src/runner.rs`.

Out of scope:

- Payload arity expansion beyond V1.

Merge gate:

- `cargo test`
- `cargo clippy --all-targets --all-features`
- `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml -- exception_local_try_catch exception_cross_function_catch exception_rethrow_outer_catch exception_delegate_to_outer exception_uncaught_trap`
- `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml -- call_indirect_oob_trap unreachable_trap invalid_memory_load_trap division_by_zero_trap callstack_overflow_trap`

### PR 7 - Docs + rollout

Goal: align repository docs with shipped V1 behavior.

Scope:

- Update EH status and supported subset in `README.md`.
- Keep `docs/exception-handling-plan.md` aligned with final implementation details.
- Add concise migration notes for new test-case config fields (if introduced).

Merge gate:

- `cargo test`

### PR 8 (optional) - V1.1 single-i32 payload support

Goal: extend V1 to support one `i32` payload per tag.

Scope:

- Extend validator signature checks for one-arg payloads.
- Extend parser/lowering/backend payload transport.
- Add blackbox payload round-trip and catch tests.

Merge gate:

- `cargo test`
- `cargo clippy --all-targets --all-features`
- `cargo run --release --manifest-path tests/blackbox-runner/Cargo.toml -- <new-payload-case-ids>`

## Dependency Order

Recommended merge order: `PR1 -> PR2 -> PR3 -> PR4 -> PR5 -> PR6 -> PR7`, then `PR8`.
