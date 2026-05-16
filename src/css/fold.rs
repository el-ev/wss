//! AST-level fold passes — the structural replacement for the
//! byte-level rewrites in `src/emit/inline.rs::simplify_calc` and
//! `fold_compare_funcs`.
//!
//! [`fold`] walks the node tree bottom-up and applies a small set of
//! peephole rules at every node until a fixed point. Operating on the
//! AST instead of strings eliminates the bug class where a string
//! pass strips a `calc(...)` wrapper that was actually shielding a
//! non-math function argument — the AST never confuses the two
//! contexts because they're distinct enum variants.

use super::{Arm, Node, Sign, Term};

/// Run all simplification rules to fixpoint.
pub fn fold(node: Node) -> Node {
    let mut current = fold_step(node);
    loop {
        let next = fold_step(current.clone());
        if next == current {
            return current;
        }
        current = next;
    }
}

/// One round of bottom-up simplification.
fn fold_step(node: Node) -> Node {
    let node = fold_children(node);
    fold_local(node)
}

/// Recurse into children of `node`, leaving the shape of the parent
/// alone. Each child is fully simplified before we look at the parent.
fn fold_children(node: Node) -> Node {
    match node {
        Node::Int(_) | Node::Raw(_) | Node::Style { .. } => node,
        Node::Var { name, fallback } => Node::Var {
            name,
            fallback: fallback.map(|fb| Box::new(fold_step(*fb))),
        },
        Node::Calc(inner) => Node::Calc(Box::new(fold_step(*inner))),
        Node::MathFn { name, args } => Node::MathFn {
            name,
            args: args.into_iter().map(fold_step).collect(),
        },
        Node::Fn { name, args } => Node::Fn {
            name,
            args: args.into_iter().map(fold_step).collect(),
        },
        Node::Sum(terms) => Node::Sum(
            terms
                .into_iter()
                .map(|t| Term {
                    sign: t.sign,
                    node: fold_step(t.node),
                })
                .collect(),
        ),
        Node::Product(factors) => Node::Product(factors.into_iter().map(fold_step).collect()),
        Node::Div(l, r) => Node::Div(Box::new(fold_step(*l)), Box::new(fold_step(*r))),
        Node::Paren(inner) => Node::Paren(Box::new(fold_step(*inner))),
        Node::If { arms, default } => Node::If {
            arms: arms
                .into_iter()
                .map(|a| Arm {
                    cond: fold_step(a.cond),
                    value: fold_step(a.value),
                })
                .collect(),
            default: Box::new(fold_step(*default)),
        },
        Node::Or(conds) => Node::Or(conds.into_iter().map(fold_step).collect()),
    }
}

/// Apply local rewrites to `node` assuming its children are already
/// simplified. May produce a node that requires another round (callers
/// drive to fixpoint).
fn fold_local(node: Node) -> Node {
    match node {
        Node::Fn { name, args } => fold_fn(&name, args),
        Node::Calc(inner) => fold_calc(*inner),
        Node::Sum(terms) => fold_sum(terms),
        Node::Product(factors) => fold_product(factors),
        Node::Paren(inner) => fold_paren(*inner),
        Node::If { arms, default } => fold_if(arms, *default),
        Node::MathFn { name, args } => fold_math_fn(name, args),
        other => other,
    }
}

/// `mod(X * K, M)` where K divides M → `mod(X, M/K) * K`. Both forms
/// produce the same integer result (proof: `X·K = (X/q)·M + (X mod q)·K`
/// where `q = M/K`, so the first term vanishes mod M and the second is
/// already in [0, M)). The rewritten form keeps the inner mod range
/// smaller, which is the actual speed win for CSS renderers that
/// internally use wider integers for the multiplication; bytes are
/// near-neutral.
fn fold_math_fn(name: String, args: Vec<Node>) -> Node {
    if name == "mod"
        && let [num, denom] = args.as_slice()
        && let Some(m) = as_int(denom)
        && m > 0
    {
        // `mod(mod(X, A), B)` where B divides A → `mod(X, B)`. The
        // inner mod is redundant since the outer one is stricter.
        if let Node::MathFn {
            name: inner_name,
            args: inner_args,
        } = num
            && inner_name == "mod"
            && let [inner_num, inner_denom] = inner_args.as_slice()
            && let Some(a) = as_int(inner_denom)
            && a > 0
            && a % m == 0
        {
            return Node::MathFn {
                name: "mod".to_string(),
                args: vec![inner_num.clone(), Node::Int(m)],
            };
        }
        // `mod(X * K, M)` where K divides M → `mod(X, M/K) * K`.
        if let Some((other_factor, k)) = product_with_int_factor(num)
            && k > 0
            && m % k == 0
            && k < m
        {
            let q = m / k;
            return Node::Product(vec![
                Node::MathFn {
                    name: "mod".to_string(),
                    args: vec![other_factor, Node::Int(q)],
                },
                Node::Int(k),
            ]);
        }
    }
    Node::MathFn { name, args }
}

/// If `node` is `Product([..., Int(k), ...])` or `Paren`/`Calc` over
/// the same, return the product of the non-literal factors together
/// with `k`. Used by `mod(X * k, M)` folding above.
fn product_with_int_factor(node: &Node) -> Option<(Node, i64)> {
    match node {
        Node::Product(factors) => {
            let mut others: Vec<Node> = Vec::new();
            let mut k: Option<i64> = None;
            for f in factors {
                if let Some(n) = as_int(f) {
                    if k.is_some() {
                        return None; // multiple literal factors: handled by fold_product
                    }
                    k = Some(n);
                } else {
                    others.push(f.clone());
                }
            }
            let k = k?;
            let other = match others.len() {
                0 => return None,
                1 => others.into_iter().next().unwrap(),
                _ => Node::Product(others),
            };
            Some((other, k))
        }
        Node::Calc(inner) | Node::Paren(inner) => product_with_int_factor(inner),
        _ => None,
    }
}

// ---------------------------------------------------------------------
// Compare/select function folds
// ---------------------------------------------------------------------

fn fold_fn(name: &str, args: Vec<Node>) -> Node {
    match (name, args.as_slice()) {
        ("--eq1", [arg]) => fold_eq1(arg.clone()),
        ("--eqz", [arg]) => fold_eqz(arg.clone()),
        ("--nez", [arg]) => fold_nez(arg.clone()),
        ("--eq", [a, b]) => fold_eq_ne(a.clone(), b.clone(), /*is_eq=*/ true),
        ("--ne", [a, b]) => fold_eq_ne(a.clone(), b.clone(), /*is_eq=*/ false),
        ("--sel", [c, t, f]) => fold_sel(c.clone(), t.clone(), f.clone()),
        ("--lt", [a, b]) => fold_lt_ge(a.clone(), b.clone(), /*is_lt=*/ true),
        ("--ge", [a, b]) => fold_lt_ge(a.clone(), b.clone(), /*is_lt=*/ false),
        _ => Node::Fn {
            name: name.to_string(),
            args,
        },
    }
}

/// Custom-property names that the wss runtime guarantees to be
/// non-negative by construction:
///   - `--_1cs_sp` — the callstack stack pointer. The increment /
///     decrement gates use `--inrange(sp ± 1, 257)`, which is
///     equivalent to `0 ≤ sp ± 1 < 257`, so the pointer can never
///     dip below zero (and never exceed 256).
fn known_nonneg_var(node: &Node) -> bool {
    matches!(
        node,
        Node::Var { name, .. } if name == "--_1cs_sp"
    )
}

fn fold_lt_ge(a: Node, b: Node, is_lt: bool) -> Node {
    let name = if is_lt { "--lt" } else { "--ge" };

    // Constant-constant fold.
    if let (Some(x), Some(y)) = (as_int(&a), as_int(&b)) {
        let result = if is_lt { x < y } else { x >= y };
        return Node::Int(i64::from(result));
    }

    // Known-nonneg LHS vs literal threshold.
    //   --lt(sp, K) where K <= 0 → 0   (since sp >= 0)
    //   --ge(sp, K) where K <= 0 → 1
    if known_nonneg_var(&a)
        && let Some(k) = as_int(&b)
        && k <= 0
    {
        return Node::Int(i64::from(!is_lt));
    }
    // Symmetric: --lt(K, sp) where K < 0 → 1 (negative < nonneg).
    //           --ge(K, sp) where K < 0 → 0
    if known_nonneg_var(&b)
        && let Some(k) = as_int(&a)
        && k < 0
    {
        return Node::Int(i64::from(is_lt));
    }

    Node::Fn {
        name: name.to_string(),
        args: vec![a, b],
    }
}

fn fold_eq1(arg: Node) -> Node {
    if let Some(n) = as_int(&arg) {
        return Node::Int(i64::from(n == 1));
    }
    if is_bool_if(&arg) {
        return arg;
    }
    if let Some(mapped) = distribute_into_if(&arg, |v| i64::from(v == 1)) {
        return mapped;
    }
    Node::Fn {
        name: "--eq1".to_string(),
        args: vec![arg],
    }
}

fn fold_eqz(arg: Node) -> Node {
    if let Some(n) = as_int(&arg) {
        return Node::Int(i64::from(n == 0));
    }
    if is_bool_if(&arg) {
        // `calc(1 - <bool>)`
        return Node::Calc(Box::new(Node::Sum(vec![
            Term::pos(Node::Int(1)),
            Term::neg(arg),
        ])));
    }
    if let Some(mapped) = distribute_into_if(&arg, |v| i64::from(v == 0)) {
        return mapped;
    }
    Node::Fn {
        name: "--eqz".to_string(),
        args: vec![arg],
    }
}

fn fold_nez(arg: Node) -> Node {
    if let Some(n) = as_int(&arg) {
        return Node::Int(i64::from(n != 0));
    }
    if is_bool_if(&arg) {
        return arg;
    }
    if let Some(mapped) = distribute_into_if(&arg, |v| i64::from(v != 0)) {
        return mapped;
    }
    Node::Fn {
        name: "--nez".to_string(),
        args: vec![arg],
    }
}

fn fold_eq_ne(a: Node, b: Node, is_eq: bool) -> Node {
    let name = if is_eq { "--eq" } else { "--ne" };

    // Constant-constant fold.
    if let (Some(x), Some(y)) = (as_int(&a), as_int(&b)) {
        let eq = x == y;
        return Node::Int(i64::from(if is_eq { eq } else { !eq }));
    }

    if let Some((bool_side, lit)) = match (as_int(&a), as_int(&b)) {
        (None, Some(n)) if is_bool_if(&a) => Some((a.clone(), n)),
        (Some(n), None) if is_bool_if(&b) => Some((b.clone(), n)),
        _ => None,
    } {
        return match (is_eq, lit) {
            (true, 1) | (false, 0) => bool_side,
            (true, 0) | (false, 1) => Node::Calc(Box::new(Node::Sum(vec![
                Term::pos(Node::Int(1)),
                Term::neg(bool_side),
            ]))),
            (true, _) => Node::Int(0),
            (false, _) => Node::Int(1),
        };
    }

    // Distribute compare into an if() whose arms are all literals.
    let (if_arg, k) = match (as_int(&a), as_int(&b)) {
        (Some(k), None) => (&b, k),
        (None, Some(k)) => (&a, k),
        _ => {
            return Node::Fn {
                name: name.to_string(),
                args: vec![a, b],
            };
        }
    };
    if let Some(mapped) =
        distribute_into_if(if_arg, |v| i64::from(if is_eq { v == k } else { v != k }))
    {
        return mapped;
    }
    Node::Fn {
        name: name.to_string(),
        args: vec![a, b],
    }
}

fn fold_sel(c: Node, t: Node, f: Node) -> Node {
    // Literal condition → pick the surviving branch.
    if let Some(n) = as_int(&c) {
        return if n != 0 { t } else { f };
    }
    // Identical branches: the condition doesn't matter.
    if t == f {
        return t;
    }
    // Bool result patterns:
    //   --sel(<bool>, 1, 0) ≡ <bool>
    //   --sel(<bool>, 0, 1) ≡ calc(1 - <bool>)
    // Only safe when the condition already returns 0/1, which the
    // booleans recognized by [`is_bool_node`] do by construction.
    if is_bool_node(&c) {
        if as_int(&t) == Some(1) && as_int(&f) == Some(0) {
            return c;
        }
        if as_int(&t) == Some(0) && as_int(&f) == Some(1) {
            return Node::Calc(Box::new(Node::Sum(vec![
                Term::pos(Node::Int(1)),
                Term::neg(c),
            ])));
        }
    }
    // `--sel(calc(1 - <bool>), t, f)` flips its branches into
    // `--sel(<bool>, f, t)`, letting downstream folds catch the bool
    // form.
    if let Some(unwrapped) = peel_one_minus_bool(&c) {
        return Node::Fn {
            name: "--sel".to_string(),
            args: vec![unwrapped, f, t],
        };
    }
    // `--sel(min(K, X), t, f)` with a positive literal `K` is just
    // `--sel(X, t, f)` — `--sel` only tests `c == 0`, and `min(K, X)`
    // is zero iff `X` is zero when `K > 0`.
    if let Some(unwrapped) = peel_min_positive_clamp(&c) {
        return Node::Fn {
            name: "--sel".to_string(),
            args: vec![unwrapped, t, f],
        };
    }
    Node::Fn {
        name: "--sel".to_string(),
        args: vec![c, t, f],
    }
}

/// Returns `Some(X)` when `node` is `min(K, X)` or `min(X, K)` with a
/// positive integer literal `K`. The clamp is truthiness-preserving:
/// `min(K, X) == 0 ⟺ X == 0` when `K > 0`, so a surrounding `--sel`
/// (which only compares against zero) sees the same answer either way.
fn peel_min_positive_clamp(node: &Node) -> Option<Node> {
    let Node::MathFn { name, args } = node else {
        return None;
    };
    if name != "min" || args.len() != 2 {
        return None;
    }
    match (as_int(&args[0]), as_int(&args[1])) {
        (Some(k), None) if k > 0 => Some(args[1].clone()),
        (None, Some(k)) if k > 0 => Some(args[0].clone()),
        _ => None,
    }
}

/// Returns `Some(b)` when `node` matches the shape `calc(1 - <b>)`
/// where `<b>` itself is a recognized boolean expression. Used by
/// [`fold_sel`] to swap branches when the condition is a negated bool.
fn peel_one_minus_bool(node: &Node) -> Option<Node> {
    let Node::Calc(inner) = node else {
        return None;
    };
    let Node::Sum(terms) = inner.as_ref() else {
        return None;
    };
    if terms.len() != 2 {
        return None;
    }
    let [a, b] = [&terms[0], &terms[1]];
    if a.sign != Sign::Pos || b.sign != Sign::Neg {
        return None;
    }
    if as_int(&a.node) != Some(1) {
        return None;
    }
    if !is_bool_node(&b.node) {
        return None;
    }
    Some(b.node.clone())
}

/// A node is statically known to evaluate to 0 or 1. The boolean
/// recognizer used by [`fold_sel`] and friends to enable the
/// bool-vs-literal shortcuts.
fn is_bool_node(node: &Node) -> bool {
    match node {
        Node::Int(0) | Node::Int(1) => true,
        // The comparison helpers always return 0 or 1.
        Node::Fn { name, .. } => matches!(
            name.as_str(),
            "--eq" | "--ne" | "--eq1" | "--eqz" | "--nez" | "--lt" | "--ge" | "--inrange"
        ),
        Node::If { arms, default } => is_bool_if(&Node::If {
            arms: arms.clone(),
            default: default.clone(),
        }),
        Node::Calc(inner) | Node::Paren(inner) => is_bool_node(inner),
        _ => false,
    }
}

/// Returns Some(integer) if `node` is just an integer literal — possibly
/// wrapped in trivial Calc/Paren layers stripped by the simplifier.
fn as_int(node: &Node) -> Option<i64> {
    match node {
        Node::Int(n) => Some(*n),
        Node::Calc(inner) | Node::Paren(inner) => as_int(inner),
        _ => None,
    }
}

/// `node` evaluates to a boolean (always 0 or 1) when it's an `if`
/// whose every arm value and the default are integer 0/1 literals.
fn is_bool_if(node: &Node) -> bool {
    let Node::If { arms, default } = node else {
        return false;
    };
    let bool_lit = |n: &Node| matches!(as_int(n), Some(0) | Some(1));
    if !bool_lit(default) {
        return false;
    }
    arms.iter().all(|arm| bool_lit(&arm.value))
}

/// If `node` is `if(arms; else: V)` and every value (arms + default)
/// is an integer literal, rebuild the `If` with each value mapped
/// through `f`.
fn distribute_into_if(node: &Node, mut f: impl FnMut(i64) -> i64) -> Option<Node> {
    let Node::If { arms, default } = node else {
        return None;
    };
    let default_v = as_int(default)?;
    let mut new_arms = Vec::with_capacity(arms.len());
    for arm in arms {
        let v = as_int(&arm.value)?;
        new_arms.push(Arm {
            cond: arm.cond.clone(),
            value: Node::Int(f(v)),
        });
    }
    Some(Node::If {
        arms: new_arms,
        default: Box::new(Node::Int(f(default_v))),
    })
}

// ---------------------------------------------------------------------
// Arithmetic / wrapper folds
// ---------------------------------------------------------------------

fn fold_calc(inner: Node) -> Node {
    match inner {
        // `calc(<atom>)` is redundant — the atom is already valid as a
        // standalone CSS value. (The printer re-introduces `calc` when
        // the same node appears inside a non-math function argument.)
        Node::Int(_) | Node::Var { .. } | Node::Fn { .. } | Node::If { .. } | Node::Raw(_) => inner,
        Node::Calc(deeper) => Node::Calc(deeper),
        // `calc((X))` collapses to `calc(X)`.
        Node::Paren(inner) => Node::Calc(inner),
        other => Node::Calc(Box::new(other)),
    }
}

fn fold_paren(inner: Node) -> Node {
    match inner {
        // `(X)` around an atomic value is redundant.
        Node::Int(_) | Node::Var { .. } | Node::Fn { .. } | Node::MathFn { .. } | Node::Raw(_) => {
            inner
        }
        // `((X))` collapses.
        Node::Paren(deeper) => Node::Paren(deeper),
        other => Node::Paren(Box::new(other)),
    }
}

fn fold_sum(terms: Vec<Term>) -> Node {
    // Flatten nested sums, drop zero terms, and merge adjacent integer
    // literals into the running term — but otherwise preserve original
    // term order so the printer doesn't reshuffle `1 - X` into
    // `0 - X + 1`.
    let mut flat: Vec<Term> = Vec::with_capacity(terms.len());
    for term in terms {
        match term.node {
            Node::Sum(inner) => {
                for it in inner {
                    let combined_sign = combine(term.sign, it.sign);
                    push_sum_term(&mut flat, combined_sign, it.node);
                }
            }
            other => push_sum_term(&mut flat, term.sign, other),
        }
    }
    // Collect identical positive terms — `X + X + X` becomes `X * 3`.
    // Also cancel matching pos/neg pairs (`X - X` → drop both).
    combine_like_terms(&mut flat);
    match flat.len() {
        0 => Node::Int(0),
        1 if flat[0].sign == Sign::Pos => flat.into_iter().next().unwrap().node,
        _ => Node::Sum(flat),
    }
}

/// In-place pass over a sum's term list that:
///   - Cancels `X` and `-X` against each other.
///   - Collapses N copies of `X` (all with the same sign) into a
///     single `Product([X, Int(N)])`. The reverse rewrite (a
///     Product whose factor is `Int(1)`) is already simplified by
///     `fold_product`.
fn combine_like_terms(flat: &mut Vec<Term>) {
    // Use indices because we need to mutate `flat` while iterating.
    let mut i = 0;
    while i < flat.len() {
        let mut total: i64 = match flat[i].sign {
            Sign::Pos => 1,
            Sign::Neg => -1,
        };
        // Don't collapse pure integer literals — those are handled by
        // the running constant accumulator in `push_sum_term`.
        let key = flat[i].node.clone();
        if matches!(key, Node::Int(_)) {
            i += 1;
            continue;
        }
        let mut j = i + 1;
        while j < flat.len() {
            if flat[j].node == key {
                match flat[j].sign {
                    Sign::Pos => total += 1,
                    Sign::Neg => total -= 1,
                }
                flat.remove(j);
            } else {
                j += 1;
            }
        }
        if total == 0 {
            flat.remove(i);
            continue;
        }
        let (sign, mag) = if total > 0 {
            (Sign::Pos, total)
        } else {
            (Sign::Neg, -total)
        };
        flat[i].sign = sign;
        if mag == 1 {
            flat[i].node = key;
        } else {
            flat[i].node = Node::Product(vec![key, Node::Int(mag)]);
        }
        i += 1;
    }
}

fn push_sum_term(flat: &mut Vec<Term>, sign: Sign, node: Node) {
    match &node {
        // Drop literal zeros entirely.
        Node::Int(0) => return,
        // `0 * X` factor anywhere kills the term.
        Node::Product(factors) if factors.iter().any(|f| matches!(f, Node::Int(0))) => return,
        _ => {}
    }
    // Try to merge an integer literal into the most recent literal
    // term, accumulating constants without reshuffling order.
    if let Node::Int(n) = &node
        && let Some(last) = flat.last_mut()
        && let Node::Int(prev) = last.node
    {
        let prev_signed = if last.sign == Sign::Pos { prev } else { -prev };
        let new_signed = if sign == Sign::Pos { *n } else { -*n };
        let total = prev_signed + new_signed;
        if total == 0 {
            flat.pop();
        } else if total > 0 {
            last.sign = Sign::Pos;
            last.node = Node::Int(total);
        } else {
            last.sign = Sign::Neg;
            last.node = Node::Int(-total);
        }
        return;
    }
    flat.push(Term { sign, node });
}

fn combine(a: Sign, b: Sign) -> Sign {
    match (a, b) {
        (Sign::Pos, Sign::Pos) | (Sign::Neg, Sign::Neg) => Sign::Pos,
        _ => Sign::Neg,
    }
}

fn fold_product(factors: Vec<Node>) -> Node {
    // Flatten nested products and apply identities.
    let mut flat: Vec<Node> = Vec::with_capacity(factors.len());
    let mut const_acc: i64 = 1;
    let mut saw_zero = false;
    for f in factors {
        match f {
            Node::Product(inner) => {
                for it in inner {
                    if !push_factor(&mut flat, &mut const_acc, &mut saw_zero, it) {
                        return Node::Int(0);
                    }
                }
            }
            other => {
                if !push_factor(&mut flat, &mut const_acc, &mut saw_zero, other) {
                    return Node::Int(0);
                }
            }
        }
    }
    if saw_zero {
        return Node::Int(0);
    }
    if const_acc != 1 {
        flat.push(Node::Int(const_acc));
    }
    match flat.len() {
        0 => Node::Int(1),
        1 => flat.into_iter().next().unwrap(),
        _ => Node::Product(flat),
    }
}

/// Returns `false` if the factor zeroes the entire product.
fn push_factor(flat: &mut Vec<Node>, const_acc: &mut i64, saw_zero: &mut bool, node: Node) -> bool {
    match &node {
        Node::Int(0) => {
            *saw_zero = true;
            false
        }
        Node::Int(1) => true,
        Node::Int(n) => {
            *const_acc *= *n;
            true
        }
        _ => {
            flat.push(node);
            true
        }
    }
}

fn fold_if(arms: Vec<Arm>, default: Node) -> Node {
    // Drop arms whose value equals the default — they're already
    // covered by the else clause, so emitting them just adds bytes.
    let arms: Vec<Arm> = arms.into_iter().filter(|a| a.value != default).collect();
    // If every arm has been dropped (or there were none to begin with),
    // the whole if collapses to its default.
    if arms.is_empty() {
        return default;
    }
    Node::If {
        arms,
        default: Box::new(default),
    }
}
