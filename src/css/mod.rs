//! CSS expression AST used by the emit pipeline.

#![allow(dead_code)]

mod doc;
mod fold;
mod parse;
#[cfg(test)]
mod tests;

pub use doc::{
    Body, Decl, DeclItem, Doc, Item, count_custom_names, count_refs, count_refs_in_text, dce,
    fold_doc, inline_single_use, parse_doc, print_doc, rename_custom_names, scrub_verbatim,
};
pub(crate) use doc::{is_custom_name_cont, is_custom_name_start};
pub use fold::fold;
pub use parse::parse;
pub(crate) use parse::skip_css_string;

#[derive(Clone, Debug, PartialEq, Eq)]
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
        value: String,
    },
    Or(Vec<Node>),
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
