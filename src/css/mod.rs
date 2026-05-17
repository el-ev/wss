//! CSS expression AST used by the emit pipeline.

#![allow(dead_code)]

mod doc;
mod fold;
mod parse;
#[cfg(test)]
mod tests;

#[allow(unused_imports)]
pub use doc::{
    Body, Decl, DeclItem, Doc, Item, Rule, count_refs, count_refs_in_text, dce, inline_single_use,
    parse_doc, print_doc,
};
pub use fold::fold;
pub use parse::parse;
pub(crate) use parse::skip_css_string;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    /// Integer literal. We do not currently model floats.
    Int(i64),
    /// `var(--name)` or `var(--name, fallback)`.
    Var {
        name: String,
        fallback: Option<Box<Node>>,
    },
    /// `calc(body)` — explicit math context.
    Calc(Box<Node>),
    /// CSS math function whose argument bodies inherit math context:
    /// `min`, `max`, `clamp`, `mod`, `rem`, `round`, `abs`, `sign`,
    /// plus the trig/log family.
    MathFn { name: String, args: Vec<Node> },
    /// Any other CSS function (`--eq`, `--sel`, `--read_cs`, …).
    /// Each argument starts a fresh non-math context.
    Fn { name: String, args: Vec<Node> },
    /// `a + b - c + …`. Only meaningful in math context. The printer
    /// emits a leading `+ ` or `- ` between adjacent terms; the first
    /// term's sign is folded into the term itself (e.g. as `Neg`).
    Sum(Vec<Term>),
    /// `a * b * …`. Only meaningful in math context.
    Product(Vec<Node>),
    /// `a / b`. Right-hand side is typically a small literal.
    Div(Box<Node>, Box<Node>),
    /// Explicit `(inner)` grouping. The parser preserves these where
    /// they originally appeared so the printer can reproduce them
    /// faithfully when in doubt; the fold passes may freely strip
    /// redundant ones.
    Paren(Box<Node>),
    /// `if(arms; else: default)` — the CSS-3 conditional.
    If { arms: Vec<Arm>, default: Box<Node> },
    /// `style(--prop: value)` container query. `value` is the raw text
    /// after the colon — we don't currently parse the right-hand side
    /// of a style() check.
    Style { prop: String, value: String },
    /// `cond1 or cond2 or …` — the connector inside arm conditions
    /// (and occasionally at the top level of an arm head).
    Or(Vec<Node>),
    /// Escape hatch: any token sequence we don't recognize gets stashed
    /// verbatim and re-emitted by the printer. Used for keywords like
    /// `down` inside `round(down, …)`, or for unrecognized values inside
    /// `style(--p: <value>)`.
    Raw(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Term {
    pub sign: Sign,
    pub node: Node,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    Pos,
    Neg,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arm {
    pub cond: Node,
    pub value: Node,
}

impl Node {
    /// Whether `name` denotes a CSS math function whose argument bodies
    /// inherit math context. Inside such a function, `calc(X)` is
    /// redundant with bare `(X)`; outside one, every arithmetic
    /// subexpression must be wrapped in `calc(...)`.
    pub fn is_math_function(name: &str) -> bool {
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
}

impl Term {
    pub fn pos(node: Node) -> Self {
        Self {
            sign: Sign::Pos,
            node,
        }
    }
    pub fn neg(node: Node) -> Self {
        Self {
            sign: Sign::Neg,
            node,
        }
    }
}

/// Fold every property-value expression inside `s` via the AST.
///
/// Scans the document byte by byte, tracking paren depth so that the
/// `: ` markers we look for only fire at depth 0 (i.e. between a
/// property name and its value, not inside an `if` arm or a `style()`
/// container query). When a value is found we splice from the `: ` to
/// the next `;` (or `\n`) at paren depth 0, parse it, run [`fold`] on
/// it, print it back, and substitute. Values whose body fails to
/// parse cleanly fall back to [`Node::Raw`] and survive verbatim.
pub fn fold_document_values(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let mut paren_depth: i32 = 0;
    while i < bytes.len() {
        if paren_depth == 0
            && bytes[i] == b':'
            && bytes.get(i + 1) == Some(&b' ')
            && i > 0
            && is_prop_name_byte(bytes[i - 1])
        {
            let value_start = i + 2;
            let value_end = find_value_end(bytes, value_start);
            let slice = &s[value_start..value_end];
            let folded = fold(parse(slice)).to_css();
            out.push_str(": ");
            out.push_str(&folded);
            i = value_end;
            continue;
        }
        match bytes[i] {
            b'(' => paren_depth += 1,
            b')' => paren_depth -= 1,
            _ => {}
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

fn is_prop_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'-' || b == b'_'
}

/// Find the index where the current property value ends. Stops at the
/// first `;` or `\n` at paren depth 0, or end of input. The byte at
/// the returned index is NOT consumed.
fn find_value_end(bytes: &[u8], start: usize) -> usize {
    let mut depth: i32 = 0;
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => {
                if depth == 0 {
                    // End of an enclosing scope (e.g. the closing paren
                    // of an `@function ... { result: <expr>` we never
                    // saw the `{` for); treat this as value-end.
                    return i;
                }
                depth -= 1;
            }
            b';' if depth == 0 => return i,
            b'\n' if depth == 0 => return i,
            _ => {}
        }
        i += 1;
    }
    i
}

#[cfg(test)]
mod doc_tests {
    use super::*;

    #[test]
    fn document_fold_preserves_non_value_text() {
        let input = ".foo { color: red; }\n";
        // `red` parses as Raw, prints as `red`. Net result is unchanged.
        assert_eq!(fold_document_values(input), input);
    }

    #[test]
    fn document_fold_simplifies_property_value() {
        let input = " --r12: calc(var(--x) + 0);\n";
        let expected = " --r12: var(--x);\n";
        assert_eq!(fold_document_values(input), expected);
    }

    #[test]
    fn document_fold_handles_if_value() {
        let input = " --pc: if(style(--p: 1): --eq1(1); else: 0);\n";
        let expected = " --pc: if(style(--p: 1): 1; else: 0);\n";
        assert_eq!(fold_document_values(input), expected);
    }

    #[test]
    fn document_fold_leaves_pseudo_selector_alone() {
        let input = ".btn:hover { color: red; }\n";
        // `:hover` has no following space so the trigger doesn't fire.
        assert_eq!(fold_document_values(input), input);
    }

    #[test]
    fn document_fold_preserves_function_definition_body() {
        // The `result: <expr>;` inside an @function body should be
        // folded just like any other declaration.
        let input = "@function --csmerge() { result: calc(var(--x) * 0 + var(--y)); }";
        let expected = "@function --csmerge() { result: var(--y); }";
        assert_eq!(fold_document_values(input), expected);
    }

    #[test]
    fn document_fold_unparseable_value_passes_through() {
        // Hex colors aren't in our parser's grammar; they should pass
        // through untouched.
        let input = "  color: #ff0000;\n";
        assert_eq!(fold_document_values(input), input);
    }

    #[test]
    fn document_fold_runs_compare_folds() {
        let input = " --x: --eq(2, 2);\n";
        let expected = " --x: 1;\n";
        assert_eq!(fold_document_values(input), expected);
    }
}
