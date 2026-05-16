use super::{Node, parse};

fn idempotent(input: &str) {
    let first = parse(input);
    let printed1 = first.to_css();
    let second = parse(&printed1);
    let printed2 = second.to_css();
    assert_eq!(
        printed1,
        printed2,
        "parse → print is not stable\ninput:    {}\nprinted1: {}\nprinted2: {}\nfirst:    {}\nsecond:   {}",
        input,
        printed1,
        printed2,
        first.to_dump(),
        second.to_dump()
    );
}

#[test]
fn int_literal() {
    let n = parse("42");
    assert_eq!(n, Node::Int(42));
    assert_eq!(n.to_css(), "42");
}

#[test]
fn negative_int() {
    let n = parse("-3");
    assert_eq!(n, Node::Int(-3));
    assert_eq!(n.to_css(), "-3");
}

#[test]
fn var_plain() {
    let n = parse("var(--x)");
    assert_eq!(
        n,
        Node::Var {
            name: "--x".into(),
            fallback: None,
        }
    );
    assert_eq!(n.to_css(), "var(--x)");
}

#[test]
fn var_with_fallback() {
    let n = parse("var(--x, 0)");
    assert_eq!(n.to_css(), "var(--x, 0)");
}

#[test]
fn calc_sum() {
    let n = parse("calc(var(--a) + var(--b))");
    assert_eq!(n.to_css(), "calc(var(--a) + var(--b))");
}

#[test]
fn calc_product_in_sum() {
    let n = parse("calc(var(--a) * 2 + var(--b))");
    assert_eq!(n.to_css(), "calc(var(--a) * 2 + var(--b))");
}

#[test]
fn calc_paren_sum_times_var() {
    // Precedence: Sum inside a Product needs explicit parens.
    let n = parse("calc((var(--a) + var(--b)) * var(--c))");
    assert_eq!(n.to_css(), "calc((var(--a) + var(--b)) * var(--c))");
}

#[test]
fn nonmath_fn_arg_arithmetic_wrapped() {
    // --read_cs takes a non-math argument: arithmetic must be in calc().
    let n = parse("--read_cs(calc(var(--sp) + 1))");
    assert_eq!(n.to_css(), "--read_cs(calc(var(--sp) + 1))");
}

#[test]
fn math_fn_strips_redundant_calc() {
    // `min` is a math function — `min(1, calc(X + Y))` is semantically
    // identical to `min(1, (X + Y))`. The parser preserves the calc;
    // we still re-emit the same shape (fold pass would strip).
    let n = parse("min(1, calc(var(--a) + var(--b)))");
    // We don't auto-strip here; assert roundtrip parses identically.
    idempotent("min(1, calc(var(--a) + var(--b)))");
    let _ = n;
}

#[test]
fn if_simple() {
    let n = parse("if(style(--_1pc: 5): 1; else: 0)");
    let s = n.to_css();
    assert_eq!(s, "if(style(--_1pc: 5): 1; else: 0)");
}

#[test]
fn if_or_chain_collapses_same_prop_to_compact() {
    // `style(--p: a) or style(--p: b)` prints as the compact
    // `style((--p: a) or (--p: b))` per CSS-spec since both
    // clauses share `--p`.
    let input = "if(style(--_1pc: 5) or style(--_1pc: 6): 1; else: 0)";
    let n = parse(input);
    assert_eq!(
        n.to_css(),
        "if(style((--_1pc: 5) or (--_1pc: 6)): 1; else: 0)"
    );
}

#[test]
fn if_or_chain_keeps_distinct_props_separate() {
    // Different props → no collapse, each clause keeps its own
    // `style()` wrapper.
    let input = "if(style(--a: 1) or style(--b: 2): 1; else: 0)";
    let n = parse(input);
    assert_eq!(n.to_css(), input);
}

#[test]
fn parses_compact_style_or() {
    let input = "if(style((--p: 1) or (--p: 2)): 1; else: 0)";
    let n = parse(input);
    let printed = n.to_css();
    // Stable roundtrip through the compact form.
    assert_eq!(printed, input);
}

#[test]
fn if_multi_arm() {
    let input = "if(style(--_1pc: 5): 1; style(--_1pc: 6): 2; else: 0)";
    let n = parse(input);
    assert_eq!(n.to_css(), input);
}

#[test]
fn nested_fn() {
    let input = "--sel(--ge(var(--mb0), 461), -3, 2001)";
    let n = parse(input);
    assert_eq!(n.to_css(), input);
}

#[test]
fn sum_with_negative_term() {
    let input = "calc(1 - var(--x))";
    let n = parse(input);
    assert_eq!(n.to_css(), "calc(1 - var(--x))");
}

#[test]
fn product_in_sum_in_product() {
    let input = "calc((1 - var(--x)) * var(--y))";
    let n = parse(input);
    assert_eq!(n.to_css(), input);
}

#[test]
fn idempotent_real_world_samples() {
    // These are direct snippets from generated examples.
    idempotent("var(--cri0)");
    idempotent("calc(var(--cri0) * 2)");
    idempotent("--eq(var(--csi0), 2)");
    idempotent("--sel(--ge(var(--mb0), 461), -3, 2001)");
    idempotent("if(style(--_1pc: 5): 1; else: 0)");
    idempotent("if(style(--_1pc: 5) or style(--_1pc: 6): 1; else: 0)");
    idempotent("min(1, calc(var(--a) * var(--b)))");
    idempotent("--mload(calc(var(--mb0) + 512))");
    idempotent("--inrange(var(--_1cs_sp), 64)");
    idempotent("--inrange(calc(var(--_1cs_sp) + 1), 257)");
}

#[test]
fn dump_shape() {
    let n = parse("calc(var(--a) * 2 + 1)");
    let dump = n.to_dump();
    // We don't pin every detail of the dump, just that the major
    // structural lines are present.
    assert!(dump.contains("Calc"));
    assert!(dump.contains("Sum"));
    assert!(dump.contains("Product"));
    assert!(dump.contains("Int(2)"));
    assert!(dump.contains("Int(1)"));
    assert!(dump.contains("Var --a"));
}

#[test]
fn raw_fallback_on_unknown() {
    // Something we can't model becomes Raw and prints verbatim.
    let input = "weirdthing#yes";
    let n = parse(input);
    assert!(matches!(n, Node::Raw(_)));
    assert_eq!(n.to_css(), input);
}

// =====================================================================
// fold() tests — AST-level peephole rewrites used by stage 2 of the
// css-AST refactor. Mirrors what `inline.rs::fold_compare_funcs` +
// `simplify_calc` do, but with structural guarantees instead of
// byte-level rewriting.
// =====================================================================

use super::fold;

fn fold_str(input: &str) -> String {
    fold(parse(input)).to_css()
}

#[test]
fn fold_eq1_literal() {
    assert_eq!(fold_str("--eq1(0)"), "0");
    assert_eq!(fold_str("--eq1(1)"), "1");
    assert_eq!(fold_str("--eq1(7)"), "0");
}

#[test]
fn fold_eqz_nez_literal() {
    assert_eq!(fold_str("--eqz(0)"), "1");
    assert_eq!(fold_str("--eqz(3)"), "0");
    assert_eq!(fold_str("--nez(0)"), "0");
    assert_eq!(fold_str("--nez(5)"), "1");
}

#[test]
fn fold_eq_ne_const_const() {
    assert_eq!(fold_str("--eq(2, 2)"), "1");
    assert_eq!(fold_str("--eq(2, 5)"), "0");
    assert_eq!(fold_str("--ne(2, 2)"), "0");
    assert_eq!(fold_str("--ne(2, 5)"), "1");
}

#[test]
fn fold_sel_literal_cond() {
    assert_eq!(fold_str("--sel(1, 7, 9)"), "7");
    assert_eq!(fold_str("--sel(0, 7, 9)"), "9");
    assert_eq!(fold_str("--sel(42, var(--x), 0)"), "var(--x)");
}

#[test]
fn fold_sel_identical_branches() {
    // `--sel(c, x, x)` ≡ `x` regardless of `c`.
    assert_eq!(fold_str("--sel(var(--c), 5, 5)"), "5");
    assert_eq!(
        fold_str("--sel(--eq(var(--a), var(--b)), var(--x), var(--x))"),
        "var(--x)"
    );
}

#[test]
fn fold_sel_bool_result_collapses_to_cond() {
    // `--sel(<bool>, 1, 0)` ≡ `<bool>`.
    let r = fold_str("--sel(--eq(var(--a), 2), 1, 0)");
    assert_eq!(r, "--eq(var(--a), 2)");
    // `--sel(<bool>, 0, 1)` ≡ `calc(1 - <bool>)`.
    let r = fold_str("--sel(--eq(var(--a), 2), 0, 1)");
    assert_eq!(r, "calc(1 - --eq(var(--a), 2))");
}

#[test]
fn fold_sel_one_minus_bool_swaps_branches() {
    // `--sel(calc(1 - <bool>), t, f)` ≡ `--sel(<bool>, f, t)`.
    let r = fold_str("--sel(calc(1 - --eq(var(--a), 2)), var(--x), var(--y))");
    assert_eq!(r, "--sel(--eq(var(--a), 2), var(--y), var(--x))");
}

#[test]
fn fold_sel_strips_min_positive_clamp() {
    // `--sel(min(1, X), t, f)` ≡ `--sel(X, t, f)`: `--sel` only tests
    // for zero, and `min(1, X) == 0 ⟺ X == 0`.
    let r = fold_str("--sel(min(1, calc(var(--a) + var(--b))), 2011, 2023)");
    assert_eq!(r, "--sel(calc(var(--a) + var(--b)), 2011, 2023)");
    // Literal on either side, any positive value works.
    let r = fold_str("--sel(min(var(--x), 5), 1, 0)");
    assert_eq!(r, "--sel(var(--x), 1, 0)");
    // K = 0 is *not* truthiness-preserving (min(0, -3) = -3 ≠ 0).
    let r = fold_str("--sel(min(0, var(--x)), 1, 0)");
    assert_eq!(r, "--sel(min(0, var(--x)), 1, 0)");
}

#[test]
fn fold_eq_bool_against_lit() {
    // `--eq(bool, 1)` ≡ bool.
    let r = fold_str("--eq(if(style(--p: 1): 1; else: 0), 1)");
    assert_eq!(r, "if(style(--p: 1): 1; else: 0)");
    // `--eq(bool, 0)` ≡ calc(1 - bool).
    let r = fold_str("--eq(if(style(--p: 1): 1; else: 0), 0)");
    assert_eq!(r, "calc(1 - if(style(--p: 1): 1; else: 0))");
    // `--ne(bool, 0)` ≡ bool.
    let r = fold_str("--ne(if(style(--p: 1): 1; else: 0), 0)");
    assert_eq!(r, "if(style(--p: 1): 1; else: 0)");
    // Bool compared to out-of-range literal: constant.
    assert_eq!(fold_str("--eq(if(style(--p: 1): 1; else: 0), 7)"), "0");
    assert_eq!(fold_str("--ne(if(style(--p: 1): 1; else: 0), 7)"), "1");
}

#[test]
fn fold_distribute_eq_into_if() {
    let r = fold_str("--eq(if(style(--p: 1): 3; else: 4), 3)");
    assert_eq!(r, "if(style(--p: 1): 1; else: 0)");
    let r = fold_str("--eq(if(style(--p: 1): 3; else: 4), 4)");
    assert_eq!(r, "if(style(--p: 1): 0; else: 1)");
    let r = fold_str("--ne(if(style(--p: 1): 3; else: 4), 4)");
    assert_eq!(r, "if(style(--p: 1): 1; else: 0)");
}

#[test]
fn fold_eq1_of_bool_if() {
    let input = "--eq1(if(style(--p: 1): 1; else: 0))";
    assert_eq!(fold_str(input), "if(style(--p: 1): 1; else: 0)");
}

#[test]
fn fold_calc_drops_zero_terms() {
    assert_eq!(fold_str("calc(var(--a) + 0)"), "var(--a)");
    assert_eq!(fold_str("calc(0 + var(--a))"), "var(--a)");
    assert_eq!(
        fold_str("calc(var(--a) + 0 + var(--b))"),
        "calc(var(--a) + var(--b))"
    );
}

#[test]
fn fold_calc_drops_one_factor() {
    assert_eq!(fold_str("calc(1 * var(--a))"), "var(--a)");
    assert_eq!(fold_str("calc(var(--a) * 1)"), "var(--a)");
    assert_eq!(
        fold_str("calc(var(--a) * 1 + var(--b))"),
        "calc(var(--a) + var(--b))"
    );
}

#[test]
fn fold_calc_kills_zero_product() {
    assert_eq!(fold_str("calc(0 * var(--a))"), "0");
    assert_eq!(fold_str("calc(var(--a) * 0)"), "0");
    assert_eq!(fold_str("calc(var(--a) * 0 + var(--b))"), "var(--b)");
}

#[test]
fn fold_calc_combines_like_terms() {
    // `X + X` → `X * 2`, `X + X + X` → `X * 3`.
    assert_eq!(fold_str("calc(var(--a) + var(--a))"), "calc(var(--a) * 2)");
    assert_eq!(
        fold_str("calc(var(--a) + var(--a) + var(--a))"),
        "calc(var(--a) * 3)"
    );
    // `X - X` cancels.
    assert_eq!(fold_str("calc(var(--a) - var(--a))"), "0");
    // `X + X - X` → `X`.
    assert_eq!(fold_str("calc(var(--a) + var(--a) - var(--a))"), "var(--a)");
    // Mixed: only equal-shaped terms combine.
    assert_eq!(
        fold_str("calc(var(--a) + var(--b) + var(--a))"),
        "calc(var(--a) * 2 + var(--b))"
    );
}

#[test]
fn fold_mod_extracts_power_of_two_factor() {
    // `mod(X * K, M)` with K | M → `mod(X, M/K) * K`. The outer
    // `calc(...)` wrap shows up because the resulting Product is no
    // longer inside a math context.
    assert_eq!(
        fold_str("mod(var(--r) * 4, 256)"),
        "calc(mod(var(--r), 64) * 4)"
    );
    assert_eq!(
        fold_str("mod(var(--r) * 16, 256)"),
        "calc(mod(var(--r), 16) * 16)"
    );
    assert_eq!(
        fold_str("mod(var(--r) * 2, 256)"),
        "calc(mod(var(--r), 128) * 2)"
    );
}

#[test]
fn fold_mod_does_not_apply_when_k_does_not_divide_m() {
    // K=3 doesn't divide M=256, so the rewrite isn't applicable.
    let r = fold_str("mod(var(--r) * 3, 256)");
    assert_eq!(r, "mod(var(--r) * 3, 256)");
}

#[test]
fn fold_mod_in_math_context_no_calc_wrap() {
    // The cascade `mod(mod(X*K, M), N)` fully collapses: the inner
    // `mod(X*K, M)` rewrites to `mod(X, M/K)*K`, the outer mod then
    // rewrites the same way, and the redundant inner mod drops out
    // (`mod(mod(X, 64), 16)` → `mod(X, 16)` since 16 | 64). Final
    // shape is `mod(X, 16)*4`, range [0, 60] — already < 64 so the
    // outer mod-64 wrapper would be a no-op and is absent.
    assert_eq!(
        fold_str("mod(mod(var(--r) * 4, 256), 64)"),
        "calc(mod(var(--r), 16) * 4)"
    );
}

#[test]
fn fold_mod_collapses_redundant_nesting() {
    // The inner mod-A is dominated by the outer mod-B (B | A).
    assert_eq!(fold_str("mod(mod(var(--r), 256), 64)"), "mod(var(--r), 64)");
    assert_eq!(fold_str("mod(mod(var(--r), 64), 16)"), "mod(var(--r), 16)");
    // Different moduli that don't divide stay nested.
    assert_eq!(
        fold_str("mod(mod(var(--r), 64), 15)"),
        "mod(mod(var(--r), 64), 15)"
    );
}

#[test]
fn fold_calc_combines_inside_mod() {
    // `var + var` combines to `var * 2`, then the `mod(X*K, M)` rule
    // pulls the literal factor out of the mod (K=2 | M=256), giving
    // `mod(var, 128) * 2`.
    let r = fold_str("mod(calc(var(--_1r108) + var(--_1r108)), 256)");
    assert_eq!(r, "calc(mod(var(--_1r108), 128) * 2)");
}

#[test]
fn fold_calc_constant_fold_sum() {
    assert_eq!(fold_str("calc(1 + 2)"), "3");
    assert_eq!(fold_str("calc(1 - 1)"), "0");
    assert_eq!(fold_str("calc(1 - 0)"), "1");
    // Mixed constant + var: var stays, constants collapse to a single
    // trailing literal.
    assert_eq!(fold_str("calc(var(--x) + 1 + 2)"), "calc(var(--x) + 3)");
}

#[test]
fn fold_unwraps_trivial_calc() {
    assert_eq!(fold_str("calc(var(--x))"), "var(--x)");
    assert_eq!(fold_str("calc(42)"), "42");
    // calc((X)) → calc(X), then if X is atom, drops to X.
    assert_eq!(fold_str("calc((var(--x)))"), "var(--x)");
}

#[test]
fn fold_unwraps_paren_around_atom() {
    assert_eq!(fold_str("calc((var(--x)) + 1)"), "calc(var(--x) + 1)");
    assert_eq!(fold_str("calc((1) + var(--x))"), "calc(1 + var(--x))");
}

#[test]
fn fold_keeps_calc_around_nonmath_fn_arg() {
    // The printer always wraps arithmetic in calc() when emitted as
    // an argument to a non-math function.
    let r = fold_str("--read_cs(calc(var(--sp) + 1))");
    assert_eq!(r, "--read_cs(calc(var(--sp) + 1))");
    // Even after folding inner arithmetic: arg printer re-wraps.
    let r = fold_str("--read_cs(calc(var(--sp) + 0))");
    assert_eq!(r, "--read_cs(var(--sp))");
}

#[test]
fn fold_collapses_identical_if_arms() {
    let r = fold_str("if(style(--p: 1): 0; else: 0)");
    assert_eq!(r, "0");
}

#[test]
fn fold_drops_arms_equal_to_default() {
    // Arms whose value equals the else clause are dead — drop them.
    let r = fold_str("if(style(--p: 1): var(--x); style(--p: 2): var(--x); else: var(--x))");
    assert_eq!(r, "var(--x)");
    // Mixed: keep the distinct arm, drop the redundant one.
    let r = fold_str("if(style(--p: 1): var(--y); style(--p: 2): var(--x); else: var(--x))");
    assert_eq!(r, "if(style(--p: 1): var(--y); else: var(--x))");
}

#[test]
fn fold_idempotent_at_fixpoint() {
    // Running fold twice produces the same result as running it once.
    let n1 = fold(parse("calc(var(--a) * 1 + 0 + --eq(2, 2))"));
    let n2 = fold(n1.clone());
    assert_eq!(n1, n2);
    assert_eq!(n1.to_css(), "calc(var(--a) + 1)");
}
