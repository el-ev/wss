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

use super::opt::static_range;
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
        Node::Int(_)
        | Node::Raw(_)
        | Node::Style { .. }
        | Node::Invalid
        | Node::Ident(_)
        | Node::Str(_)
        | Node::Dim { .. }
        | Node::Hash(_) => node,
        Node::List { sep, items } => Node::List {
            sep,
            items: items.into_iter().map(fold_step).collect(),
        },
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
    // The rewrites below assume unitless integers: `X - X` is `0`, not `0px`.
    if has_dimension(&node) {
        return node;
    }
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

/// A math function whose arguments are all integer literals.
fn eval_math_fn(name: &str, args: &[Node]) -> Option<i64> {
    let ints = || args.iter().map(as_int).collect::<Option<Vec<_>>>();
    Some(match name {
        "round" => {
            let (mode, x) = match args {
                [Node::Ident(mode), x] => (mode.as_str(), x),
                [x] => ("nearest", x),
                _ => return None,
            };
            if let Some(v) = as_int(x) {
                return Some(v);
            }
            let (a, b) = as_int_div(x)?;
            if b <= 0 {
                return None;
            }
            let (q, r) = (a.div_euclid(b), a.rem_euclid(b));
            match mode {
                "down" => q,
                "up" => q + i64::from(r != 0),
                "to-zero" => a / b,
                // Halves round towards +infinity.
                "nearest" => q + i64::from(2 * r >= b),
                _ => return None,
            }
        }
        "mod" | "rem" => {
            let [a, b] = ints()?[..] else { return None };
            if b == 0 {
                return None;
            }
            // `mod` takes the divisor's sign, `rem` the dividend's.
            let r = a % b;
            if name == "mod" && r != 0 && (r < 0) != (b < 0) {
                r + b
            } else {
                r
            }
        }
        "min" => ints()?.into_iter().min()?,
        "max" => ints()?.into_iter().max()?,
        "abs" => as_int(args.first()?)?.abs(),
        "sign" => as_int(args.first()?)?.signum(),
        _ => return None,
    })
}

/// `node` is arithmetic over a value with a unit or a fraction. Function
/// calls, `var()` and `if()` are opaque here: their operands are folded on
/// their own.
fn has_dimension(node: &Node) -> bool {
    match node {
        Node::Dim { .. } => true,
        Node::Calc(inner) | Node::Paren(inner) => has_dimension(inner),
        Node::MathFn { args, .. } => args.iter().any(has_dimension),
        Node::Sum(terms) => terms.iter().any(|t| has_dimension(&t.node)),
        Node::Product(factors) => factors.iter().any(has_dimension),
        Node::Div(l, r) => has_dimension(l) || has_dimension(r),
        _ => false,
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
    if let Some(v) = eval_math_fn(&name, &args) {
        return Node::Int(v);
    }
    if name == "mod"
        && let [num, denom] = args.as_slice()
        && let Some(m) = as_int(denom)
        && m > 0
    {
        // `mod(X, M)` is `X` when `X` already lies in `0..M` (a register
        // under `mod(_, 256)`, a bool under any modulus, `mod(_, 128)`
        // under `mod(_, 256)`, …).
        let r = static_range(num);
        if r.lo >= 0.0 && r.hi < m as f64 {
            return match peel(num) {
                // Arithmetic needs `calc()` once it leaves the `mod()`.
                x @ (Node::Sum(_) | Node::Product(_) | Node::Div(..)) => {
                    Node::Calc(Box::new(x.clone()))
                }
                x => x.clone(),
            };
        }
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
        // `mod(round(down, mod(X, A) / B), M)` where B * M divides A →
        // `mod(round(down, X / B), M)`: removing a multiple of A from X
        // shifts X / B by a multiple of A / B, which M divides.
        if let Some(inner) = strip_round_down_mod_div(num, m) {
            return Node::MathFn {
                name: "mod".to_string(),
                args: vec![inner, Node::Int(m)],
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
        if let Some(reduced) = reduce_mod_linear(num, m) {
            return match reduced {
                Node::Int(v) => Node::Int(v.rem_euclid(m)),
                reduced => Node::MathFn {
                    name: "mod".to_string(),
                    args: vec![reduced, Node::Int(m)],
                },
            };
        }
    }
    Node::MathFn { name, args }
}

/// `a / b` of two literals, possibly under `calc()` / `(…)`.
fn as_int_div(node: &Node) -> Option<(i64, i64)> {
    let Node::Div(a, b) = peel(node) else {
        return None;
    };
    Some((as_int(a)?, as_int(b)?))
}

/// Rewrites `X` under `mod(X, m)` as `Σ k·node + c` with every literal
/// respecting the modulus: products distribute over a sum's constant, the
/// constant and the multipliers of integer-valued terms reduce to their
/// smallest residue, like terms merge, and an inner `mod(Y, n) * k` becomes
/// `Y * k` when `m` divides `n·k` (they differ by a multiple of `m`). That
/// last step can enlarge values, so it only stands when the result provably
/// stays exact in a `double`. `None` when nothing changes.
fn reduce_mod_linear(num: &Node, m: i64) -> Option<Node> {
    let rebuild = |drop_inner_mods: bool| {
        let mut lin = Linear::new(m, drop_inner_mods);
        lin.add(num, 1)?;
        lin.distribute_mergeable_groups()?;
        Some((lin.dropped, lin.into_node()))
    };
    let (dropped, node) = rebuild(true)?;
    let node = if dropped && static_range(&node).magnitude() > 2f64.powi(53) {
        rebuild(false)?.1
    } else {
        node
    };
    // `Linear` drops wrappers, so compare against the unwrapped input.
    (node != *peel(num)).then_some(node)
}

struct Linear {
    m: i128,
    drop_inner_mods: bool,
    dropped: bool,
    terms: Vec<(Node, i128)>,
    constant: i128,
}

impl Linear {
    fn new(m: i64, drop_inner_mods: bool) -> Self {
        Linear {
            m: i128::from(m),
            drop_inner_mods,
            dropped: false,
            terms: Vec::new(),
            constant: 0,
        }
    }

    /// Adds `k · node`; `None` on overflow.
    fn add(&mut self, node: &Node, k: i128) -> Option<()> {
        let node = peel(node);
        match node {
            Node::Int(c) => {
                self.constant = (self.constant + k.checked_mul(i128::from(*c))?).rem_euclid(self.m);
                return Some(());
            }
            Node::Sum(terms) => {
                for t in terms {
                    let sign = if t.sign == Sign::Pos { 1 } else { -1 };
                    self.add(&t.node, k * sign)?;
                }
                return Some(());
            }
            Node::MathFn { name, args } if name == "mod" && self.drop_inner_mods => {
                if let [y, n] = args.as_slice()
                    && let Some(n) = as_int(n)
                    && n > 0
                    && (i128::from(n).checked_mul(k)? % self.m) == 0
                {
                    self.dropped = true;
                    return self.add(y, k);
                }
            }
            _ => {}
        }
        if let Some((other, j)) = product_with_int_factor(node) {
            let k = k.checked_mul(i128::from(j))?;
            return match peel_sum(other) {
                // Keep a multi-term sum grouped; only its constant splits off.
                Node::Sum(terms)
                    if terms.iter().filter(|t| as_int(&t.node).is_none()).count() > 1 =>
                {
                    let (group, c) = split_int_offset(Node::Sum(terms));
                    self.add(&Node::Int(c), k)?;
                    self.push(group, k);
                    Some(())
                }
                other => self.add(&other, k),
            };
        }
        self.push(node.clone(), k);
        Some(())
    }

    fn push(&mut self, node: Node, k: i128) {
        match self.terms.iter_mut().find(|(n, _)| *n == node) {
            Some((_, acc)) => *acc += k,
            None => self.terms.push((node, k)),
        }
    }

    /// Distributes a grouped sum `(a + b·j) · k` over its terms when one of
    /// them merges with a term already present: `a + (a + b) · k` has fewer
    /// operations as `a · (k + 1) + b · k`, while an unrelated group stays
    /// one multiplication.
    fn distribute_mergeable_groups(&mut self) -> Option<()> {
        let base = |node: &Node| match product_with_int_factor(node) {
            Some((other, _)) => other,
            None => peel(node).clone(),
        };
        loop {
            let Some(i) = self.terms.iter().position(|(node, _)| {
                let Node::Sum(terms) = peel(node) else {
                    return false;
                };
                terms.iter().any(|t| {
                    let b = base(&t.node);
                    self.terms.iter().any(|(n, _)| *n != *node && *n == b)
                })
            }) else {
                return Some(());
            };
            let (group, k) = self.terms.remove(i);
            self.add(&group, k)?;
        }
    }

    fn into_node(self) -> Node {
        let m = self.m;
        let mut out = Vec::with_capacity(self.terms.len() + 1);
        for (node, k) in self.terms {
            let k = if static_range(&node).int {
                residue(k, m)
            } else {
                k
            };
            if k == 0 {
                continue;
            }
            let sign = if k > 0 { Sign::Pos } else { Sign::Neg };
            let mag = k.unsigned_abs();
            let node = if mag == 1 {
                node
            } else {
                Node::Product(vec![node, Node::Int(mag as i64)])
            };
            out.push(Term { sign, node });
        }
        let c = residue(self.constant, m);
        if c != 0 {
            out.push(Term {
                sign: if c > 0 { Sign::Pos } else { Sign::Neg },
                node: Node::Int(c.unsigned_abs() as i64),
            });
        }
        fold_sum(out)
    }
}

/// Smallest-magnitude residue of `v` modulo `m`.
fn residue(v: i128, m: i128) -> i128 {
    let r = v.rem_euclid(m);
    if 2 * r > m { r - m } else { r }
}

/// Register names: a stage digit, `r`, and the register number (`--_1r3`).
/// Every register holds a byte.
pub(crate) fn is_register_name(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("--_").map(str::as_bytes) else {
        return false;
    };
    rest.len() > 2
        && rest[0].is_ascii_digit()
        && rest[1] == b'r'
        && rest[2..].iter().all(u8::is_ascii_digit)
}

/// Matches `round(down, mod(X, A) / B)` with `B * m` dividing `A` and
/// returns `round(down, X / B)`.
fn strip_round_down_mod_div(node: &Node, m: i64) -> Option<Node> {
    let Node::MathFn { name, args } = node else {
        return None;
    };
    let [Node::Ident(strategy), arg] = args.as_slice() else {
        return None;
    };
    if name != "round" || strategy != "down" {
        return None;
    }
    let Node::Div(num, den) = peel(arg) else {
        return None;
    };
    let b = as_int(den)?;
    let Node::MathFn {
        name: inner_name,
        args: inner_args,
    } = peel(num)
    else {
        return None;
    };
    let [x, a] = inner_args.as_slice() else {
        return None;
    };
    let a = as_int(a)?;
    if inner_name != "mod" || b <= 0 || a <= 0 || a % (b * m) != 0 {
        return None;
    }
    Some(Node::MathFn {
        name: "round".to_string(),
        args: vec![
            Node::Ident("down".to_string()),
            Node::Div(Box::new(x.clone()), Box::new(Node::Int(b))),
        ],
    })
}

fn peel(node: &Node) -> &Node {
    match node {
        Node::Calc(inner) | Node::Paren(inner) => peel(inner),
        other => other,
    }
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
        ("--inrange", [x, lo, hi]) => fold_inrange(x.clone(), lo.clone(), hi.clone()),
        ("--mlo", [w]) if let Some(w) = as_int(w) => Node::Int(w.rem_euclid(256)),
        ("--mhi", [w]) if let Some(w) = as_int(w) => Node::Int(w.div_euclid(256).rem_euclid(256)),
        ("--mhalf", [x]) if let Some(x) = as_int(x) => Node::Int(x.div_euclid(2)),
        ("--mpar", [x]) if let Some(x) = as_int(x) => Node::Int(x.rem_euclid(2)),
        ("--hex", [x]) if let Some(x) = as_int(x) => Node::Str(hex_digit(x)),
        ("--i2char", [x]) if let Some(x) = as_int(x) => Node::Str(char_literal(x)),
        ("--cond_char", [c, v]) if let Some(c) = as_int(c) => {
            if c == 0 {
                Node::Str("\"\"".to_string())
            } else {
                fold_fn("--i2char", vec![v.clone()])
            }
        }
        _ => Node::Fn {
            name: name.to_string(),
            args,
        },
    }
}

/// `--hex(x)`: the lowercase hex digit of `x`, `"0"` outside 0..=15.
fn hex_digit(x: i64) -> String {
    let d = u32::try_from(x).ok().and_then(|x| char::from_digit(x, 16));
    format!("\"{}\"", d.unwrap_or('0'))
}

/// `--i2char(x)`: the string for character code `x` (see the
/// `@function --i2char` lookup table the emitter writes).
fn char_literal(x: i64) -> String {
    let body = match x {
        -1 | 8 => String::new(),
        9 => "    ".to_string(),
        10 => "\\a ".to_string(),
        34 => "\\\"".to_string(),
        92 => "\\\\".to_string(),
        32..=126 => char::from(x as u8).to_string(),
        _ => "?".to_string(),
    };
    format!("\"{body}\"")
}

fn fold_lt_ge(a: Node, b: Node, is_lt: bool) -> Node {
    let name = if is_lt { "--lt" } else { "--ge" };

    // Disjoint ranges decide the comparison: literals against literals, a
    // register or the stack pointer against a threshold outside its range.
    let (ra, rb) = (static_range(&a), static_range(&b));
    if ra.hi < rb.lo {
        return Node::Int(i64::from(is_lt));
    }
    if ra.lo >= rb.hi {
        return Node::Int(i64::from(!is_lt));
    }

    Node::Fn {
        name: name.to_string(),
        args: vec![a, b],
    }
}

/// `--inrange(x, lo, hi)` is `lo <= x < hi`. Literal terms of `x` move into
/// the bounds, so they fold to constants.
fn fold_inrange(x: Node, lo: Node, hi: Node) -> Node {
    let (x, off) = split_int_offset(x);
    let (Some(lo), Some(hi)) = (as_int(&lo), as_int(&hi)) else {
        return Node::Fn {
            name: "--inrange".to_string(),
            args: vec![x, lo, hi],
        };
    };
    let (lo, hi) = (lo - off, hi - off);
    if hi <= lo {
        return Node::Int(0);
    }
    // A bound the value cannot cross leaves a single comparison (or a
    // constant, once both are known).
    let r = static_range(&x);
    if r.lo >= lo as f64 {
        return fold_lt_ge(x, Node::Int(hi), /*is_lt=*/ true);
    }
    if r.hi < hi as f64 {
        return fold_lt_ge(x, Node::Int(lo), /*is_lt=*/ false);
    }
    Node::Fn {
        name: "--inrange".to_string(),
        args: vec![x, Node::Int(lo), Node::Int(hi)],
    }
}

/// Splits `X + c` (any literal terms of a sum) into `(X, c)`.
fn split_int_offset(node: Node) -> (Node, i64) {
    let Node::Sum(terms) = peel_sum(node.clone()) else {
        return (node, 0);
    };
    let mut off = 0;
    let mut rest = Vec::with_capacity(terms.len());
    for term in terms {
        match term.node {
            Node::Int(n) if term.sign == Sign::Pos => off += n,
            Node::Int(n) => off -= n,
            _ => rest.push(term),
        }
    }
    if off == 0 {
        return (node, 0);
    }
    let base = match rest.len() {
        0 => Node::Int(0),
        1 if rest[0].sign == Sign::Pos => rest.pop().unwrap().node,
        _ => Node::Calc(Box::new(Node::Sum(rest))),
    };
    (base, off)
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
        return fold_sel(unwrapped, f, t);
    }
    // `--sel` only tests `c == 0`, so a wrapper that keeps zero-ness can go.
    if let Some(unwrapped) = peel_zero_preserving(&c) {
        return fold_sel(unwrapped, t, f);
    }
    Node::Fn {
        name: "--sel".to_string(),
        args: vec![c, t, f],
    }
}

/// Returns `Some(X)` when `node` is zero exactly when `X` is: `abs(X)`,
/// `sign(X)`, or `min(K, X)` / `min(X, K)` with a positive literal `K`.
/// A surrounding `--sel` (which only compares against zero) sees the same
/// answer either way.
fn peel_zero_preserving(node: &Node) -> Option<Node> {
    let Node::MathFn { name, args } = node else {
        return None;
    };
    match (name.as_str(), args.as_slice()) {
        ("abs" | "sign", [x]) => Some(x.clone()),
        ("min", [a, b]) => match (as_int(a), as_int(b)) {
            (Some(k), None) if k > 0 => Some(b.clone()),
            (None, Some(k)) if k > 0 => Some(a.clone()),
            _ => None,
        },
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
    let r = static_range(node);
    r.int && r.within(0.0, 1.0)
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
        Node::Int(_)
        | Node::Var { .. }
        | Node::Fn { .. }
        | Node::If { .. }
        | Node::Raw(_)
        | Node::Ident(_) => inner,
        Node::Calc(deeper) => Node::Calc(deeper),
        // `calc((X))` collapses to `calc(X)`.
        Node::Paren(inner) => Node::Calc(inner),
        other => Node::Calc(Box::new(other)),
    }
}

fn fold_paren(inner: Node) -> Node {
    match inner {
        // `(X)` around an atomic value is redundant.
        Node::Int(_)
        | Node::Var { .. }
        | Node::Fn { .. }
        | Node::MathFn { .. }
        | Node::Raw(_)
        | Node::Ident(_) => inner,
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
        match peel_sum(term.node) {
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
    if let Some(negated) = negate_bool_sum(&flat) {
        return negated;
    }
    match flat.len() {
        0 => Node::Int(0),
        1 if flat[0].sign == Sign::Pos => flat.into_iter().next().unwrap().node,
        _ => Node::Sum(flat),
    }
}

/// `1 - --ge(a, b)` is `--lt(a, b)`, and likewise for the other comparison
/// helpers with a complement.
fn negate_bool_sum(flat: &[Term]) -> Option<Node> {
    let [one, b] = flat else {
        return None;
    };
    if one.sign != Sign::Pos || one.node != Node::Int(1) || b.sign != Sign::Neg {
        return None;
    }
    let Node::Fn { name, args } = &b.node else {
        return None;
    };
    let complement = match name.as_str() {
        "--lt" => "--ge",
        "--ge" => "--lt",
        "--eq" => "--ne",
        "--ne" => "--eq",
        "--eqz" => "--nez",
        "--nez" => "--eqz",
        _ => return None,
    };
    Some(fold_fn(complement, args.clone()))
}

/// A sum under `(…)` or `calc(…)` is still a sum: a term's sign
/// distributes over it, so its terms (and constants) can join the outer sum.
/// A product there needs no wrapper either: it binds tighter than `+`/`-`.
fn peel_sum(node: Node) -> Node {
    match node {
        Node::Paren(inner) | Node::Calc(inner)
            if matches!(*inner, Node::Sum(_) | Node::Product(_)) =>
        {
            *inner
        }
        other => other,
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
    // Try to merge an integer literal into an earlier literal term,
    // accumulating constants without reshuffling order.
    if let Node::Int(n) = &node
        && let Some((pos, prev)) = flat
            .iter()
            .enumerate()
            .rev()
            .find_map(|(i, t)| match t.node {
                Node::Int(v) => Some((i, if t.sign == Sign::Pos { v } else { -v })),
                _ => None,
            })
    {
        let total = prev + if sign == Sign::Pos { *n } else { -*n };
        if total == 0 {
            flat.remove(pos);
        } else {
            let lit = &mut flat[pos];
            lit.sign = if total > 0 { Sign::Pos } else { Sign::Neg };
            lit.node = Node::Int(total.abs());
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
        if !push_factor(&mut flat, &mut const_acc, &mut saw_zero, f) {
            return Node::Int(0);
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
    // A `calc()` or `(…)` factor needs no wrapper: a sum prints in
    // parentheses, and a product joins this one.
    match peel_sum(node) {
        Node::Int(0) => {
            *saw_zero = true;
            false
        }
        Node::Int(1) => true,
        Node::Int(n) => {
            *const_acc *= n;
            true
        }
        Node::Product(factors) => factors
            .into_iter()
            .all(|f| push_factor(flat, const_acc, saw_zero, f)),
        other => {
            flat.push(other);
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
