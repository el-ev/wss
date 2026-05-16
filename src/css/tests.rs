use super::{Node, parse};

/// Parse → print roundtrip on a handful of patterns the wss emitter
/// actually produces. Some inputs aren't bit-equal after the roundtrip
/// because the printer normalizes whitespace, but the resulting CSS
/// must parse to the same AST.
fn idempotent(input: &str) {
    let first = parse(input);
    let printed = first.to_css();
    let second = parse(&printed);
    assert_eq!(
        first,
        second,
        "AST changed after roundtrip\ninput:   {}\nprinted: {}\nfirst:   {}\nsecond:  {}",
        input,
        printed,
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
fn if_or_chain() {
    let input = "if(style(--_1pc: 5) or style(--_1pc: 6): 1; else: 0)";
    let n = parse(input);
    assert_eq!(n.to_css(), input);
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
    // Deeply nested sums-of-products from --pc dispatcher.
    idempotent(
        "calc((--lt(var(--_1cs_sp), 0) + --ge(var(--_1cs_sp), 257)) * -5 + (1 - (--lt(var(--_1cs_sp), 0) + --ge(var(--_1cs_sp), 257))) * --sel(--ge(var(--mb0), 509), -3, -1))",
    );
    // Variable with integer fallback.
    idempotent("var(--g0_0, 0)");
    // Or-chain in if-arm condition.
    idempotent(
        "if(style(--_1pc: 2017) or style(--_1pc: 2037) or style(--_1pc: 2088): -1; else: 0)",
    );
    // Memory merge function body slice.
    idempotent(
        "calc((var(--cso0) * --eq(var(--csi0), var(--idx))) * (1 + --eq(var(--csp0), 1)) + (1 - (var(--cso0) * --eq(var(--csi0), var(--idx))) * (1 + --eq(var(--csp0), 1))) * --mlo(var(--prev)))",
    );
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
