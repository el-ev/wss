use super::Node;

#[derive(Clone, Debug, PartialEq)]
pub struct Doc {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Item {
    /// Whitespace, comments, or text the parser does not model.
    Verbatim(String),
    Rule(Rule),
    Decl(Decl),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Rule {
    pub prelude: Prelude,
    /// Whitespace between the prelude and `{`.
    pub head_pad: String,
    pub open_pad: String,
    pub body: Body,
    pub close_pad: String,
}

/// What a rule applies to: the text before its `{`.
#[derive(Clone, Debug, PartialEq)]
pub enum Prelude {
    /// A selector list (`.clk`, `0%, 100%`), kept as text.
    Selector(String),
    /// `@property --name`
    Property(String),
    /// `@function --name(--p <type>, ...) returns <type>`
    Function(FunctionHead),
    /// `@keyframes name`
    Keyframes(String),
    /// `@container <condition>`
    Container(Node),
    /// Any other at-rule, e.g. `@media (max-width: 720px)`.
    AtRule { name: String, prelude: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct FunctionHead {
    pub name: String,
    pub params: Vec<Param>,
    /// The `returns` type, e.g. `<integer>`.
    pub returns: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    /// The parameter type, e.g. `<number>`.
    pub syntax: Option<String>,
}

impl Prelude {
    pub fn is_property(&self) -> bool {
        matches!(self, Prelude::Property(_))
    }

    pub fn is_function(&self) -> bool {
        matches!(self, Prelude::Function(_))
    }

    pub fn is_keyframes(&self) -> bool {
        matches!(self, Prelude::Keyframes(_))
    }

    pub fn to_css(&self) -> String {
        let mut out = String::new();
        print_prelude(self, &mut out);
        out
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Body {
    Decls(Vec<DeclItem>),
    Nested(Vec<Item>),
}

#[derive(Clone, Debug, PartialEq)]
pub enum DeclItem {
    /// Whitespace, comments, or text the parser does not model.
    Verbatim(String),
    Decl(Decl),
}

#[derive(Clone, Debug, PartialEq)]
pub struct Decl {
    pub name: String,
    pub value: Node,
    pub important: bool,
}

impl Decl {
    pub fn new(name: String, value: Node) -> Self {
        Self {
            name,
            value,
            important: false,
        }
    }
}

// =====================================================================
// Parser
// =====================================================================

pub fn parse_doc(input: &str) -> Doc {
    let mut p = DocParser::new(input);
    let items = p.parse_items_to_end();
    Doc { items }
}

struct DocParser<'a> {
    input: &'a str,
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> DocParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input,
            bytes: input.as_bytes(),
            pos: 0,
        }
    }

    /// Parse items until end of input. Used both for the top-level
    /// document and for nested rule bodies (`@keyframes`, `@container`).
    fn parse_items_to_end(&mut self) -> Vec<Item> {
        let mut items: Vec<Item> = Vec::new();
        let mut verbatim_start = self.pos;
        // Where the next rule's head would start: just past the last
        // top-level `;` or `}`.
        let mut head_start = self.pos;
        let mut depth: i32 = 0;
        while self.pos < self.bytes.len() {
            match self.bytes[self.pos] {
                // Skip over a quoted string literal so it doesn't confuse
                // brace-matching.
                b'"' | b'\'' => {
                    self.pos = super::parse::skip_css_string(self.bytes, self.pos);
                    continue;
                }
                b'(' => depth += 1,
                b')' => depth -= 1,
                b';' if depth == 0 => head_start = self.pos + 1,
                b'}' => head_start = self.pos + 1,
                _ => {}
            }
            if self.bytes[self.pos] == b'{' {
                let pre_text = &self.input[verbatim_start..head_start];
                push_text_as_items(pre_text, &mut items);
                let (trivia, head) = split_leading_trivia(&self.input[head_start..self.pos]);
                if !trivia.is_empty() {
                    items.push(Item::Verbatim(trivia.to_string()));
                }
                let head_end = head.trim_end_matches(is_css_space).len();
                let prelude = parse_prelude(&head[..head_end]);
                self.pos += 1; // consume `{`
                let (open_pad, body, close_pad) = self.parse_rule_body(&prelude);
                items.push(Item::Rule(Rule {
                    prelude,
                    head_pad: head[head_end..].to_string(),
                    open_pad,
                    body,
                    close_pad,
                }));
                verbatim_start = self.pos;
                head_start = self.pos;
                depth = 0;
                continue;
            }
            self.pos += 1;
        }
        let tail = &self.input[verbatim_start..self.pos];
        push_text_as_items(tail, &mut items);
        items
    }

    fn parse_rule_body(&mut self, prelude: &Prelude) -> (String, Body, String) {
        // Find matching `}` while respecting nested braces and string
        // literals.
        let body_start = self.pos;
        let mut depth: i32 = 1;
        let mut i = self.pos;
        while i < self.bytes.len() {
            match self.bytes[i] {
                b'"' | b'\'' => {
                    i = super::parse::skip_css_string(self.bytes, i);
                    continue;
                }
                b'{' => depth += 1,
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
            i += 1;
        }
        let body_end = i;
        // Skip past the closing `}` (if present).
        self.pos = body_end + if body_end < self.bytes.len() { 1 } else { 0 };

        let body_text = &self.input[body_start..body_end];
        let (open_pad, inner, close_pad) = trim_pads(body_text);
        let body = classify_body(prelude, inner);
        (open_pad.to_string(), body, close_pad.to_string())
    }
}

fn push_text_as_items(text: &str, out: &mut Vec<Item>) {
    if text.is_empty() {
        return;
    }
    if any_unquoted(text, |b, _| b == b'{' || b == b'}') {
        out.push(Item::Verbatim(text.to_string()));
        return;
    }
    for di in parse_decl_items(text) {
        match di {
            DeclItem::Verbatim(s) => out.push(Item::Verbatim(s)),
            DeclItem::Decl(d) => out.push(Item::Decl(d)),
        }
    }
}

fn is_css_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r')
}

/// Length of the comment opening `s`, if `s` starts with one.
fn comment_len(s: &str) -> Option<usize> {
    let rest = s.strip_prefix("/*")?;
    Some(rest.find("*/").map_or(s.len(), |i| i + 4))
}

/// Splits leading whitespace and comments off `s`.
fn split_leading_trivia(s: &str) -> (&str, &str) {
    let mut i = 0;
    loop {
        let rest = &s[i..];
        let ws = rest.len() - rest.trim_start_matches(is_css_space).len();
        if ws > 0 {
            i += ws;
        } else if let Some(n) = comment_len(rest) {
            i += n;
        } else {
            return s.split_at(i);
        }
    }
}

fn trim_pads(s: &str) -> (&str, &str, &str) {
    let leading_end = s.len() - s.trim_start_matches(is_css_space).len();
    let trailing_start = s.trim_end_matches(is_css_space).len();
    if trailing_start < leading_end {
        // All whitespace.
        return (s, "", "");
    }
    (
        &s[..leading_end],
        &s[leading_end..trailing_start],
        &s[trailing_start..],
    )
}

fn parse_prelude(text: &str) -> Prelude {
    let Some(rest) = text.strip_prefix('@') else {
        return Prelude::Selector(text.to_string());
    };
    let name_end = rest
        .bytes()
        .position(|b| !is_custom_name_cont(b))
        .unwrap_or(rest.len());
    let (name, prelude) = (&rest[..name_end], rest[name_end..].trim());
    let is_name = |s: &str| !s.is_empty() && s.bytes().all(is_custom_name_cont);
    match name {
        "property" if prelude.starts_with("--") && is_name(prelude) => {
            return Prelude::Property(prelude.to_string());
        }
        "keyframes" if is_name(prelude) => return Prelude::Keyframes(prelude.to_string()),
        "function" => {
            if let Some(head) = parse_function_head(prelude) {
                return Prelude::Function(head);
            }
        }
        "container" if !prelude.is_empty() => {
            return Prelude::Container(super::parse(prelude));
        }
        _ => {}
    }
    Prelude::AtRule {
        name: name.to_string(),
        prelude: prelude.to_string(),
    }
}

/// `--name(--a <number>, --b) returns <integer>`
fn parse_function_head(s: &str) -> Option<FunctionHead> {
    let open = s.find('(')?;
    let close = open + s[open..].find(')')?;
    let name = s[..open].trim();
    if !name.starts_with("--") || !name.bytes().all(is_custom_name_cont) {
        return None;
    }
    let params_text = s[open + 1..close].trim();
    let params = if params_text.is_empty() {
        Vec::new()
    } else {
        params_text
            .split(',')
            .map(|p| {
                let mut words = p.split_whitespace();
                let name = words.next().filter(|n| n.starts_with("--"))?;
                let syntax = words.next().map(str::to_string);
                // Defaults (`--a <number>: 0`) are not modeled.
                if words.next().is_some() || name.contains(':') {
                    return None;
                }
                Some(Param {
                    name: name.to_string(),
                    syntax,
                })
            })
            .collect::<Option<Vec<_>>>()?
    };
    let tail = s[close + 1..].trim();
    let returns = if tail.is_empty() {
        None
    } else {
        let ty = tail.strip_prefix("returns")?.trim();
        if ty.is_empty() || ty.contains(char::is_whitespace) {
            return None;
        }
        Some(ty.to_string())
    };
    Some(FunctionHead {
        name: name.to_string(),
        params,
        returns,
    })
}

fn classify_body(prelude: &Prelude, inner: &str) -> Body {
    if matches!(prelude, Prelude::Keyframes(_) | Prelude::Container(_))
        || contains_top_level_brace(inner)
    {
        let mut p = DocParser::new(inner);
        return Body::Nested(p.parse_items_to_end());
    }
    Body::Decls(parse_decl_items(inner))
}

fn contains_top_level_brace(s: &str) -> bool {
    any_unquoted(s, |b, depth| b == b'{' && depth == 0)
}

/// Whether `pred` holds for some byte of `s` outside string literals,
/// given the paren depth before it.
fn any_unquoted(s: &str, mut pred: impl FnMut(u8, i32) -> bool) -> bool {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            i = super::parse::skip_css_string(bytes, i);
            continue;
        }
        if pred(b, depth) {
            return true;
        }
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    false
}

fn parse_decl_items(inner: &str) -> Vec<DeclItem> {
    let bytes = inner.as_bytes();
    let mut items: Vec<DeclItem> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // Leading whitespace and comments stay verbatim.
        let (trivia, _) = split_leading_trivia(&inner[i..]);
        if !trivia.is_empty() {
            items.push(DeclItem::Verbatim(trivia.to_string()));
            i += trivia.len();
        }
        if i >= bytes.len() {
            break;
        }
        let decl_start = i;
        let mut depth: i32 = 0;
        while i < bytes.len() {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b';' if depth == 0 => {
                    i += 1; // include the `;`
                    break;
                }
                b'"' | b'\'' => {
                    i = super::parse::skip_css_string(bytes, i);
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        // Trailing whitespace before the end of a final, `;`-less
        // declaration is left for the next round.
        let decl_text = inner[decl_start..i].trim_end_matches(is_css_space);
        i = decl_start + decl_text.len();
        match parse_decl(decl_text) {
            Some(decl) => items.push(DeclItem::Decl(decl)),
            None => items.push(DeclItem::Verbatim(decl_text.to_string())),
        }
    }
    items
}

fn parse_decl(slice: &str) -> Option<Decl> {
    let trimmed = slice.strip_suffix(';').unwrap_or(slice);
    let colon = find_decl_colon(trimmed)?;
    let name = trimmed[..colon].trim();
    if name.is_empty() || !name.bytes().all(is_custom_name_cont) {
        return None;
    }
    let value_str = trimmed[colon + 1..].trim();
    let (value_str, important) = match value_str.strip_suffix("!important") {
        Some(v) => (v.trim_end(), true),
        None => (value_str, false),
    };
    Some(Decl {
        name: name.to_string(),
        value: super::parse(value_str),
        important,
    })
}

fn find_decl_colon(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut depth: i32 = 0;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b':' if depth == 0 => {
                // Skip `::` pseudo-element prefix.
                if bytes.get(i + 1) == Some(&b':') {
                    continue;
                }
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

// =====================================================================
// Printer
// =====================================================================

pub fn print_doc(doc: &Doc) -> String {
    let mut out = String::new();
    print_items(&doc.items, &mut out);
    out
}

fn print_decl(d: &Decl, out: &mut String) {
    out.push_str(&d.name);
    out.push_str(": ");
    out.push_str(&d.value.to_css());
    if d.important {
        out.push_str(" !important");
    }
    out.push(';');
}

fn print_items(items: &[Item], out: &mut String) {
    for item in items {
        match item {
            Item::Verbatim(s) => out.push_str(s),
            Item::Rule(r) => print_rule(r, out),
            Item::Decl(d) => print_decl(d, out),
        }
    }
}

fn print_prelude(prelude: &Prelude, out: &mut String) {
    let at_rule = |out: &mut String, name: &str, prelude: &str| {
        out.push('@');
        out.push_str(name);
        if !prelude.is_empty() {
            out.push(' ');
            out.push_str(prelude);
        }
    };
    match prelude {
        Prelude::Selector(s) => out.push_str(s),
        Prelude::Property(name) => at_rule(out, "property", name),
        Prelude::Keyframes(name) => at_rule(out, "keyframes", name),
        Prelude::Container(cond) => at_rule(out, "container", &cond.to_css()),
        Prelude::AtRule { name, prelude } => at_rule(out, name, prelude),
        Prelude::Function(f) => {
            out.push_str("@function ");
            out.push_str(&f.name);
            out.push('(');
            for (i, p) in f.params.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&p.name);
                if let Some(syntax) = &p.syntax {
                    out.push(' ');
                    out.push_str(syntax);
                }
            }
            out.push(')');
            if let Some(ty) = &f.returns {
                out.push_str(" returns ");
                out.push_str(ty);
            }
        }
    }
}

fn print_rule(r: &Rule, out: &mut String) {
    print_prelude(&r.prelude, out);
    out.push_str(&r.head_pad);
    out.push('{');
    out.push_str(&r.open_pad);
    match &r.body {
        Body::Decls(items) => {
            for di in items {
                match di {
                    DeclItem::Verbatim(s) => out.push_str(s),
                    DeclItem::Decl(d) => print_decl(d, out),
                }
            }
        }
        Body::Nested(items) => print_items(items, out),
    }
    out.push_str(&r.close_pad);
    out.push('}');
}

// =====================================================================
// Reference counting + DCE
// =====================================================================

use std::collections::HashMap;

/// Pre-order traversal of every node reachable from `node`. The
/// callback observes each node before recursion descends into its
/// children. Shared by every read-only value walker in this module —
/// the recursion shape (which variants carry children, in what order)
/// only lives here.
fn visit_value(node: &Node, visit: &mut impl FnMut(&Node)) {
    visit(node);
    match node {
        Node::Int(_)
        | Node::Raw(_)
        | Node::Invalid
        | Node::Ident(_)
        | Node::Str(_)
        | Node::Dim { .. }
        | Node::Hash(_) => {}
        Node::Style { value, .. } => visit_value(value, visit),
        Node::List { items, .. } => {
            for item in items {
                visit_value(item, visit);
            }
        }
        Node::Var { fallback, .. } => {
            if let Some(fb) = fallback {
                visit_value(fb, visit);
            }
        }
        Node::Calc(inner) | Node::Paren(inner) => visit_value(inner, visit),
        Node::MathFn { args, .. } | Node::Fn { args, .. } => {
            for a in args {
                visit_value(a, visit);
            }
        }
        Node::Sum(terms) => {
            for t in terms {
                visit_value(&t.node, visit);
            }
        }
        Node::Product(factors) => {
            for f in factors {
                visit_value(f, visit);
            }
        }
        Node::Div(l, r) => {
            visit_value(l, visit);
            visit_value(r, visit);
        }
        Node::If { arms, default } => {
            for arm in arms {
                visit_value(&arm.cond, visit);
                visit_value(&arm.value, visit);
            }
            visit_value(default, visit);
        }
        Node::Or(conds) => {
            for c in conds {
                visit_value(c, visit);
            }
        }
    }
}

pub(super) fn visit_value_mut(node: &mut Node, visit: &mut impl FnMut(&mut Node)) {
    visit(node);
    match node {
        Node::Int(_)
        | Node::Raw(_)
        | Node::Invalid
        | Node::Ident(_)
        | Node::Str(_)
        | Node::Dim { .. }
        | Node::Hash(_) => {}
        Node::Style { value, .. } => visit_value_mut(value, visit),
        Node::List { items, .. } => {
            for item in items {
                visit_value_mut(item, visit);
            }
        }
        Node::Var { fallback, .. } => {
            if let Some(fb) = fallback {
                visit_value_mut(fb, visit);
            }
        }
        Node::Calc(inner) | Node::Paren(inner) => visit_value_mut(inner, visit),
        Node::MathFn { args, .. } | Node::Fn { args, .. } => {
            for a in args {
                visit_value_mut(a, visit);
            }
        }
        Node::Sum(terms) => {
            for t in terms {
                visit_value_mut(&mut t.node, visit);
            }
        }
        Node::Product(factors) => {
            for f in factors {
                visit_value_mut(f, visit);
            }
        }
        Node::Div(l, r) => {
            visit_value_mut(l, visit);
            visit_value_mut(r, visit);
        }
        Node::If { arms, default } => {
            for arm in arms {
                visit_value_mut(&mut arm.cond, visit);
                visit_value_mut(&mut arm.value, visit);
            }
            visit_value_mut(default, visit);
        }
        Node::Or(conds) => {
            for c in conds {
                visit_value_mut(c, visit);
            }
        }
    }
}

pub fn count_refs(doc: &Doc) -> HashMap<String, usize> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    walk_items(&doc.items, &mut counts);
    counts
}

fn walk_items(items: &[Item], counts: &mut HashMap<String, usize>) {
    for item in items {
        match item {
            Item::Rule(r) => walk_rule(r, counts),
            Item::Decl(d) => walk_value(&d.value, counts),
            Item::Verbatim(s) => count_refs_in_text(s, counts),
        }
    }
}

fn walk_rule(rule: &Rule, counts: &mut HashMap<String, usize>) {
    if let Prelude::Container(cond) = &rule.prelude {
        walk_value(cond, counts);
    }
    match &rule.body {
        Body::Decls(items) => {
            for di in items {
                if let DeclItem::Decl(d) = di {
                    walk_value(&d.value, counts);
                }
            }
        }
        Body::Nested(items) => walk_items(items, counts),
    }
}

fn walk_value(node: &Node, counts: &mut HashMap<String, usize>) {
    visit_value(node, &mut |n| match n {
        Node::Var { name, .. } => bump_ref(counts, name),
        Node::Style { prop, .. } => bump_ref(counts, prop),
        Node::Raw(s) => count_refs_in_text(s, counts),
        _ => {}
    });
}

fn bump_ref(counts: &mut HashMap<String, usize>, full_name: &str) {
    let stripped = full_name.strip_prefix("--").unwrap_or(full_name);
    *counts.entry(stripped.to_string()).or_insert(0) += 1;
}

pub fn dce(doc: &mut Doc, seed_refs: &HashMap<String, usize>) -> usize {
    let mut total = 0usize;
    loop {
        let mut refs = count_refs(doc);
        for (k, v) in seed_refs {
            *refs.entry(k.clone()).or_insert(0) += v;
        }
        let dropped = drop_dead_in_items(&mut doc.items, &refs);
        if dropped == 0 {
            break;
        }
        total += dropped;
    }
    total
}

fn drop_dead_in_items(items: &mut Vec<Item>, refs: &HashMap<String, usize>) -> usize {
    let mut dropped = 0usize;
    items.retain(|item| {
        let Item::Decl(d) = item else {
            return true;
        };
        let Some(key) = d.name.strip_prefix("--") else {
            return true;
        };
        let count = refs.get(key).copied().unwrap_or(0);
        if count == 0 {
            dropped += 1;
            false
        } else {
            true
        }
    });
    for item in items.iter_mut() {
        if let Item::Rule(r) = item {
            dropped += drop_dead_in_rule(r, refs);
        }
    }
    dropped
}

fn drop_dead_in_rule(rule: &mut Rule, refs: &HashMap<String, usize>) -> usize {
    let mut dropped = 0usize;
    let scoped = rule.prelude.is_function() || rule.prelude.is_property();
    match &mut rule.body {
        Body::Decls(items) if !scoped => {
            items.retain(|di| {
                let DeclItem::Decl(d) = di else {
                    return true;
                };
                let Some(key) = d.name.strip_prefix("--") else {
                    return true;
                };
                let count = refs.get(key).copied().unwrap_or(0);
                if count == 0 {
                    dropped += 1;
                    false
                } else {
                    true
                }
            });
        }
        Body::Nested(items) => dropped += drop_dead_in_items(items, refs),
        _ => {}
    }
    dropped
}

pub fn count_refs_in_text(s: &str, counts: &mut HashMap<String, usize>) {
    scan_text_for(s, b"var(", b',', b')', counts);
    scan_text_for(s, b"style(", b':', b':', counts);
}

pub fn fold_doc(doc: &mut Doc) {
    fold_in_items(&mut doc.items);
}

fn fold_in_items(items: &mut [Item]) {
    for item in items {
        match item {
            Item::Decl(d) => d.value = super::fold(std::mem::replace(&mut d.value, Node::Int(0))),
            Item::Rule(r) => fold_in_rule(r),
            Item::Verbatim(_) => {}
        }
    }
}

fn fold_in_rule(rule: &mut Rule) {
    match &mut rule.body {
        Body::Decls(items) => {
            for di in items {
                if let DeclItem::Decl(d) = di {
                    d.value = super::fold(std::mem::replace(&mut d.value, Node::Int(0)));
                }
            }
        }
        Body::Nested(items) => fold_in_items(items),
    }
}

// =====================================================================
// Single-use inlining
// =====================================================================

pub fn inline_single_use(doc: &mut Doc, seed_refs: &HashMap<String, usize>) -> usize {
    let (var_refs, text_refs) = count_var_and_text_refs(doc);
    let mut total_refs = var_refs.clone();
    for (k, v) in text_refs {
        *total_refs.entry(k).or_insert(0) += v;
    }
    for (k, v) in seed_refs {
        *total_refs.entry(k.clone()).or_insert(0) += v;
    }

    let mut candidates: HashMap<String, Node> = HashMap::new();
    collect_inline_candidates(&doc.items, &total_refs, &var_refs, &mut candidates);
    if candidates.is_empty() {
        return 0;
    }

    let names: std::collections::HashSet<String> = candidates.keys().cloned().collect();
    candidates.retain(|_, v| !value_references_any(v, &names));
    if candidates.is_empty() {
        return 0;
    }

    let mut substituted: std::collections::HashSet<String> = std::collections::HashSet::new();
    substitute_in_items(&mut doc.items, &candidates, &mut substituted);
    drop_inlined(&mut doc.items, &substituted);
    substituted.len()
}

fn count_var_and_text_refs(doc: &Doc) -> (HashMap<String, usize>, HashMap<String, usize>) {
    let mut var_refs = HashMap::new();
    let mut text_refs = HashMap::new();
    walk_items_var_text(&doc.items, &mut var_refs, &mut text_refs);
    (var_refs, text_refs)
}

fn walk_items_var_text(
    items: &[Item],
    var_refs: &mut HashMap<String, usize>,
    text_refs: &mut HashMap<String, usize>,
) {
    for item in items {
        match item {
            Item::Rule(r) => walk_rule_var_text(r, var_refs, text_refs),
            Item::Decl(d) => walk_decl_var_text(d, var_refs, text_refs),
            Item::Verbatim(s) => count_refs_in_text(s, text_refs),
        }
    }
}

fn walk_rule_var_text(
    rule: &Rule,
    var_refs: &mut HashMap<String, usize>,
    text_refs: &mut HashMap<String, usize>,
) {
    if let Prelude::Container(cond) = &rule.prelude {
        walk_value(cond, text_refs);
    }
    match &rule.body {
        Body::Decls(items) => {
            for di in items {
                match di {
                    DeclItem::Verbatim(s) => count_refs_in_text(s, text_refs),
                    DeclItem::Decl(d) => walk_decl_var_text(d, var_refs, text_refs),
                }
            }
        }
        Body::Nested(items) => walk_items_var_text(items, var_refs, text_refs),
    }
}

/// Only custom-property values take inlined values: the emitter chunks long
/// standard-property values (the visualizers' `box-shadow` lists) into
/// custom properties on purpose.
fn walk_decl_var_text(
    d: &Decl,
    var_refs: &mut HashMap<String, usize>,
    text_refs: &mut HashMap<String, usize>,
) {
    if d.name.starts_with("--") {
        walk_value_var_text(&d.value, var_refs, text_refs);
    } else {
        walk_value(&d.value, text_refs);
    }
}

fn walk_value_var_text(
    node: &Node,
    var_refs: &mut HashMap<String, usize>,
    text_refs: &mut HashMap<String, usize>,
) {
    visit_value(node, &mut |n| match n {
        Node::Var { name, .. } => bump_ref(var_refs, name),
        Node::Style { prop, .. } => bump_ref(text_refs, prop),
        Node::Raw(s) => count_refs_in_text(s, text_refs),
        _ => {}
    });
}

fn collect_inline_candidates(
    items: &[Item],
    total_refs: &HashMap<String, usize>,
    var_refs: &HashMap<String, usize>,
    candidates: &mut HashMap<String, Node>,
) {
    for item in items {
        match item {
            Item::Decl(d) => consider_candidate(d, total_refs, var_refs, candidates),
            Item::Rule(r) => collect_in_rule(r, total_refs, var_refs, candidates),
            Item::Verbatim(_) => {}
        }
    }
}

fn collect_in_rule(
    rule: &Rule,
    total_refs: &HashMap<String, usize>,
    var_refs: &HashMap<String, usize>,
    candidates: &mut HashMap<String, Node>,
) {
    // Keyframe values are latched snapshots: inlining one into its reader
    // would read the live value instead.
    let p = &rule.prelude;
    if p.is_function() || p.is_property() || p.is_keyframes() {
        return;
    }
    match &rule.body {
        Body::Decls(items) => {
            for di in items {
                if let DeclItem::Decl(d) = di {
                    consider_candidate(d, total_refs, var_refs, candidates);
                }
            }
        }
        Body::Nested(items) => collect_inline_candidates(items, total_refs, var_refs, candidates),
    }
}

fn consider_candidate(
    d: &Decl,
    total_refs: &HashMap<String, usize>,
    var_refs: &HashMap<String, usize>,
    candidates: &mut HashMap<String, Node>,
) {
    let Some(key) = d.name.strip_prefix("--") else {
        return;
    };

    if total_refs.get(key).copied().unwrap_or(0) != 1 {
        return;
    }
    if var_refs.get(key).copied().unwrap_or(0) != 1 {
        return;
    }

    if matches!(&d.value, Node::Var { fallback: None, .. }) {
        return;
    }
    candidates.insert(key.to_string(), d.value.clone());
}

fn value_references_any(node: &Node, names: &std::collections::HashSet<String>) -> bool {
    let mut found = false;
    visit_value(node, &mut |n| {
        if let Node::Var { name, .. } = n {
            found |= names.contains(name.strip_prefix("--").unwrap_or(name));
        }
    });
    found
}

fn substitute_in_items(
    items: &mut [Item],
    candidates: &HashMap<String, Node>,
    substituted: &mut std::collections::HashSet<String>,
) {
    for item in items.iter_mut() {
        match item {
            Item::Decl(d) => substitute_in_node(&mut d.value, candidates, substituted),
            Item::Rule(r) => substitute_in_rule(r, candidates, substituted),
            Item::Verbatim(_) => {}
        }
    }
}

fn substitute_in_rule(
    rule: &mut Rule,
    candidates: &HashMap<String, Node>,
    substituted: &mut std::collections::HashSet<String>,
) {
    if rule.prelude.is_property() {
        return;
    }
    match &mut rule.body {
        Body::Decls(items) => {
            for di in items {
                if let DeclItem::Decl(d) = di {
                    substitute_in_node(&mut d.value, candidates, substituted);
                }
            }
        }
        Body::Nested(items) => substitute_in_items(items, candidates, substituted),
    }
}

fn substitute_in_node(
    node: &mut Node,
    candidates: &HashMap<String, Node>,
    substituted: &mut std::collections::HashSet<String>,
) {
    match node {
        Node::Var { name, fallback } => {
            if let Some(fb) = fallback {
                substitute_in_node(fb, candidates, substituted);
            }
            let stripped = name.strip_prefix("--").unwrap_or(name);
            if let Some(value) = candidates.get(stripped) {
                substituted.insert(stripped.to_string());
                *node = value.clone();
            }
        }
        Node::Calc(inner) | Node::Paren(inner) => {
            substitute_in_node(inner, candidates, substituted)
        }
        Node::MathFn { args, .. } | Node::Fn { args, .. } => {
            for a in args {
                substitute_in_node(a, candidates, substituted);
            }
        }
        Node::Sum(terms) => {
            for t in terms {
                substitute_in_node(&mut t.node, candidates, substituted);
            }
        }
        Node::Product(factors) => {
            for f in factors {
                substitute_in_node(f, candidates, substituted);
            }
        }
        Node::Div(l, r) => {
            substitute_in_node(l, candidates, substituted);
            substitute_in_node(r, candidates, substituted);
        }
        Node::If { arms, default } => {
            for arm in arms {
                substitute_in_node(&mut arm.cond, candidates, substituted);
                substitute_in_node(&mut arm.value, candidates, substituted);
            }
            substitute_in_node(default, candidates, substituted);
        }
        Node::Or(conds) => {
            for c in conds {
                substitute_in_node(c, candidates, substituted);
            }
        }
        Node::List { items, .. } => {
            for item in items {
                substitute_in_node(item, candidates, substituted);
            }
        }
        // A `style()` query compares against its value as written.
        Node::Style { .. } => {}
        Node::Int(_)
        | Node::Raw(_)
        | Node::Invalid
        | Node::Ident(_)
        | Node::Str(_)
        | Node::Dim { .. }
        | Node::Hash(_) => {}
    }
}

fn drop_inlined(items: &mut Vec<Item>, substituted: &std::collections::HashSet<String>) {
    items.retain(|item| {
        if let Item::Decl(d) = item {
            let key = d.name.strip_prefix("--").unwrap_or(&d.name);
            return !substituted.contains(key);
        }
        true
    });
    for item in items.iter_mut() {
        if let Item::Rule(r) = item {
            drop_inlined_in_rule(r, substituted);
        }
    }
}

fn drop_inlined_in_rule(rule: &mut Rule, substituted: &std::collections::HashSet<String>) {
    match &mut rule.body {
        Body::Decls(items) => {
            items.retain(|di| {
                if let DeclItem::Decl(d) = di {
                    let key = d.name.strip_prefix("--").unwrap_or(&d.name);
                    return !substituted.contains(key);
                }
                true
            });
        }
        Body::Nested(items) => drop_inlined(items, substituted),
    }
}

pub fn count_custom_names(doc: &Doc, counts: &mut HashMap<String, usize>) {
    count_custom_in_items(&doc.items, counts);
}

pub fn rename_custom_names(doc: &mut Doc, rename: &HashMap<String, String>) {
    rename_in_items(&mut doc.items, rename);
}

pub fn scrub_verbatim(doc: &mut Doc, transform: &impl Fn(&str) -> String) {
    scrub_in_items(&mut doc.items, transform);
}

fn scrub_in_items(items: &mut [Item], transform: &impl Fn(&str) -> String) {
    for item in items.iter_mut() {
        match item {
            Item::Verbatim(s) => *s = transform(s),
            Item::Decl(_) => {}
            Item::Rule(r) => {
                match &mut r.prelude {
                    Prelude::Selector(s) | Prelude::AtRule { prelude: s, .. } => *s = transform(s),
                    _ => {}
                }
                r.head_pad = transform(&r.head_pad);
                r.open_pad = transform(&r.open_pad);
                match &mut r.body {
                    Body::Decls(items) => {
                        for di in items {
                            if let DeclItem::Verbatim(s) = di {
                                *s = transform(s);
                            }
                        }
                    }
                    Body::Nested(items) => scrub_in_items(items, transform),
                }
                r.close_pad = transform(&r.close_pad);
            }
        }
    }
}

fn count_custom_in_items(items: &[Item], counts: &mut HashMap<String, usize>) {
    for item in items {
        match item {
            Item::Verbatim(s) => scan_custom_in_text(s, counts),
            Item::Decl(d) => {
                bump_custom(counts, &d.name);
                count_custom_in_value(&d.value, counts);
            }
            Item::Rule(r) => count_custom_in_rule(r, counts),
        }
    }
}

fn count_custom_in_rule(r: &Rule, counts: &mut HashMap<String, usize>) {
    match &r.prelude {
        Prelude::Selector(s) | Prelude::AtRule { prelude: s, .. } => scan_custom_in_text(s, counts),
        Prelude::Property(name) | Prelude::Keyframes(name) => bump_custom(counts, name),
        Prelude::Function(f) => {
            bump_custom(counts, &f.name);
            for p in &f.params {
                bump_custom(counts, &p.name);
            }
        }
        Prelude::Container(cond) => count_custom_in_value(cond, counts),
    }
    match &r.body {
        Body::Decls(items) => {
            for di in items {
                match di {
                    DeclItem::Verbatim(s) => scan_custom_in_text(s, counts),
                    DeclItem::Decl(d) => {
                        bump_custom(counts, &d.name);
                        count_custom_in_value(&d.value, counts);
                    }
                }
            }
        }
        Body::Nested(items) => count_custom_in_items(items, counts),
    }
}

fn count_custom_in_value(node: &Node, counts: &mut HashMap<String, usize>) {
    visit_value(node, &mut |n| match n {
        Node::Raw(s) => scan_custom_in_text(s, counts),
        Node::Var { name, .. }
        | Node::Style { prop: name, .. }
        | Node::MathFn { name, .. }
        | Node::Fn { name, .. }
        | Node::Ident(name) => bump_custom(counts, name),
        _ => {}
    });
}

fn bump_custom(counts: &mut HashMap<String, usize>, full_name: &str) {
    let Some(stripped) = full_name.strip_prefix("--") else {
        return;
    };
    if stripped
        .as_bytes()
        .first()
        .is_some_and(|&b| b.is_ascii_alphabetic() || b == b'_')
    {
        *counts.entry(stripped.to_string()).or_insert(0) += 1;
    }
}

fn scan_custom_in_text(s: &str, counts: &mut HashMap<String, usize>) {
    each_custom_token(s, |name| {
        *counts.entry(name.to_string()).or_insert(0) += 1;
    });
}

fn rename_in_items(items: &mut [Item], rename: &HashMap<String, String>) {
    for item in items.iter_mut() {
        match item {
            Item::Verbatim(s) => *s = rewrite_custom_in_text(s, rename),
            Item::Decl(d) => {
                rename_full_name(&mut d.name, rename);
                rename_in_value(&mut d.value, rename);
            }
            Item::Rule(r) => rename_in_rule(r, rename),
        }
    }
}

fn rename_in_rule(r: &mut Rule, rename: &HashMap<String, String>) {
    match &mut r.prelude {
        Prelude::Selector(s) | Prelude::AtRule { prelude: s, .. } => {
            *s = rewrite_custom_in_text(s, rename);
        }
        Prelude::Property(name) | Prelude::Keyframes(name) => rename_full_name(name, rename),
        Prelude::Function(f) => {
            rename_full_name(&mut f.name, rename);
            for p in &mut f.params {
                rename_full_name(&mut p.name, rename);
            }
        }
        Prelude::Container(cond) => rename_in_value(cond, rename),
    }
    match &mut r.body {
        Body::Decls(items) => {
            for di in items {
                match di {
                    DeclItem::Verbatim(s) => *s = rewrite_custom_in_text(s, rename),
                    DeclItem::Decl(d) => {
                        rename_full_name(&mut d.name, rename);
                        rename_in_value(&mut d.value, rename);
                    }
                }
            }
        }
        Body::Nested(items) => rename_in_items(items, rename),
    }
}

fn rename_in_value(node: &mut Node, rename: &HashMap<String, String>) {
    visit_value_mut(node, &mut |n| match n {
        Node::Raw(s) => *s = rewrite_custom_in_text(s, rename),
        Node::Var { name, .. }
        | Node::Style { prop: name, .. }
        | Node::MathFn { name, .. }
        | Node::Fn { name, .. }
        | Node::Ident(name) => rename_full_name(name, rename),
        _ => {}
    });
}

fn rename_full_name(full_name: &mut String, rename: &HashMap<String, String>) {
    if let Some(stripped) = full_name.strip_prefix("--")
        && let Some(new) = rename.get(stripped)
    {
        let mut next = String::with_capacity(2 + new.len());
        next.push_str("--");
        next.push_str(new);
        *full_name = next;
    }
}

fn rewrite_custom_in_text(s: &str, rename: &HashMap<String, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            let end = super::parse::skip_css_string(bytes, i);
            out.push_str(&s[i..end]);
            i = end;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            let start = i;
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            out.push_str(&s[start..i]);
            continue;
        }
        if b == b'-'
            && i + 2 < bytes.len()
            && bytes[i + 1] == b'-'
            && is_custom_name_start(bytes[i + 2])
        {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && is_custom_name_cont(bytes[j]) {
                j += 1;
            }
            if let Some(new) = rename.get(&s[start..j]) {
                out.push_str("--");
                out.push_str(new);
            } else {
                out.push_str(&s[i..j]);
            }
            i = j;
            continue;
        }
        out.push(b as char);
        i += 1;
    }
    out
}

fn each_custom_token(s: &str, mut visit: impl FnMut(&str)) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            i = super::parse::skip_css_string(bytes, i);
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if b == b'-'
            && i + 2 < bytes.len()
            && bytes[i + 1] == b'-'
            && is_custom_name_start(bytes[i + 2])
        {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && is_custom_name_cont(bytes[j]) {
                j += 1;
            }
            visit(&s[start..j]);
            i = j;
            continue;
        }
        i += 1;
    }
}

pub(crate) fn is_custom_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

pub(crate) fn is_custom_name_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn scan_text_for(
    buf: &str,
    prefix: &[u8],
    end_a: u8,
    end_b: u8,
    counts: &mut HashMap<String, usize>,
) {
    let bytes = buf.as_bytes();
    let plen = prefix.len();
    let mut i = 0;
    while i + plen < bytes.len() {
        if &bytes[i..i + plen] == prefix {
            let start = i + plen;
            let mut j = start;
            while j < bytes.len() {
                let b = bytes[j];
                if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' {
                    j += 1;
                } else {
                    break;
                }
            }
            if j > start && bytes.get(j).is_some_and(|&b| b == end_a || b == end_b) {
                let name = &buf[start..j];
                if let Some(stripped) = name.strip_prefix("--") {
                    *counts.entry(stripped.to_string()).or_insert(0) += 1;
                }
            }
            i = j;
            continue;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_simple_selector_with_decls() {
        let doc = parse_doc(".foo { --a: 1; --b: 2; }");
        assert_eq!(doc.items.len(), 1);
        let Item::Rule(r) = &doc.items[0] else {
            panic!("expected rule");
        };
        assert_eq!(r.prelude, Prelude::Selector(".foo".to_string()));
        assert_eq!(r.head_pad, " ");
        let Body::Decls(items) = &r.body else {
            panic!("expected decl body");
        };
        // Verbatim ws + decl + ws + decl + ws.
        let decls: Vec<_> = items
            .iter()
            .filter_map(|d| {
                if let DeclItem::Decl(d) = d {
                    Some(d)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].name, "--a");
        assert_eq!(decls[1].name, "--b");
    }

    #[test]
    fn roundtrips_simple_selector() {
        let input = ".foo { --a: 1; --b: 2; }";
        let doc = parse_doc(input);
        assert_eq!(print_doc(&doc), input);
    }

    #[test]
    fn roundtrips_at_property() {
        let input = "@property --x { syntax: \"<integer>\"; initial-value: 0; inherits: true; }";
        let doc = parse_doc(input);
        assert_eq!(print_doc(&doc), input);
    }

    #[test]
    fn roundtrips_at_function() {
        let input =
            "@function --eqz(--a <number>) returns <integer> { result: --eq(var(--a), 0); }";
        let doc = parse_doc(input);
        assert_eq!(print_doc(&doc), input);
    }

    #[test]
    fn roundtrips_at_keyframes() {
        let input = "@keyframes anim { 0%, 100% { --x: 0; } 50% { --x: 1; } }";
        let doc = parse_doc(input);
        assert_eq!(print_doc(&doc), input);
    }

    #[test]
    fn roundtrips_at_container() {
        let input = "@container style(--c: 1) { .foo { --x: 1; } }";
        let doc = parse_doc(input);
        assert_eq!(print_doc(&doc), input);
    }

    #[test]
    fn preserves_inter_rule_whitespace() {
        let input = ".a { --x: 1; }\n.b { --y: 2; }\n";
        let doc = parse_doc(input);
        assert_eq!(print_doc(&doc), input);
    }

    #[test]
    fn value_parses_through_to_css_node() {
        let doc = parse_doc(".foo { --x: calc(var(--a) + 1); }");
        let Item::Rule(r) = &doc.items[0] else {
            unreachable!()
        };
        let Body::Decls(items) = &r.body else {
            unreachable!()
        };
        let DeclItem::Decl(d) = items
            .iter()
            .find(|d| matches!(d, DeclItem::Decl(_)))
            .unwrap()
        else {
            unreachable!()
        };
        // `calc(var(--a) + 1)` should parse as a Calc-wrapped Sum.
        assert!(matches!(d.value, Node::Calc(_)));
    }

    #[test]
    fn unrecognized_decl_value_falls_through_as_raw() {
        // `#fff` isn't in our value grammar; should land as Raw and
        // print verbatim.
        let input = ".foo { color: #fff; }";
        let doc = parse_doc(input);
        assert_eq!(print_doc(&doc), input);
    }

    #[test]
    fn roundtrips_realistic_emit_fragment() {
        // A small but structurally representative slice of what wss
        // emits: property registrations, an @function, an @keyframes,
        // a selector with mixed value shapes.
        let input = "@property --pc { syntax: \"<integer>\"; initial-value: 0; inherits: true; }\n\
                     @function --eqz(--a <number>) returns <integer> { result: --eq(var(--a), 0); }\n\
                     @keyframes store { 0%, 100% { --_2pc: var(--_0pc); --_2r0: var(--_0r0); } }\n\
                     .clk { --pc: 0; --r0: var(--_1r0); --cs_sp: if(style(--pcg0: 1): calc(var(--_1cs_sp) + 1); else: var(--_1cs_sp)); }";
        let doc = parse_doc(input);
        let printed = print_doc(&doc);
        assert_eq!(printed, input, "roundtrip diverged");
    }

    #[test]
    fn roundtrips_function_with_angle_bracket_signature() {
        let input = "@function --csmerge(--idx <number>, --prev <number>) returns <integer> { result: --m16(calc(var(--a) + 1), calc(var(--b) + 2)); }";
        let doc = parse_doc(input);
        let printed = print_doc(&doc);
        assert_eq!(printed, input, "roundtrip diverged");
    }

    #[test]
    fn parse_print_keeps_style_cond_value() {
        let input = ".clk { --cs000: if(style(--cswdp0: 1): --csmerge(0, var(--_1cs000)); else: var(--_1cs000)); }";
        let doc = parse_doc(input);
        let printed = print_doc(&doc);
        assert_eq!(printed, input, "parse+print should be identity");
    }

    #[test]
    fn fold_preserves_style_cond_value() {
        use super::super::fold;
        let value = "if(style(--cswdp0: 1): --csmerge(0, var(--_1cs000)); else: var(--_1cs000))";
        let node = super::super::parse(value);
        let folded = fold(node);
        let printed = folded.to_css();
        assert_eq!(printed, value, "fold should be identity for this expr");
    }

    #[test]
    fn count_refs_sees_fallback_in_multi_decl_line() {
        // The shadow-stage line `--_1g0_0: var(--_2g0_0, N); --_1g0_1: var(--_2g0_1, M); ...`
        // is the one that feeds the doc parser the lookup chain. Each
        // Var's fallback Int(N) must NOT prevent counting `_2g0_*` as
        // referenced.
        let input = ".clk { --_1g0_0: var(--_2g0_0, 68); --_1g0_1: var(--_2g0_1, 51); }";
        let doc = parse_doc(input);
        let counts = count_refs(&doc);
        assert_eq!(
            counts.get("_2g0_0").copied(),
            Some(1),
            "counts: {:?}",
            counts
        );
        assert_eq!(counts.get("_2g0_1").copied(), Some(1));
    }

    #[test]
    fn inline_single_use_substitutes_and_drops() {
        let input = ".clk { --csv6: if(style(--_1pc: 2102): var(--_1r14); else: 0); --consumer: --eq(var(--csv6), 7); }";
        let mut doc = parse_doc(input);
        let inlined = inline_single_use(&mut doc, &HashMap::new());
        assert_eq!(inlined, 1, "csv6 should be inlined");
        let printed = print_doc(&doc);
        // After inline, csv6 is gone and the consumer holds the value directly.
        assert!(
            !printed.contains("--csv6:"),
            "decl not removed: {}",
            printed
        );
        assert!(
            printed
                .contains("--consumer: --eq(if(style(--_1pc: 2102): var(--_1r14); else: 0), 7);"),
            "substitution missing: {}",
            printed
        );
    }

    #[test]
    fn inline_single_use_skips_var_aliases() {
        // `--r7: var(--_1r7);` is a var-alias; should NOT inline even
        // though it has refcount 1.
        let input = ".clk { --r7: var(--_1r7); --consumer: var(--r7); }";
        let mut doc = parse_doc(input);
        let inlined = inline_single_use(&mut doc, &HashMap::new());
        assert_eq!(inlined, 0, "var-alias must be preserved");
        let printed = print_doc(&doc);
        assert!(printed.contains("--r7: var(--_1r7);"));
    }

    #[test]
    fn inline_single_use_skips_keyframe_decls() {
        let input = "@keyframes execute { 0%,100% { --_0x: calc(var(--y) + 1); } } @keyframes store { 0%,100% { --_2x: var(--_0x); } }";
        let mut doc = parse_doc(input);
        let inlined = inline_single_use(&mut doc, &HashMap::new());
        assert_eq!(inlined, 0, "{}", print_doc(&doc));
    }

    #[test]
    fn inline_single_use_skips_style_referenced_names() {
        // `--csi0` is referenced via style(--csi0: 1), not var(...).
        // Inlining isn't valid: we'd have to splice the value into
        // the style() predicate, which is a different operation.
        let input = ".clk { --csi0: if(style(--p: 1): 1; else: 0); --consumer: if(style(--csi0: 1): 5; else: 9); }";
        let mut doc = parse_doc(input);
        let inlined = inline_single_use(&mut doc, &HashMap::new());
        assert_eq!(inlined, 0);
    }

    #[test]
    fn inline_single_use_respects_external_seed() {
        let input = ".clk { --x: if(style(--p: 1): 1; else: 0); --y: --eq(var(--x), 0); }";
        let mut doc = parse_doc(input);
        let mut external = HashMap::new();
        external.insert("x".to_string(), 1);
        let inlined = inline_single_use(&mut doc, &external);
        assert_eq!(inlined, 0, "externally-referenced decl must survive");
    }

    #[test]
    fn dce_keeps_decls_referenced_from_other_buffer_via_seed() {
        // Two rules: one declares `--_2g0_0` inside a keyframe stage,
        // the other reads it via `var(--_2g0_0, 0)`. DCE should keep
        // BOTH when the reader is counted in the seed refs.
        let input_a = "@keyframes store { 0%, 100% { --_2g0_0: var(--_0g0_0); } }";
        let input_b = ".clk { --_1g0_0: var(--_2g0_0, 0); }";
        let mut doc_a = parse_doc(input_a);
        let doc_b = parse_doc(input_b);
        let mut refs = count_refs(&doc_a);
        for (k, v) in count_refs(&doc_b) {
            *refs.entry(k).or_insert(0) += v;
        }
        // Seed --_1g0_0 as externally referenced (mimics base.html).
        *refs.entry("_1g0_0".to_string()).or_insert(0) += 1;
        *refs.entry("_0g0_0".to_string()).or_insert(0) += 1;
        let dropped = dce(&mut doc_a, &refs);
        assert_eq!(dropped, 0, "--_2g0_0 should be kept");
        let printed = print_doc(&doc_a);
        assert!(printed.contains("--_2g0_0:"), "decl missing: {}", printed);
    }

    #[test]
    fn roundtrips_keyframes_with_multi_decl_lines() {
        // The real @keyframes store body has many `--name: var(...)`
        // decls separated by `;` on the SAME line, often multiple
        // lines like that, all inside `0%, 100% { ... }`. Parser must
        // preserve them through doc → fold → doc roundtrip.
        let input = "@keyframes store {\n  0%, 100% {\n    --_2pc: var(--_0pc);\n    --_2r0: var(--_0r0); --_2r1: var(--_0r1); --_2r2: var(--_0r2);\n    --_2g0_0: var(--_0g0_0); --_2g0_1: var(--_0g0_1); --_2g0_2: var(--_0g0_2); --_2g0_3: var(--_0g0_3);\n  }\n}";
        let doc = parse_doc(input);
        let printed = print_doc(&doc);
        assert_eq!(printed, input, "roundtrip diverged");
    }

    #[test]
    fn roundtrips_function_with_nested_if_semicolons() {
        // The actual `--csmerge` body has `if(arms; else: V)` patterns
        // with `;` at paren depth > 0 — these must not terminate the
        // outer `result: <expr>;` decl.
        let input = "@function --csmerge(--idx <number>, --prev <number>) returns <integer> { result: --m16(calc(if(style(--pcg0: 1): 0; else: 1)), calc(if(style(--pcg0: 1): 1; else: 0))); }";
        let doc = parse_doc(input);
        let printed = print_doc(&doc);
        assert_eq!(printed, input, "roundtrip diverged");
    }

    #[test]
    fn property_value_simplifies_through_to_css_fold() {
        use super::super::fold;
        let input = ".foo { --x: calc(var(--a) + 0); }";
        let mut doc = parse_doc(input);
        // Pull out the decl, run fold on its value, write back.
        if let Item::Rule(r) = &mut doc.items[0]
            && let Body::Decls(items) = &mut r.body
        {
            for di in items {
                if let DeclItem::Decl(d) = di {
                    d.value = fold(d.value.clone());
                }
            }
        }
        let printed = print_doc(&doc);
        assert!(printed.contains("--x: var(--a);"));
    }

    #[test]
    fn roundtrips_comments_in_various_positions() {
        for input in [
            ".a { /* hi */ --x: 1; }",
            "/* top */ .a { --x: 1; }",
            ".a { --x: 1; /* between */ --y: 2; }",
        ] {
            let doc = parse_doc(input);
            let printed = print_doc(&doc);
            assert_eq!(printed, input, "comment-bearing fragment lost on roundtrip");
        }
    }

    fn fold_buffer(input: &str) -> String {
        let mut doc = parse_doc(input);
        fold_doc(&mut doc);
        print_doc(&doc)
    }

    #[test]
    fn fold_doc_preserves_non_value_text() {
        let input = ".foo { color: red; }\n";
        assert_eq!(fold_buffer(input), input);
    }

    #[test]
    fn fold_doc_simplifies_decl_at_top_level() {
        let input = " --r12: calc(var(--x) + 0);\n";
        // Top-level decl outside a rule still folds.
        let out = fold_buffer(input);
        assert!(out.contains("--r12: var(--x);"), "got: {out}");
    }

    #[test]
    fn fold_doc_handles_if_value() {
        let input = ".clk { --pc: if(style(--p: 1): --eq1(1); else: 0); }";
        let out = fold_buffer(input);
        assert!(
            out.contains("--pc: if(style(--p: 1): 1; else: 0);"),
            "got: {out}"
        );
    }

    #[test]
    fn fold_doc_preserves_pseudo_selector_layout() {
        let input = ".btn:hover { color: red; }\n";
        assert_eq!(fold_buffer(input), input);
    }

    #[test]
    fn fold_doc_folds_inside_function_body() {
        let input = "@function --csmerge() { result: calc(var(--x) * 0 + var(--y)); }";
        let out = fold_buffer(input);
        assert!(out.contains("result: var(--y);"), "got: {out}");
    }

    #[test]
    fn fold_doc_passes_through_unparseable_value() {
        let input = "  color: #ff0000;\n";
        assert_eq!(fold_buffer(input), input);
    }

    #[test]
    fn fold_doc_folds_compare_calls() {
        let input = ".x { --x: --eq(2, 2); }";
        let out = fold_buffer(input);
        assert!(out.contains("--x: 1;"), "got: {out}");
    }

    fn rule_at(doc: &Doc, i: usize) -> &Rule {
        let Item::Rule(r) = &doc.items[i] else {
            panic!("expected a rule at {i}: {:?}", doc.items);
        };
        r
    }

    #[test]
    fn parses_preludes() {
        let doc = parse_doc(
            "@property --x { syntax: \"<integer>\"; }\n\
             @function --sel(--c <integer>, --t) returns <number> { result: 0; }\n\
             @keyframes store { 0%, 100% { --a: 1; } }\n\
             @container style(--clk: 3) { .m { --b: 1; } }\n\
             @media (max-width: 720px) { .m { --b: 1; } }",
        );
        assert_eq!(rule_at(&doc, 0).prelude, Prelude::Property("--x".into()));
        assert_eq!(
            rule_at(&doc, 2).prelude,
            Prelude::Function(FunctionHead {
                name: "--sel".into(),
                params: vec![
                    Param {
                        name: "--c".into(),
                        syntax: Some("<integer>".into()),
                    },
                    Param {
                        name: "--t".into(),
                        syntax: None,
                    },
                ],
                returns: Some("<number>".into()),
            })
        );
        assert_eq!(rule_at(&doc, 4).prelude, Prelude::Keyframes("store".into()));
        let Prelude::Container(Node::Style { prop, .. }) = &rule_at(&doc, 6).prelude else {
            panic!("expected a style() container query");
        };
        assert_eq!(prop, "--clk");
        assert!(matches!(
            &rule_at(&doc, 8).prelude,
            Prelude::AtRule { name, prelude } if name == "media" && prelude == "(max-width: 720px)"
        ));
    }

    #[test]
    fn parses_final_decl_without_semicolon() {
        let doc = parse_doc("@keyframes r { 0% { --rng0: 0 } }");
        let Body::Nested(stages) = &rule_at(&doc, 0).body else {
            panic!("expected keyframe stages");
        };
        let Item::Rule(stage) = &stages[0] else {
            panic!("expected a stage");
        };
        let Body::Decls(items) = &stage.body else {
            panic!("expected decls");
        };
        assert!(matches!(&items[..], [DeclItem::Decl(d)] if d.name == "--rng0"));
        assert_eq!(print_doc(&doc), "@keyframes r { 0% { --rng0: 0; } }");
    }

    #[test]
    fn parses_important_and_comments() {
        let input = ".a { /* hi */ animation-play-state: paused, paused !important; }";
        let doc = parse_doc(input);
        let Body::Decls(items) = &rule_at(&doc, 0).body else {
            panic!("expected decls");
        };
        assert!(matches!(&items[0], DeclItem::Verbatim(c) if c == "/* hi */ "));
        assert!(matches!(&items[1], DeclItem::Decl(d) if d.important));
        assert_eq!(print_doc(&doc), input);
    }

    #[test]
    fn braces_in_strings_do_not_split_rules() {
        let input = "@function --c(--i <integer>) returns <string> { result: if(style(--i: 1): \"{\"; else: \"}\"); }";
        let doc = parse_doc(input);
        let Body::Decls(items) = &rule_at(&doc, 0).body else {
            panic!("expected decls");
        };
        assert!(matches!(&items[..], [DeclItem::Decl(d)] if d.name == "result"));
        assert_eq!(print_doc(&doc), input);
    }

    #[test]
    fn inline_single_use_skips_standard_properties() {
        // The emitter chunks long `box-shadow` lists into custom properties.
        let input = ".m { --mv-0: 8px 8px red; box-shadow: var(--mv-0), 0 0 transparent; }";
        let mut doc = parse_doc(input);
        assert_eq!(inline_single_use(&mut doc, &HashMap::new()), 0);
        assert_eq!(count_refs(&doc).get("mv-0"), Some(&1));
    }
}
