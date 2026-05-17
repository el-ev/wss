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
                Some(q @ ('"' | '\'')) => {
                    input.next();
                    while let Some(c) = input.peek() {
                        input.next();
                        if c == '\\' {
                            if input.peek().is_some() {
                                input.next();
                            }
                            continue;
                        }
                        if c == q {
                            break;
                        }
                    }
                }
                Some(_) => {
                    input.next();
                }
                None => break,
            }
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
        let value = parse(arm[idx + 1..].trim());
        arms.push(Arm { cond, value });
    }
    Node::If {
        arms,
        default: Box::new(default),
    }
}

fn parse_style_body(body: &str) -> Node {
    let trimmed = body.trim();
    // Compact-or form: `style((--p: 1) or (--p: 2) or ...)`. We parse
    // each `(--p: v)` clause as a Style and wrap the whole thing in
    // `Or` so it round-trips back through the printer's same-prop
    // collapse rule.
    if trimmed.starts_with('(') {
        let clauses = split_top_level_or(trimmed);
        let parsed: Option<Vec<Node>> = clauses
            .iter()
            .map(|c| parse_paren_style_feature(c.trim()))
            .collect();
        if let Some(nodes) = parsed
            && !nodes.is_empty()
        {
            if nodes.len() == 1 {
                return nodes.into_iter().next().unwrap();
            }
            return Node::Or(nodes);
        }
    }
    if let Some(idx) = top_level_colon(trimmed) {
        let (prop, value) = trimmed.split_at(idx);
        return Node::Style {
            prop: prop.trim().to_string(),
            value: value[1..].trim().to_string(),
        };
    }
    Node::Raw(format!("style({})", body))
}

/// Parse one `(--p: v)` clause from inside a compact `style()` query.
/// Returns `None` if the clause doesn't match the shape.
fn parse_paren_style_feature(s: &str) -> Option<Node> {
    let inner = s.strip_prefix('(').and_then(|x| x.strip_suffix(')'))?;
    let inner = inner.trim();
    let idx = top_level_colon(inner)?;
    let (prop, value) = inner.split_at(idx);
    Some(Node::Style {
        prop: prop.trim().to_string(),
        value: value[1..].trim().to_string(),
    })
}

/// Find the first `:` at paren depth 0 in `s`. Returns its byte offset.
fn top_level_colon(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            i = skip_css_string(bytes, i);
            continue;
        }
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b':' if depth == 0 => return Some(i),
            _ => {}
        }
        i += 1;
    }
    None
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
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            i = skip_css_string(bytes, i);
            continue;
        }
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            x if x == sep && depth == 0 => {
                parts.push(&s[start..i]);
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    parts.push(&s[start..]);
    parts
}

pub(crate) fn skip_css_string(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => i += 2,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

fn find_arm_colon(arm: &str) -> Option<usize> {
    let bytes = arm.as_bytes();
    let mut depth: i32 = 0;
    let mut best = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            i = skip_css_string(bytes, i);
            continue;
        }
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b':' if depth == 0 => {
                best = Some(i);
            }
            _ => {}
        }
        i += 1;
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
