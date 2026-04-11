# IR8 Optimization Report

Generated 2026-04-09 from 150 compiled test cases in `tests/out/`.

## Current Optimizer Performance

The opt8 pass pipeline (copy propagation, copy elimination, instcombine,
boolean chain combination, dead memory store elimination, goto threading,
DCE, unreachable block removal, block coalescing) achieves a **35.9%
instruction reduction** across the test suite:

| Metric | Count |
|--------|------:|
| Pre-opt instructions | 71,086 |
| Post-opt instructions | 45,533 |
| Eliminated | 25,553 (35.9%) |

## Post-Opt Instruction Breakdown

The 45,533 remaining instructions break down as:

| Category | Instructions | Share |
|----------|------------:|------:|
| add32 (b0-b3) | 16,028 | 35.2% |
| sel | 4,498 | 9.9% |
| sub32 (b0-b3 + borrow) | 5,811 | 12.8% |
| load.mem | 2,441 | 5.4% |
| copy | 2,197 | 4.8% |
| add / carry | 3,015 | 6.6% |
| mul (lo/hi) | 1,822 | 4.0% |
| store.mem | 1,176 | 2.6% |
| cs.store / cs.load / cs.* | 2,495 | 5.5% |
| comparisons (ne/eq/lt_u/ge_u) | 2,446 | 5.4% |
| bitwise (and/or/xor) | 2,119 | 4.7% |
| bool (and/or/not) | 1,308 | 2.9% |
| other | 177 | 0.4% |

---

## Optimization Opportunity #1: Store-Load Forwarding

**Estimated impact: eliminates 1,742 of 2,441 loads (71.4%)**

### The Problem

The wasm-to-IR8 lowering produces stack-frame access patterns where a value
is stored to linear memory and then immediately (or shortly after) loaded
back. The existing optimizer does not forward stored values to subsequent
loads.

Example from `arithmetic_basic`, post-opt:

```
store.mem [0xc+%r4:%r5] lane=0 0x07      // store constant 7 to stack
store.mem [0xc+%r4:%r5] lane=1 0x00
store.mem [0xe+%r4:%r5] lane=0 0x00
store.mem [0xe+%r4:%r5] lane=1 0x00
...
%r68 = load.mem [0xc+%r4:%r5] lane=0     // load it right back!
%r69 = load.mem [0xc+%r4:%r5] lane=1
%r70 = load.mem [0xe+%r4:%r5] lane=0
%r71 = load.mem [0xe+%r4:%r5] lane=1
```

After forwarding, these loads become trivial copies:

```
%r68 = copy 0x07
%r69 = copy 0x00
%r70 = copy 0x00
%r71 = copy 0x00
```

Which are then absorbed by copy propagation and instcombine.

### Scope

Two sub-patterns, both handled by the same forward-walk pass:

| Pattern | Count | % of loads |
|---------|------:|----------:|
| Store-then-load (same addr, no intervening write) | 1,265 | 51.8% |
| Redundant load (same addr loaded twice, no intervening write) | 477 | 19.5% |
| **Total eliminable** | **1,742** | **71.4%** |

The analysis tracks forwarding across basic block boundaries when the block
split is caused by a builtin call (builtins are pure arithmetic and do not
touch linear memory). Tracking is invalidated on user-function calls and on
redefinition of address registers.

### Worst Cases

Cases where store-load forwarding has the highest proportional impact:

| Case | Loads | Elim | Elim % of all instrs |
|------|------:|-----:|---------------------:|
| bitwise_shift | 32 | 32 | 76.2% |
| arithmetic_basic | 32 | 32 | 64.0% |
| switch_loop_bitcount_optnone | 109 | 105 | 52.0% |
| nested_switch_walk | 111 | 107 | 45.0% |
| bitcount_relop_blend | 78 | 70 | 46.4% |
| relops_signed_unsigned_full | 116 | 108 | 44.1% |
| switch_br_table_optnone | 74 | 73 | 43.7% |
| pointer_fold_loop | 66 | 58 | 38.9% |
| control_switch_weave | 67 | 63 | 35.6% |
| up8cc_control_if_chain | 16 | 12 | 33.3% |

94 out of 150 test cases have zero eliminable loads (already clean).

### Cascade Effects

Eliminating loads produces copies that feed into existing passes:

1. **Copy propagation** absorbs the new `copy` instructions
2. **Instcombine** folds forwarded constants into downstream operations
3. **DCE** removes stores that no longer have any live readers
4. Estimated ~1,265 stores become dead after their loads are eliminated

### Implementation

A single forward walk per function, tracking `(addr, base+lane)` -> value:

- On `store.mem`: record the stored value
- On `load.mem`: if tracked, replace with `copy stored_value`
- On `call_setup` to user function: invalidate all entries
- On `call_setup` to builtin: keep tracking (builtins are pure)
- On redefinition of an address register: invalidate affected entries

Fits naturally into the existing fixed-point optimization loop.

---

## Optimization Opportunity #2: Constant-Shift Inlining

**Estimated impact: eliminates 103 of 153 builtin calls (67.3%)**

### The Problem

Shift and rotate operations are lowered to builtin calls
(`builtin.shl_32`, `builtin.shr_u32`, `builtin.shr_s32`, etc.) which
require a `call_setup` terminator — splitting the basic block and
preventing further optimization across the call boundary.

103 of 153 builtin calls (67.3%) have a **compile-time-constant shift
amount**. These could be expanded inline as byte shuffles and small
shifts, eliminating the call overhead and enabling the block to be
coalesced.

Example from `loop_if_else`:

```
call_setup builtin.shl_32 cont=$B5
    args=[(%r76:%r77:%r78:%r79), (0x01:0x00:0x00:0x00)]
```

A shift-left by 1 is mechanically:

```
%hi = shl %r79 1        // or: add %r79 %r79
%t2 = shr %r78 7
%b2 = shl %r78 1
%b2 = or  %b2 %t2       // ... etc for each byte lane
```

### Benefit

- Eliminates block splits, enabling coalescing and further local optimization
- Each inlined shift is ~4-8 byte-level ops vs a full call round-trip
- Particularly valuable inside loops (e.g., `loop_if_else`, `pointer_fold_loop`)

---

## Optimization Opportunity #3: Upper-Byte Narrowing

**Estimated scope: affects 48.4% of add32 and 96.6% of ne instructions**

### The Problem

Many 32-bit operations operate on values that are known to fit in fewer
bytes. Loop counters, array indices, and small constants produce `add32`
instructions where the upper 3 bytes of the RHS are `0x00`:

```
%r130 = add32.b0 (%r4:%r5:%r6:%r7) (0x01:0x00:0x00:0x00)
%r131 = add32.b1 (%r4:%r5:%r6:%r7) (0x01:0x00:0x00:0x00)
%r132 = add32.b2 (%r4:%r5:%r6:%r7) (0x01:0x00:0x00:0x00)  // carry-only
%r133 = add32.b3 (%r4:%r5:%r6:%r7) (0x01:0x00:0x00:0x00)  // carry-only
```

Similarly, 96.6% of `ne` instructions are `ne %rN 0x00` — testing upper
bytes of values known to be small. These feed into `bool.or` chains for
32-bit comparisons:

```
%r118 = ne  %r110 0x07   // meaningful comparison (byte 0)
%r119 = ne  %r111 0x00   // always false if counter < 256
%r120 = ne  %r112 0x00   // always false if counter < 256
%r121 = ne  %r113 0x00   // always false if counter < 256
%r122 = bool.or  %r118 %r119 %r120 %r121
```

If value-range analysis proves the upper bytes are always zero, the `ne`s
fold to `false` and the `bool.or` simplifies to just `%r118`.

| Pattern | Count | % of kind |
|---------|------:|----------:|
| add32 with small-constant RHS (upper 3 bytes = 0x00) | 7,756 | 48.4% |
| ne against 0x00 | 1,309 | 96.6% |
| copy of immediate | 541 | 24.6% |

### Complexity

This requires per-byte value-range tracking (or at minimum, known-zero-byte
propagation) through the dataflow graph. More complex than store-load
forwarding but high potential payoff given the instruction volumes.

---

## Recommendation

**Priority order by impact-to-complexity ratio:**

1. **Store-load forwarding** — highest impact, straightforward local
   analysis, eliminates 1,742 loads + ~1,265 dead stores. Integrates
   naturally into the existing pass pipeline. Should be implemented first.

2. **Constant-shift inlining** — eliminates 103 builtin calls, unblocks
   further optimization by removing block splits. Medium complexity
   (need per-shift-amount expansion templates).

3. **Upper-byte narrowing** — largest raw instruction count impact (affects
   ~48% of all add32) but requires dataflow analysis infrastructure.
   Best tackled after (1) and (2) reduce the IR and expose simpler patterns.
