//! Chumsky-based parser for the CSS expressions wss emits.
//!
//! The grammar is small and entirely under our control. Anything we
//! don't model — bare keywords inside `round()`, unrecognized syntax
//! inside `style()` values — falls back to [`Node::Raw`] and is
//! ferried through unchanged by the printer.
//!
//! Two parsers cooperate via [`recursive`]:
//!  - `atom` — a single value: `Int`, `Var`, `Calc`, `Fn`, `MathFn`,
//!    `If`, `Style`, `Paren`, …
//!  - `math` — a sum-of-products built from atoms, only legal inside
//!    a math context.
//!
//! [`parse`] is the public entry point. It returns the parsed node, or
//! `Raw(input)` if parsing fails (so the printer can re-emit the
//! original verbatim).

use chumsky::error::Rich;
use chumsky::extra;
use chumsky::prelude::*;

use super::{Arm, Node, Sign, Term};

/// Parse a CSS value expression. On failure, the entire input is
/// preserved as [`Node::Raw`].
pub fn parse(input: &str) -> Node {
    let trimmed = input.trim();
    match atom_parser()
        .then_ignore(end())
        .parse(trimmed)
        .into_result()
    {
        Ok(n) => n,
        Err(_) => Node::Raw(input.to_string()),
    }
}

type Err<'a> = extra::Err<Rich<'a, char>>;

fn atom_parser<'a>() -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    recursive(|atom| {
        let math = math_expr_parser(atom.clone()).boxed();

        choice((
            // Order matters: more-specific keywords (calc/var/if/style)
            // before the generic math-fn / user-fn / ident dispatch.
            calc_call(math.clone()),
            var_call(atom.clone()),
            if_call(),
            style_call(),
            math_fn_call(math.clone()),
            user_fn_call(atom.clone()),
            paren_atom(math),
            int_literal(),
            bare_ident(),
        ))
        .padded()
    })
}

/// Sum-of-products in math context. Uses `atom` as factor.
fn math_expr_parser<'a>(
    atom: impl Parser<'a, &'a str, Node, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    let product = atom.clone().foldl(
        choice((just('*').padded().to('*'), just('/').padded().to('/')))
            .then(atom)
            .repeated(),
        |lhs, (op, rhs)| match op {
            '*' => match lhs {
                Node::Product(mut fs) => {
                    fs.push(rhs);
                    Node::Product(fs)
                }
                other => Node::Product(vec![other, rhs]),
            },
            '/' => Node::Div(Box::new(lhs), Box::new(rhs)),
            _ => unreachable!(),
        },
    );

    product.clone().foldl(
        choice((
            just('+').padded().to(Sign::Pos),
            just('-').padded().to(Sign::Neg),
        ))
        .then(product)
        .repeated(),
        |lhs, (sign, rhs)| {
            let mut terms = match lhs {
                Node::Sum(ts) => ts,
                other => vec![Term::pos(other)],
            };
            terms.push(Term { sign, node: rhs });
            Node::Sum(terms)
        },
    )
}

fn int_literal<'a>() -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    just('-')
        .or_not()
        .then(text::int(10))
        .to_slice()
        .map(|s: &str| Node::Int(s.parse().unwrap_or(0)))
}

/// CSS-flavored identifier: starts with a letter or `-`, continues with
/// letters/digits/`-`/`_`.
fn css_ident<'a>() -> impl Parser<'a, &'a str, &'a str, Err<'a>> + Clone {
    any()
        .filter(|c: &char| c.is_ascii_alphabetic() || *c == '-' || *c == '_')
        .then(
            any()
                .filter(|c: &char| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .repeated(),
        )
        .to_slice()
}

fn calc_call<'a>(
    math: impl Parser<'a, &'a str, Node, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    just("calc(")
        .ignore_then(math)
        .then_ignore(just(')').padded())
        .map(|inner| Node::Calc(Box::new(inner)))
}

fn var_call<'a>(
    atom: impl Parser<'a, &'a str, Node, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    let fallback = just(',').padded().ignore_then(atom).or_not();
    just("var(")
        .ignore_then(css_ident().padded())
        .then(fallback)
        .then_ignore(just(')').padded())
        .map(|(name, fb)| Node::Var {
            name: name.to_string(),
            fallback: fb.map(Box::new),
        })
}

fn math_fn_call<'a>(
    math: impl Parser<'a, &'a str, Node, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    css_ident()
        .filter(|s: &&str| Node::is_math_function(s) && *s != "calc")
        .then_ignore(just('('))
        .then(
            math.separated_by(just(',').padded())
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(')').padded())
        .map(|(name, args)| Node::MathFn {
            name: name.to_string(),
            args,
        })
}

fn user_fn_call<'a>(
    atom: impl Parser<'a, &'a str, Node, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    css_ident()
        .filter(|s: &&str| !Node::is_math_function(s) && *s != "var" && *s != "if" && *s != "style")
        .then_ignore(just('('))
        .then(
            atom.separated_by(just(',').padded())
                .at_least(1)
                .collect::<Vec<_>>(),
        )
        .then_ignore(just(')').padded())
        .map(|(name, args)| Node::Fn {
            name: name.to_string(),
            args,
        })
}

fn paren_atom<'a>(
    math: impl Parser<'a, &'a str, Node, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    just('(')
        .ignore_then(math)
        .then_ignore(just(')').padded())
        .map(|inner| Node::Paren(Box::new(inner)))
}

fn bare_ident<'a>() -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    css_ident().map(|s: &str| Node::Raw(s.to_string()))
}

// ---------------------------------------------------------------------
// `if(...)` and `style(...)` have grammars (semicolon-separated arms,
// `else:` default, raw-text values inside `style(...)`) that don't fit
// chumsky's pure-combinator model neatly. We read the parenthesized
// body as a single balanced slice via [`balanced_body`] and then split
// it with small hand-rolled helpers.
// ---------------------------------------------------------------------

fn if_call<'a>() -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    just("if(")
        .ignore_then(balanced_body())
        .then_ignore(just(')').padded())
        .map(|body| parse_if_body(&body))
}

fn style_call<'a>() -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    just("style(")
        .ignore_then(balanced_body())
        .then_ignore(just(')').padded())
        .map(|body| parse_style_body(&body))
}

/// Consume characters up to the matching close paren of an *already
/// opened* call (i.e. the leading `name(` has already been parsed).
/// Tracks paren depth so nested `()` inside the body don't terminate
/// the match early. Built as a custom parser because chumsky's
/// `recursive` combinator struggles with the inner-type inference
/// when the result is a `String`.
fn balanced_body<'a>() -> impl Parser<'a, &'a str, String, Err<'a>> + Clone {
    custom::<_, &str, _, Err>(|input| {
        let start = input.cursor();
        let mut depth: i32 = 0;
        loop {
            let before = input.cursor();
            match input.peek() {
                Some(')') if depth == 0 => break,
                Some('(') => {
                    depth += 1;
                    input.next();
                }
                Some(')') => {
                    depth -= 1;
                    input.next();
                }
                Some(_) => {
                    input.next();
                }
                None => break,
            }
            // Defensive: cursor must advance.
            debug_assert!(input.cursor() != before);
        }
        Ok(input.slice(&start..&input.cursor()).to_string())
    })
}

fn parse_if_body(body: &str) -> Node {
    let parts = split_top_level(body, b';');
    let mut arms: Vec<Arm> = Vec::new();
    let mut default = Node::Raw(String::new());
    for arm in parts {
        let arm = arm.trim();
        if arm.is_empty() {
            continue;
        }
        if let Some(rest) = arm.strip_prefix("else:") {
            default = parse(rest.trim());
            continue;
        }
        let Some(idx) = find_arm_colon(arm) else {
            arms.push(Arm {
                cond: Node::Raw(arm.to_string()),
                value: Node::Raw(String::new()),
            });
            continue;
        };
        let cond = parse_cond(&arm[..idx]);
        let value = parse(arm[idx + 2..].trim());
        arms.push(Arm { cond, value });
    }
    Node::If {
        arms,
        default: Box::new(default),
    }
}

fn parse_style_body(body: &str) -> Node {
    if let Some((prop, value)) = body.split_once(':') {
        return Node::Style {
            prop: prop.trim().to_string(),
            value: value.trim().to_string(),
        };
    }
    Node::Raw(format!("style({})", body))
}

fn parse_cond(s: &str) -> Node {
    let parts = split_top_level_or(s);
    if parts.len() <= 1 {
        return parse(s.trim());
    }
    Node::Or(parts.into_iter().map(|p| parse(p.trim())).collect())
}

fn split_top_level(s: &str, sep: u8) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0;
    let bytes = s.as_bytes();
    for (i, &b) in bytes.iter().enumerate() {
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

fn find_arm_colon(arm: &str) -> Option<usize> {
    let bytes = arm.as_bytes();
    let mut depth: i32 = 0;
    let mut best = None;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b':' if depth == 0 && bytes.get(i + 1) == Some(&b' ') => {
                best = Some(i);
            }
            _ => {}
        }
    }
    best
}

fn split_top_level_or(s: &str) -> Vec<&str> {
    let bytes = s.as_bytes();
    let mut parts = Vec::new();
    let mut depth: i32 = 0;
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => {
                depth += 1;
                i += 1;
            }
            b')' => {
                depth -= 1;
                i += 1;
            }
            b' ' if depth == 0 && s[i..].starts_with(" or ") => {
                parts.push(&s[start..i]);
                i += 4;
                start = i;
            }
            _ => i += 1,
        }
    }
    parts.push(&s[start..]);
    parts
}
