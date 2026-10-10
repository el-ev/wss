//! CSS expression AST used by the emit pipeline.

mod doc;
mod fold;
mod opt;
mod parse;
#[cfg(test)]
mod tests;

pub use doc::{
    Body, Decl, DeclItem, Doc, Item, Prelude, count_custom_names, count_refs, count_refs_in_text,
    dce, fold_doc, inline_single_use, parse_doc, print_doc, rename_custom_names, scrub_verbatim,
};
pub(crate) use doc::{is_custom_name_cont, is_custom_name_start};
pub use fold::fold;
pub(crate) use fold::is_register_name;
pub use opt::{parse_registrations, propagate_state_facts};
pub use parse::parse;
pub(crate) use parse::skip_css_string;

#[derive(Clone, Debug, PartialEq)]
pub enum Node {
    Int(i64),
    Var {
        name: String,
        fallback: Option<Box<Node>>,
    },
    Calc(Box<Node>),
    MathFn {
        name: String,
        args: Vec<Node>,
    },
    Fn {
        name: String,
        args: Vec<Node>,
    },
    Sum(Vec<Term>),
    Product(Vec<Node>),
    Div(Box<Node>, Box<Node>),
    Paren(Box<Node>),
    If {
        arms: Vec<Arm>,
        default: Box<Node>,
    },
    Style {
        prop: String,
        value: Box<Node>,
    },
    Or(Vec<Node>),
    /// A keyword: `down`, `transparent`, `jump-end`.
    Ident(String),
    /// A quoted string, quotes and escapes kept as written.
    Str(String),
    /// A number with a unit or a fraction: `12px`, `50%`, `0.5`. Unitless
    /// integers are `Int`.
    Dim {
        value: f64,
        unit: String,
    },
    /// A hash token such as a hex color, stored without the `#`.
    Hash(String),
    /// Component values joined by whitespace or commas: `1px solid #ddd`,
    /// `var(--a, X) var(--b, Y)`, `a 1s, b 2s`.
    List {
        sep: ListSep,
        items: Vec<Node>,
    },
    /// Text outside the value grammar, kept verbatim.
    Raw(String),
    /// The guaranteed-invalid value: the default of an `if()` without an
    /// `else` arm. Distinct from an empty value (`else:`).
    Invalid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ListSep {
    Space,
    Comma,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Term {
    pub sign: Sign,
    pub node: Node,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sign {
    Pos,
    Neg,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Arm {
    pub cond: Node,
    pub value: Node,
}

impl Node {
    /// The empty value, as in `if(cond: )` or `--x: ;`.
    pub fn empty() -> Self {
        Node::List {
            sep: ListSep::Space,
            items: Vec::new(),
        }
    }

    /// The text between the quotes of a string literal.
    pub fn string_contents(&self) -> Option<&str> {
        match self {
            Node::Str(s) => s.get(1..s.len() - 1),
            _ => None,
        }
    }

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
