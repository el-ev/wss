//! CSS expression AST used by the emit pipeline.
//!
//! Each `Node` carries enough information to know whether its subtrees
//! evaluate in CSS *math context* (where bare arithmetic is allowed) or
//! *non-math context* (where a `calc(...)` wrapper is required for any
//! arithmetic). The printer (in [`crate::print`]) uses this distinction
//! to insert `calc(...)` only where the CSS spec actually requires it,
//! eliminating the class of bug where a string-level pass strips a
//! `calc` wrapper sitting inside a non-math function argument.

// The module is wired up in stage 1 but not yet consumed by the emit
// pipeline. Suppress unused-warnings until stage 2 routes existing
// passes through it.
#![allow(dead_code)]

mod parse;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub use parse::parse;

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
