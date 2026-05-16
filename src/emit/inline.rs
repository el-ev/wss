use std::collections::HashMap;

use crate::css::fold_document_values;

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

    for (name, value) in &subs {
        let pattern = format!("var({})", name);
        *support = support.replace(&pattern, value);
        *logic = logic.replace(&pattern, value);
    }
}

/// Run the CSS AST fold over every property value in both buffers.
pub(super) fn fold_value_expressions(logic: &mut String, support: &mut String) {
    *support = fold_document_values(support);
    *logic = fold_document_values(logic);
}

/// Remove `--name: <value>;` declarations that have no reader anywhere
/// in the cascaded output, and inline single-use decls. Iterates to
/// fixpoint because each pass can expose work for the other.
pub(super) fn eliminate_dead_decls(logic: &mut String, support: &mut String, base_html: &str) {
    let mut logic_doc = crate::css::parse_doc(logic);
    let mut support_doc = crate::css::parse_doc(support);

    // Seed with refs from base.html (not parsed into the AST) and the
    // JS-read property denylist so DCE never drops their backing decls.
    let external = build_external_refs(base_html);

    loop {
        let dce_seed = build_dce_seed(&logic_doc, &support_doc, &external);
        let dropped_a = crate::css::dce(&mut logic_doc, &dce_seed);
        let dropped_b = crate::css::dce(&mut support_doc, &dce_seed);
        // For inlining, the seed must include refs from the OTHER doc
        // plus externals — `inline_single_use` adds the current doc's
        // refs internally.
        let logic_external = merge_refs(&external, &crate::css::count_refs(&support_doc));
        let support_external = merge_refs(&external, &crate::css::count_refs(&logic_doc));
        let inlined_a = crate::css::inline_single_use(&mut logic_doc, &logic_external);
        let inlined_b = crate::css::inline_single_use(&mut support_doc, &support_external);
        if dropped_a == 0 && dropped_b == 0 && inlined_a == 0 && inlined_b == 0 {
            break;
        }
    }

    *logic = crate::css::print_doc(&logic_doc);
    *support = crate::css::print_doc(&support_doc);
}

/// References that live outside the parsed buffers — base.html's
/// static rules and the JS coprocessor / debugger runtime.
fn build_external_refs(base_html: &str) -> HashMap<String, usize> {
    let mut refs = HashMap::new();
    crate::css::count_refs_in_text(base_html, &mut refs);
    for name in JS_READ_PROPERTIES {
        *refs.entry((*name).to_string()).or_insert(0) += 1;
    }
    refs
}

fn build_dce_seed(
    logic: &crate::css::Doc,
    support: &crate::css::Doc,
    external: &HashMap<String, usize>,
) -> HashMap<String, usize> {
    let mut refs = crate::css::count_refs(logic);
    for (k, v) in crate::css::count_refs(support) {
        *refs.entry(k).or_insert(0) += v;
    }
    for (k, v) in external {
        *refs.entry(k.clone()).or_insert(0) += v;
    }
    refs
}

fn merge_refs(a: &HashMap<String, usize>, b: &HashMap<String, usize>) -> HashMap<String, usize> {
    let mut out = a.clone();
    for (k, v) in b {
        *out.entry(k.clone()).or_insert(0) += v;
    }
    out
}

/// Names that look like custom properties never read via `var(...)` or
/// `style(...)` from CSS but ARE read by the JS coprocessor / debugger
/// runtime in base.html. They MUST survive DCE.
const JS_READ_PROPERTIES: &[&str] = &["_1pc", "pc", "cop_op"];

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
    fn end_to_end_inlining() {
        let mut logic =
            String::from(" --crp28: 0;\n --cri0: if(style(--pcg3: 1): var(--_1cs_sp); else: 0);\n");
        let mut support = String::from(" foo: calc((var(--cri0) * 2) + --eq1(var(--crp28)));\n");
        inline_slot_indicators(&mut logic, &mut support);
        fold_value_expressions(&mut logic, &mut support);
        assert!(
            !logic.contains("--crp28:"),
            "trivial decl should be removed"
        );
        assert!(logic.contains("--cri0:"), "non-trivial decl preserved");
        // The AST fold collapses `--eq1(0)` to `0`, drops the `+ 0`,
        // and strips the redundant parens around `var(--cri0) * 2`.
        assert!(support.contains("foo: calc(var(--cri0) * 2);"));
    }

    #[test]
    fn end_to_end_inlining_single_arm_if() {
        let mut logic = String::from(" --crp31: if(style(--_1pc: 5156): 1; else: 0);\n");
        let mut support = String::from(" foo: --eq1(var(--crp31));\n");
        inline_slot_indicators(&mut logic, &mut support);
        fold_value_expressions(&mut logic, &mut support);
        assert!(!logic.contains("--crp31:"));
        // --eq1(<bool_if>) folds to <bool_if>.
        assert!(support.contains("foo: if(style(--_1pc: 5156): 1; else: 0);"));
    }
}
