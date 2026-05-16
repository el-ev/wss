//! Inlines trivial slot-indicator property declarations from the logic CSS
//! into the support CSS, then folds the resulting compare-function calls
//! (`--eq1`, `--eqz`, `--nez`, `--eq`, `--ne`) that take integer-literal
//! arguments.
//!
//! Slot-indicator families: `--cri`, `--crp`, `--csi`, `--csp`, `--mri`,
//! `--msc`, `--msp` (each with an optional `b` suffix on store-paired
//! variants). A declaration is considered trivial when its value is either
//! a bare integer literal (`--crp28: 0;`) or a single-arm `if(...)` whose
//! arm and else values are both integer literals
//! (`--crp31: if(style(--_1pc: 5156): 1; else: 0);`).

const SLOT_PREFIXES: &[&str] = &["cri", "crp", "csi", "csp", "mri", "msc", "msp"];

pub(super) fn inline_slot_indicators(logic: &mut String, support: &mut String) {
    let mut subs: Vec<(String, String)> = Vec::new();
    let mut kept = String::with_capacity(logic.len());

    for line in logic.split_inclusive('\n') {
        if let Some((name, value)) = parse_trivial_slot_decl(line) {
            subs.push((name, value));
        } else {
            kept.push_str(line);
        }
    }

    if subs.is_empty() {
        return;
    }
    *logic = kept;

    let mut has_int = false;
    for (name, value) in &subs {
        let pattern = format!("var({})", name);
        *support = support.replace(&pattern, value);
        *logic = logic.replace(&pattern, value);
        if is_int_literal(value) {
            has_int = true;
        }
    }

    if has_int {
        *support = fold_compare_funcs(support);
        *logic = fold_compare_funcs(logic);
    }
    // Run the compare-fold and arithmetic-fold passes to fixpoint: an inner
    // fold (e.g. `--eq(0, 2) → 0`) can expose a surrounding fold opportunity
    // (e.g. `--sel(0, X, Y) → Y`) that a single left-to-right scan misses.
    loop {
        let support_folded = fold_compare_funcs(support);
        let support_simpl = simplify_calc(&support_folded);
        let logic_folded = fold_compare_funcs(logic);
        let logic_simpl = simplify_calc(&logic_folded);
        if support_simpl == *support && logic_simpl == *logic {
            break;
        }
        *support = support_simpl;
        *logic = logic_simpl;
    }
}

/// Iteratively applies a small set of literal-driven peephole rewrites
/// to flatten the residue left in function bodies after slot-indicator
/// inlining: `(1 - (0))` → `1`, `0 * X` terms drop out of sums, `1 * X`
/// loses the multiplier, and trivial parens around literals collapse.
fn simplify_calc(input: &str) -> String {
    let mut s = input.to_string();
    loop {
        let mut changed = false;
        for (from, to) in &[
            ("(1 - (0))", "1"),
            ("(1 - (1))", "0"),
            ("(1 - 0)", "1"),
            ("(1 - 1)", "0"),
            ("calc(1 - 0)", "1"),
            ("calc(1 - 1)", "0"),
            ("(0)", "0"),
            ("(1)", "1"),
        ] {
            let replaced = s.replace(from, to);
            if replaced != s {
                s = replaced;
                changed = true;
            }
        }
        if let Some(next) = drop_zero_terms_anywhere(&s) {
            s = next;
            changed = true;
        }
        if let Some(next) = unwrap_trivial_calc(&s) {
            s = next;
            changed = true;
        }
        if let Some(next) = unnest_calc(&s) {
            s = next;
            changed = true;
        }
        if let Some(next) = collapse_double_parens(&s) {
            s = next;
            changed = true;
        }
        if let Some(next) = drop_one_factor(&s) {
            s = next;
            changed = true;
        }
        if let Some(next) = strip_redundant_calc_wrappers(&s) {
            s = next;
            changed = true;
        }
        if !changed {
            break;
        }
    }
    s
}

/// Collapses `((X))` to `(X)` when both parens close at the same depth
/// (the inner pair covers the entire outer body). Safe in any CSS context:
/// adjacent matching parens around the same expression are redundant.
///
/// Earlier versions of this pass also tried `(calc(X)) → (X)`, but that's
/// unsafe — the surrounding `(` may be the argument list of a non-math
/// CSS function (e.g. `--read_cs(calc(...))`) where the argument needs an
/// explicit `calc(...)` because function args don't get implicit math
/// context. Stripping the `calc` there breaks evaluation.
fn strip_redundant_calc_wrappers(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut changed = false;
    while i < bytes.len() {
        if bytes[i] == b'('
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'('
            && let Some(inner_end) = find_matching_close(bytes, i + 2)
            && let Some(outer_end) = find_matching_close(bytes, i + 1)
            && inner_end + 1 == outer_end
        {
            out.push('(');
            out.push_str(&input[i + 2..inner_end]);
            out.push(')');
            i = outer_end + 1;
            changed = true;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    if changed { Some(out) } else { None }
}

/// Collapses `((X))` into `(X)` — adjacent parens that don't separate
/// distinct operations. Iterates one pass and returns `None` when nothing
/// changed.
fn collapse_double_parens(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let mut changed = false;
    while i < bytes.len() {
        if bytes[i] == b'('
            && i + 1 < bytes.len()
            && bytes[i + 1] == b'('
            && let Some(inner_close) = find_matching_close(bytes, i + 2)
            && inner_close + 1 < bytes.len()
            && bytes[inner_close + 1] == b')'
            && find_matching_close(bytes, i + 1) == Some(inner_close + 1)
        {
            // `((X))` matches: drop one layer.
            out.push('(');
            out.push_str(&input[i + 2..inner_close]);
            out.push(')');
            i = inner_close + 2;
            changed = true;
            continue;
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    if changed { Some(out) } else { None }
}

/// Strips nested `calc(...)` wrappers: inside a math-context body, a
/// `calc(X)` is semantically identical to `(X)` per CSS spec, so we drop
/// the redundant keyword. Returns `None` when nothing changed.
///
/// Math context propagates only through math functions (`calc`, `min`,
/// `max`, `clamp`, `mod`, `rem`, `round`, `abs`, `sign`, plus trig/log)
/// and plain parens that inherit from their enclosing scope. Non-math
/// CSS functions (`--read_cs`, `--inrange`, `--mload`, `--sel`, …) take
/// arguments that DON'T receive implicit math context, so stripping the
/// `calc(` of an inner argument like `--read_cs(calc(var + 1))` is
/// unsafe — we'd produce `--read_cs((var + 1))`, which is invalid.
fn unnest_calc(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    // Stack of math-context flags, one entry per open paren currently on
    // the stack. `true` means the body of that paren is evaluated in math
    // context (calc/min/max/etc.); `false` means it's a non-math function
    // arglist where the arg starts a fresh non-math context.
    let mut stack: Vec<bool> = Vec::new();
    let mut changed = false;
    while i < bytes.len() {
        if let Some((name_len, is_math)) = peek_func_open(input, i) {
            let in_math = stack.last().copied().unwrap_or(false);
            // `calc(` inside math context becomes a plain `(`.
            if name_len == 4 && &input[i..i + 4] == "calc" && in_math {
                out.push('(');
                i += 5;
                stack.push(true);
                changed = true;
                continue;
            }
            out.push_str(&input[i..i + name_len + 1]);
            i += name_len + 1;
            stack.push(is_math);
            continue;
        }
        let b = bytes[i];
        match b {
            b'(' => {
                // Plain paren inherits math context from its enclosing scope.
                let in_math = stack.last().copied().unwrap_or(false);
                stack.push(in_math);
                out.push('(');
            }
            b')' => {
                stack.pop();
                out.push(')');
            }
            _ => out.push(b as char),
        }
        i += 1;
    }
    if changed { Some(out) } else { None }
}

/// If `input[i..]` looks like `name(`, returns `(name_len, name_is_math)`.
/// Names are identifier-ish (letters, digits, `-`, `_`) and must end
/// immediately before a `(`.
fn peek_func_open(input: &str, i: usize) -> Option<(usize, bool)> {
    let bytes = input.as_bytes();
    if i >= bytes.len() {
        return None;
    }
    let start = i;
    let mut j = i;
    while j < bytes.len() {
        let b = bytes[j];
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
            j += 1;
        } else {
            break;
        }
    }
    if j == start || j >= bytes.len() || bytes[j] != b'(' {
        return None;
    }
    // Name must begin with a letter or `-` to count as a function call.
    let first = bytes[start];
    if !(first.is_ascii_alphabetic() || first == b'-') {
        return None;
    }
    let name = &input[start..j];
    Some((j - start, is_math_function(name)))
}

/// CSS math functions whose argument bodies inherit math context, per
/// css-values-4. Any `calc(...)` strictly inside one of these can be
/// flattened to bare parens.
fn is_math_function(name: &str) -> bool {
    matches!(
        name,
        "calc"
            | "min"
            | "max"
            | "clamp"
            | "mod"
            | "rem"
            | "round"
            | "abs"
            | "sign"
            | "hypot"
            | "sqrt"
            | "pow"
            | "log"
            | "exp"
            | "sin"
            | "cos"
            | "tan"
            | "asin"
            | "acos"
            | "atan"
            | "atan2"
    )
}

/// Unwraps `calc(<single_term>)` where the inner term is a bare `var(…)` or
/// a literal — the surrounding `calc()` adds no value.
fn unwrap_trivial_calc(input: &str) -> Option<String> {
    let bytes = input.as_bytes();
    let mut out = String::with_capacity(input.len());
    let mut i = 0;
    let mut changed = false;
    while i < bytes.len() {
        if let Some(rest) = input.get(i..)
            && let Some(after) = rest.strip_prefix("calc(")
            && let Some(end) = find_matching_close(after.as_bytes(), 0)
        {
            let body = after[..end].trim();
            if can_omit_calc(body) {
                out.push_str(body);
                i += 5 + end + 1;
                changed = true;
                continue;
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    if changed { Some(out) } else { None }
}

/// Whether `body` is safe to substitute in place of `calc(<body>)`. We allow
/// bare integer literals and a single balanced `var(...)` token.
fn can_omit_calc(body: &str) -> bool {
    if body.is_empty() {
        return false;
    }
    if parse_int(body).is_some() {
        return true;
    }
    if let Some(rest) = body.strip_prefix("var(")
        && let Some(close) = find_matching_close(rest.as_bytes(), 0)
        && close + 1 == rest.len()
    {
        return true;
    }
    false
}

/// Drops `0 * <atom>` terms from every `+`-separated sum that appears as the
/// body of a `calc(...)` call OR as the body of a top-level `(...)` group
/// containing additions. Iterates the whole string in one pass and returns
/// `None` when no rewrite was found.
fn drop_zero_terms_anywhere(input: &str) -> Option<String> {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    let mut changed = false;
    while i < bytes.len() {
        if let Some(rest) = input.get(i..) {
            if let Some(stripped) = rest.strip_prefix("calc(")
                && let Some(body_end) = find_matching_close(stripped.as_bytes(), 0)
            {
                let body = &stripped[..body_end];
                if let Some(new_body) = remove_zero_terms_from_sum(body) {
                    out.push_str("calc(");
                    out.push_str(&new_body);
                    out.push(')');
                    i += 5 + body_end + 1;
                    changed = true;
                    continue;
                }
            } else if bytes[i] == b'('
                && let Some(body_end) = find_matching_close(bytes, i + 1)
            {
                let body = &input[i + 1..body_end];
                if body.contains(" + ")
                    && let Some(new_body) = remove_zero_terms_from_sum(body)
                {
                    out.push('(');
                    out.push_str(&new_body);
                    out.push(')');
                    i = body_end + 1;
                    changed = true;
                    continue;
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    if changed { Some(out) } else { None }
}

/// Drops `1 * <atom>` and `<atom> * 1` from sums and products inside the
/// final emitted string. Returns `None` when no such factor is found.
fn drop_one_factor(input: &str) -> Option<String> {
    let needles_prefix: &[(&str, &str)] = &[
        (" + 1 * ", " + "),
        (" * 1 +", " +"),
        (" * 1)", ")"),
        ("(1 * ", "("),
    ];
    let mut s = input.to_string();
    let mut changed = false;
    for (from, to) in needles_prefix {
        let replaced = s.replace(from, to);
        if replaced != s {
            s = replaced;
            changed = true;
        }
    }
    if changed { Some(s) } else { None }
}

/// Splits a calc body on top-level `+` and removes any term that is exactly
/// `0` or has the form `0 * <atom>`. Returns `None` when nothing matched.
fn remove_zero_terms_from_sum(body: &str) -> Option<String> {
    let parts = split_top_level(body, b'+');
    if parts.len() < 2 {
        return None;
    }
    let mut kept: Vec<String> = Vec::with_capacity(parts.len());
    let mut dropped = false;
    for part in parts {
        let trimmed = part.trim();
        if trimmed == "0" || trimmed.starts_with("0 * ") {
            dropped = true;
            continue;
        }
        kept.push(trimmed.to_string());
    }
    if !dropped {
        return None;
    }
    if kept.is_empty() {
        return Some("0".to_string());
    }
    Some(kept.join(" + "))
}

/// Returns the index of the closing `)` that matches an opening `(` at
/// position `open` in `bytes`. `bytes[open]` is assumed to be the byte
/// immediately AFTER the opening paren of the calc call.
fn find_matching_close(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth: i32 = 1;
    let mut i = open;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn parse_trivial_slot_decl(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("--")?;
    let colon = rest.find(':')?;
    let name = &rest[..colon];
    if !is_slot_indicator_name(name) {
        return None;
    }
    let value = rest[colon + 1..].trim().strip_suffix(';')?.trim();
    if !is_trivial_value(value) {
        return None;
    }
    Some((format!("--{}", name), value.to_string()))
}

fn is_slot_indicator_name(name: &str) -> bool {
    for &prefix in SLOT_PREFIXES {
        if let Some(rest) = name.strip_prefix(prefix) {
            let digits = rest.strip_suffix('b').unwrap_or(rest);
            if !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()) {
                return true;
            }
        }
    }
    false
}

fn is_trivial_value(value: &str) -> bool {
    is_int_literal(value) || is_single_arm_int_if(value)
}

fn is_int_literal(s: &str) -> bool {
    let t = s.trim();
    let body = t.strip_prefix('-').unwrap_or(t);
    !body.is_empty() && body.bytes().all(|b| b.is_ascii_digit())
}

fn is_single_arm_int_if(value: &str) -> bool {
    let Some(body) = value.strip_prefix("if(").and_then(|s| s.strip_suffix(')')) else {
        return false;
    };
    let mut depth = 0i32;
    let mut semi = None;
    for (i, b) in body.bytes().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b';' if depth == 0 => {
                if semi.is_some() {
                    return false;
                }
                semi = Some(i);
            }
            _ => {}
        }
    }
    let Some(s) = semi else { return false };
    let arm_part = body[..s].trim();
    let else_part = body[s + 1..].trim();
    let Some(idx) = arm_part.rfind("): ") else {
        return false;
    };
    let arm_val = arm_part[idx + 3..].trim();
    let Some(else_val) = else_part.strip_prefix("else:").map(str::trim) else {
        return false;
    };
    is_int_literal(arm_val) && is_int_literal(else_val)
}

fn fold_compare_funcs(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some((consumed, replacement)) = try_fold_at(input, i) {
            out.push_str(&replacement);
            i += consumed;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    out
}

fn try_fold_at(input: &str, i: usize) -> Option<(usize, String)> {
    let rest = &input[i..];
    for (name, kind) in [
        ("--eq1(", FoldKind::Eq1),
        ("--eqz(", FoldKind::Eqz),
        ("--nez(", FoldKind::Nez),
        ("--eq(", FoldKind::Eq),
        ("--ne(", FoldKind::Ne),
        ("--sel(", FoldKind::Sel),
    ] {
        if rest.starts_with(name) {
            let open = name.len() - 1;
            let Some(close) = find_matching_paren(rest.as_bytes(), open) else {
                continue;
            };
            let body = &rest[open + 1..close];
            if let Some(folded) = try_fold_fn(kind, body) {
                return Some((close + 1, folded));
            }
        }
    }
    None
}

#[derive(Clone, Copy)]
enum FoldKind {
    Eq1,
    Eqz,
    Nez,
    Eq,
    Ne,
    Sel,
}

fn try_fold_fn(kind: FoldKind, body: &str) -> Option<String> {
    let body = body.trim();
    match kind {
        FoldKind::Eq1 => {
            if let Some(n) = parse_int(body) {
                return Some(if n == 1 { "1" } else { "0" }.to_string());
            }
            // `--eq1(<bool>)` ≡ `<bool>`.
            if is_bool_if(body) {
                return Some(body.to_string());
            }
            distribute_compare_into_if(body, |v: i64| (v == 1) as i64)
        }
        FoldKind::Eqz => {
            if let Some(n) = parse_int(body) {
                return Some(if n == 0 { "1" } else { "0" }.to_string());
            }
            // `--eqz(<bool>)` ≡ `calc(1 - <bool>)`.
            if is_bool_if(body) {
                return Some(format!("calc(1 - {})", body));
            }
            distribute_compare_into_if(body, |v: i64| (v == 0) as i64)
        }
        FoldKind::Nez => {
            if let Some(n) = parse_int(body) {
                return Some(if n == 0 { "0" } else { "1" }.to_string());
            }
            // `--nez(<bool>)` ≡ `<bool>`.
            if is_bool_if(body) {
                return Some(body.to_string());
            }
            distribute_compare_into_if(body, |v: i64| (v != 0) as i64)
        }
        FoldKind::Sel => {
            // --sel(cond, t, f): when cond is a literal, pick the surviving branch.
            let parts = split_top_level(body, b',');
            if parts.len() != 3 {
                return None;
            }
            let cond = parts[0].trim();
            let t = parts[1].trim();
            let f = parts[2].trim();
            let n = parse_int(cond)?;
            Some(if n != 0 { t.to_string() } else { f.to_string() })
        }
        FoldKind::Eq | FoldKind::Ne => {
            let parts = split_top_level(body, b',');
            if parts.len() != 2 {
                return None;
            }
            let a_str = parts[0].trim();
            let b_str = parts[1].trim();
            // Constant-constant fold.
            if let (Some(a), Some(b)) = (parse_int(a_str), parse_int(b_str)) {
                let eq = a == b;
                let val = match kind {
                    FoldKind::Eq => {
                        if eq {
                            "1"
                        } else {
                            "0"
                        }
                    }
                    FoldKind::Ne => {
                        if eq {
                            "0"
                        } else {
                            "1"
                        }
                    }
                    _ => unreachable!(),
                };
                return Some(val.to_string());
            }
            // Bool-vs-literal: --eq(b, 1) ≡ b, --eq(b, 0) ≡ calc(1 - b); --ne mirrors.
            if let Some((bool_side, lit)) = match (parse_int(a_str), parse_int(b_str)) {
                (Some(n), None) if is_bool_if(b_str) => Some((b_str, n)),
                (None, Some(n)) if is_bool_if(a_str) => Some((a_str, n)),
                _ => None,
            } {
                return match (kind, lit) {
                    (FoldKind::Eq, 1) | (FoldKind::Ne, 0) => Some(bool_side.to_string()),
                    (FoldKind::Eq, 0) | (FoldKind::Ne, 1) => {
                        Some(format!("calc(1 - {})", bool_side))
                    }
                    // A boolean is always in {0, 1}, so comparing it against any
                    // other literal is a constant: 0 for `--eq`, 1 for `--ne`.
                    (FoldKind::Eq, _) => Some("0".to_string()),
                    (FoldKind::Ne, _) => Some("1".to_string()),
                    _ => None,
                };
            }
            // Push `--eq(if(<arms>; else: V), K)` / `--ne(...)` into the if(),
            // mapping each arm value v to `1 if v == K else 0` (or its
            // negation for --ne). When the arg order is `(K, if(...))` we
            // handle it symmetrically.
            let (if_expr, k) = match (parse_int(a_str), parse_int(b_str)) {
                (Some(k), None) => (b_str, k),
                (None, Some(k)) => (a_str, k),
                _ => return None,
            };
            let map = match kind {
                FoldKind::Eq => |v: i64, k: i64| -> i64 { (v == k) as i64 },
                FoldKind::Ne => |v: i64, k: i64| -> i64 { (v != k) as i64 },
                _ => unreachable!(),
            };
            distribute_compare_into_if(if_expr, |v| map(v, k))
        }
    }
}

/// When `body` is `if(<arms>; else: V)` and every arm value is an integer
/// literal, rebuilds the if() with each arm value replaced by `f(arm_value)`.
/// Used to push pointwise compare functions (`--eq1`, `--eqz`, `--nez`,
/// `--eq(if, K)`, `--ne(if, K)`) inside the if so the surrounding compare
/// disappears and downstream folds see a plain `if(...): 0|1; else: 0|1)`.
fn distribute_compare_into_if(body: &str, mut f: impl FnMut(i64) -> i64) -> Option<String> {
    let trimmed = body.trim();
    let rest = trimmed.strip_prefix("if(")?;
    let inner = rest.strip_suffix(')')?;
    let arms = split_top_level(inner, b';');
    if arms.is_empty() {
        return None;
    }
    let mut new_arms: Vec<String> = Vec::with_capacity(arms.len());
    for arm in arms {
        let arm = arm.trim();
        if arm.is_empty() {
            continue;
        }
        if let Some(v) = arm.strip_prefix("else:") {
            let n = parse_int(v.trim())?;
            new_arms.push(format!("else: {}", f(n)));
            continue;
        }
        let idx = arm
            .rfind("): ")
            .map(|p| p + 3)
            .or_else(|| arm.rfind(": ").map(|p| p + 2))?;
        let (cond, value) = arm.split_at(idx);
        let n = parse_int(value.trim())?;
        new_arms.push(format!("{}{}", cond, f(n)));
    }
    Some(format!("if({})", new_arms.join("; ")))
}

/// Recognises `if(<arms>; else: V)` where every arm body and the `else` value
/// is either `0` or `1` — i.e. the expression always evaluates to a boolean.
fn is_bool_if(s: &str) -> bool {
    let trimmed = s.trim();
    let Some(rest) = trimmed.strip_prefix("if(") else {
        return false;
    };
    let Some(inner) = rest.strip_suffix(')') else {
        return false;
    };
    let arms = split_top_level(inner, b';');
    if arms.is_empty() {
        return false;
    }
    for arm in arms {
        let arm = arm.trim();
        if arm.is_empty() {
            continue;
        }
        let value = if let Some(v) = arm.strip_prefix("else:") {
            v.trim()
        } else if let Some(idx) = arm.rfind("): ") {
            arm[idx + 3..].trim()
        } else if let Some(idx) = arm.rfind(": ") {
            arm[idx + 2..].trim()
        } else {
            return false;
        };
        if value != "0" && value != "1" {
            return false;
        }
    }
    true
}

fn parse_int(s: &str) -> Option<i64> {
    s.trim().parse::<i64>().ok()
}

fn find_matching_paren(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0i32;
    for (i, &b) in bytes.iter().enumerate().skip(open) {
        match b {
            b'(' => depth += 1,
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn split_top_level(s: &str, sep: u8) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, &b) in s.as_bytes().iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            x if x == sep && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&s[start..]);
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_slot_indicator_names() {
        assert!(is_slot_indicator_name("crp28"));
        assert!(is_slot_indicator_name("csi0"));
        assert!(is_slot_indicator_name("msc12b"));
        assert!(is_slot_indicator_name("msp0b"));
        assert!(!is_slot_indicator_name("mso0"));
        assert!(!is_slot_indicator_name("pcg5"));
        assert!(!is_slot_indicator_name("crp"));
        assert!(!is_slot_indicator_name("crpb"));
    }

    #[test]
    fn parses_integer_decl() {
        let (n, v) = parse_trivial_slot_decl(" --crp28: 0;\n").unwrap();
        assert_eq!(n, "--crp28");
        assert_eq!(v, "0");
    }

    #[test]
    fn parses_single_arm_int_if() {
        let line = " --crp31: if(style(--_1pc: 5156): 1; else: 0);\n";
        let (n, v) = parse_trivial_slot_decl(line).unwrap();
        assert_eq!(n, "--crp31");
        assert_eq!(v, "if(style(--_1pc: 5156): 1; else: 0)");
    }

    #[test]
    fn rejects_multi_arm_if() {
        let line = " --csi0: if(style(--pcg3: 1): var(--_1cs_sp); style(--_1pc: 5157): calc(var(--_1cs_sp) + 16); else: 0);\n";
        assert!(parse_trivial_slot_decl(line).is_none());
    }

    #[test]
    fn rejects_arm_with_expression() {
        let line = " --csi2: if(style(--pcg11: 1): calc(var(--_1cs_sp) + 1); else: 0);\n";
        assert!(parse_trivial_slot_decl(line).is_none());
    }

    #[test]
    fn accepts_or_chain_arm() {
        let line = " --crp0: if(style(--_1pc: 3030) or style(--_1pc: 4003): 2; else: 0);\n";
        let (n, v) = parse_trivial_slot_decl(line).unwrap();
        assert_eq!(n, "--crp0");
        assert!(v.starts_with("if(") && v.ends_with(")"));
    }

    #[test]
    fn ignores_non_slot_indicator() {
        assert!(parse_trivial_slot_decl(" --mso0: 0;\n").is_none());
        assert!(parse_trivial_slot_decl(" --pcg5: 0;\n").is_none());
    }

    #[test]
    fn folds_eq1_eqz_nez() {
        assert_eq!(fold_compare_funcs("--eq1(0)"), "0");
        assert_eq!(fold_compare_funcs("--eq1(1)"), "1");
        assert_eq!(fold_compare_funcs("--eq1(7)"), "0");
        assert_eq!(fold_compare_funcs("--eqz(0)"), "1");
        assert_eq!(fold_compare_funcs("--eqz(3)"), "0");
        assert_eq!(fold_compare_funcs("--nez(0)"), "0");
        assert_eq!(fold_compare_funcs("--nez(5)"), "1");
    }

    #[test]
    fn folds_eq_ne_pair() {
        assert_eq!(fold_compare_funcs("--eq(0, 0)"), "1");
        assert_eq!(fold_compare_funcs("--eq(2, 5)"), "0");
        assert_eq!(fold_compare_funcs("--ne(0, 0)"), "0");
        assert_eq!(fold_compare_funcs("--ne(2, 5)"), "1");
    }

    #[test]
    fn folds_inside_calc() {
        let s = "calc((var(--cri0) * 2) + --eq1(0))";
        assert_eq!(fold_compare_funcs(s), "calc((var(--cri0) * 2) + 0)");
    }

    #[test]
    fn leaves_non_literal_args_alone() {
        let s = "--eq1(var(--crp0))";
        assert_eq!(fold_compare_funcs(s), s);
        let s = "--eq(var(--csi0), 2)";
        assert_eq!(fold_compare_funcs(s), s);
    }

    #[test]
    fn end_to_end_inlining() {
        let mut logic =
            String::from(" --crp28: 0;\n --cri0: if(style(--pcg3: 1): var(--_1cs_sp); else: 0);\n");
        let mut support = String::from("calc((var(--cri0) * 2) + --eq1(var(--crp28)))\n");
        inline_slot_indicators(&mut logic, &mut support);
        assert!(
            !logic.contains("--crp28:"),
            "trivial decl should be removed"
        );
        assert!(logic.contains("--cri0:"), "non-trivial decl preserved");
        // simplify_calc drops the trailing `+ 0` and the redundant inner parens.
        assert_eq!(support.trim_end(), "calc(var(--cri0) * 2)");
    }

    #[test]
    fn end_to_end_inlining_single_arm_if() {
        let mut logic = String::from(" --crp31: if(style(--_1pc: 5156): 1; else: 0);\n");
        let mut support = String::from("--eq1(var(--crp31))\n");
        inline_slot_indicators(&mut logic, &mut support);
        assert!(!logic.contains("--crp31:"));
        // --eq1(<bool_if>) folds to <bool_if> after the fixpoint passes.
        assert_eq!(support.trim_end(), "if(style(--_1pc: 5156): 1; else: 0)");
    }
}
