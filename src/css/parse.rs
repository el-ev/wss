use chumsky::error::Rich;
use chumsky::extra;
use chumsky::prelude::*;

use super::{Arm, ListSep, Node, Sign, Term};

pub fn parse(input: &str) -> Node {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Node::empty();
    }
    match value_parser()
        .then_ignore(end())
        .parse(trimmed)
        .into_result()
    {
        Ok(n) => n,
        Err(_) => Node::Raw(input.to_string()),
    }
}

type Err<'a> = extra::Err<Rich<'a, char>>;

/// A full value: comma-separated groups of space-separated components.
fn value_parser<'a>() -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    comma_list(space_list_parser())
}

/// Space-separated components. Function arguments and `var()` fallbacks
/// recurse back into this level.
fn space_list_parser<'a>() -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    recursive(|space_list| {
        let component = recursive(|component| {
            let atom = component.padded();
            let math = math_expr_parser(atom.clone()).boxed();

            choice((
                calc_call(math.clone()),
                var_call(comma_list(space_list.clone())),
                if_call(),
                style_call(),
                math_fn_call(math.clone()),
                user_fn_call(space_list.clone()),
                paren_atom(math),
                number(),
                string_literal(),
                hash_token(),
                bare_ident(),
            ))
        });
        // Components are only split at whitespace: `a#b` is not `a #b`.
        let gap = any()
            .filter(|c: &char| c.is_whitespace())
            .repeated()
            .at_least(1);
        component
            .clone()
            .then(gap.ignore_then(component).repeated().collect::<Vec<_>>())
            .padded()
            .map(|(first, mut rest)| {
                rest.insert(0, first);
                list(ListSep::Space, rest)
            })
    })
}

fn comma_list<'a>(
    item: impl Parser<'a, &'a str, Node, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    item.separated_by(just(',').padded())
        .at_least(1)
        .collect::<Vec<_>>()
        .map(|items| list(ListSep::Comma, items))
}

/// `)`, with any whitespace before it. Whitespace after it separates the
/// next component and is left to the caller.
fn close_paren<'a>() -> impl Parser<'a, &'a str, char, Err<'a>> + Clone {
    text::whitespace().ignore_then(just(')'))
}

fn list(sep: ListSep, mut items: Vec<Node>) -> Node {
    if items.len() == 1 {
        items.pop().unwrap()
    } else {
        Node::List { sep, items }
    }
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

/// `12`, `-4px`, `50%`, `0.5`: an `Int` when unitless and integral.
fn number<'a>() -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    let digits = any().filter(|c: &char| c.is_ascii_digit()).repeated();
    let mantissa = choice((
        digits
            .at_least(1)
            .then(just('.').then(digits.at_least(1)).or_not())
            .ignored(),
        just('.').then(digits.at_least(1)).ignored(),
    ));
    just('-')
        .or_not()
        .then(mantissa)
        .to_slice()
        .then(choice((just("%"), css_ident())).or_not())
        .try_map(|(num, unit): (&str, Option<&str>), span| {
            if unit.is_none()
                && !num.contains('.')
                && let Ok(n) = num.parse()
            {
                return Ok(Node::Int(n));
            }
            let value = num
                .parse()
                .map_err(|_| Rich::custom(span, "number out of range"))?;
            Ok(Node::Dim {
                value,
                unit: unit.unwrap_or("").to_string(),
            })
        })
}

fn string_literal<'a>() -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    custom::<_, &str, _, Err>(|input| {
        let start = input.cursor();
        let quote = match input.peek() {
            Some(q @ ('"' | '\'')) => q,
            _ => {
                let span = input.span_since(&start);
                return Err(Rich::custom(span, "expected a string"));
            }
        };
        input.next();
        loop {
            match input.next() {
                Some('\\') => {
                    input.next();
                }
                Some(c) if c == quote => break,
                Some(_) => {}
                None => {
                    let span = input.span_since(&start);
                    return Err(Rich::custom(span, "unterminated string"));
                }
            }
        }
        Ok(Node::Str(input.slice(&start..&input.cursor()).to_string()))
    })
}

fn hash_token<'a>() -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    just('#')
        .ignore_then(
            any()
                .filter(|c: &char| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .repeated()
                .at_least(1)
                .to_slice(),
        )
        .map(|s: &str| Node::Hash(s.to_string()))
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
        .then_ignore(close_paren())
        .map(|inner| Node::Calc(Box::new(inner)))
}

fn var_call<'a>(
    value: impl Parser<'a, &'a str, Node, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    let fallback = just(',').padded().ignore_then(value).or_not();
    just("var(")
        .ignore_then(css_ident().padded())
        .then(fallback)
        .then_ignore(close_paren())
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
        .then_ignore(close_paren())
        .map(|(name, args)| Node::MathFn {
            name: name.to_string(),
            args,
        })
}

fn user_fn_call<'a>(
    arg: impl Parser<'a, &'a str, Node, Err<'a>> + Clone + 'a,
) -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    css_ident()
        .filter(|s: &&str| !Node::is_math_function(s) && *s != "var" && *s != "if" && *s != "style")
        .then_ignore(just('('))
        .then(arg.separated_by(just(',').padded()).collect::<Vec<_>>())
        .then_ignore(close_paren())
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
        .then_ignore(close_paren())
        .map(|inner| Node::Paren(Box::new(inner)))
}

fn bare_ident<'a>() -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    css_ident().map(|s: &str| Node::Ident(s.to_string()))
}

fn if_call<'a>() -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    just("if(")
        .ignore_then(balanced_body())
        .then_ignore(close_paren())
        .map(|body| parse_if_body(&body))
}

fn style_call<'a>() -> impl Parser<'a, &'a str, Node, Err<'a>> + Clone {
    just("style(")
        .ignore_then(balanced_body())
        .then_ignore(close_paren())
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
    let mut default = Node::Invalid;
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
                value: Node::empty(),
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
        return style_feature(prop, &value[1..]);
    }
    Node::Raw(format!("style({})", body))
}

fn parse_paren_style_feature(s: &str) -> Option<Node> {
    let inner = s.strip_prefix('(').and_then(|x| x.strip_suffix(')'))?;
    let inner = inner.trim();
    let idx = top_level_colon(inner)?;
    let (prop, value) = inner.split_at(idx);
    Some(style_feature(prop, &value[1..]))
}

fn style_feature(prop: &str, value: &str) -> Node {
    Node::Style {
        prop: prop.trim().to_string(),
        value: Box::new(parse(value)),
    }
}

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
