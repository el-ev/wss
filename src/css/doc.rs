//! Document-level CSS AST. Wraps the value AST in `super::Node` with
//! the surrounding rule structure — selectors, `@function`,
//! `@property`, `@keyframes`, `@container` — so document-level
//! transforms (DCE, single-use inlining, slot CSE) can operate on
//! typed structure instead of byte-walking.

use super::Node;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Doc {
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// Whitespace, comments, HTML — anything outside a rule body.
    Verbatim(String),
    /// A CSS rule with a head and a brace body.
    Rule(Rule),
    /// A `--name: <value>;` declaration that lives at the top level
    /// of a buffer (e.g. the bare decls wss emits into the
    /// `.terminal { /*__WSS_LOGIC__*/ }` placeholder in base.html).
    /// Treated by DCE the same way as a decl inside a rule body.
    Decl(Decl),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    /// Everything between the previous item and the opening `{`,
    /// including the selector or at-rule prelude and any leading
    /// whitespace. Stored verbatim so the printer can reproduce
    /// exact layout.
    pub head: String,
    /// Text immediately after `{` and before the body content (often
    /// `" "` or `"\n  "`). Stored to preserve spacing.
    pub open_pad: String,
    pub body: Body,
    /// Text immediately before the closing `}` (often `" "` or `"\n"`).
    pub close_pad: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    /// `name: value;` declarations interleaved with whitespace and
    /// other verbatim text. Used for plain selectors, `@property`
    /// bodies, and the `result: <expr>;` body of an `@function`.
    Decls(Vec<DeclItem>),
    /// Nested CSS rules — `@keyframes` stages, `@container` rules.
    Nested(Vec<Item>),
    /// Anything the parser couldn't classify; preserved verbatim.
    Verbatim(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeclItem {
    /// Whitespace and other inter-decl text.
    Verbatim(String),
    /// One `--name: <value>;` (or `name: <value>;` for non-custom
    /// CSS properties; we still parse those into a `Decl` and let
    /// the value fall through as `Node::Raw` if it doesn't fit our
    /// grammar).
    Decl(Decl),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decl {
    /// Property name, including the leading `--` for custom
    /// properties (e.g. `"--cri0"`) or the bare CSS property name
    /// (e.g. `"display"`).
    pub name: String,
    /// Value parsed via `super::parse`. Failed parses become
    /// `Node::Raw` so the printer can reproduce them verbatim.
    pub value: Node,
}

// =====================================================================
// Parser
// =====================================================================

/// Parse a CSS document buffer into a `Doc`. The parser is forgiving:
/// anything it can't classify falls into a `Verbatim` variant and
/// passes through unchanged on print.
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

    /// Parse items until end of input. Top-level entry point.
    fn parse_items_to_end(&mut self) -> Vec<Item> {
        self.parse_items_until(None)
    }

    /// Parse items until we hit `stop_at` (a closing brace) or end of
    /// input. Used both for the top-level document and for nested
    /// rule bodies (`@keyframes`, `@container`).
    fn parse_items_until(&mut self, stop_at: Option<u8>) -> Vec<Item> {
        let mut items: Vec<Item> = Vec::new();
        let mut verbatim_start = self.pos;
        while self.pos < self.bytes.len() {
            let b = self.bytes[self.pos];
            if stop_at == Some(b) {
                break;
            }
            // Skip over a quoted string literal so it doesn't confuse
            // brace-matching.
            if b == b'"' || b == b'\'' {
                if let Some(end) = self.skip_string(self.pos) {
                    self.pos = end;
                    continue;
                }
                self.pos += 1;
                continue;
            }
            if b == b'{' {
                // Walk backwards to find the head start: it begins right
                // after the previous top-level `;`, `}`, or the
                // verbatim-start marker.
                let head_start = self.find_head_start(verbatim_start);
                let head = self.input[head_start..self.pos].to_string();
                let pre_text = &self.input[verbatim_start..head_start];
                push_text_as_items(pre_text, &mut items);
                self.pos += 1; // consume `{`
                let (open_pad, body, close_pad) = self.parse_rule_body(&head);
                items.push(Item::Rule(Rule {
                    head,
                    open_pad,
                    body,
                    close_pad,
                }));
                verbatim_start = self.pos;
                continue;
            }
            self.pos += 1;
        }
        let tail = &self.input[verbatim_start..self.pos];
        push_text_as_items(tail, &mut items);
        items
    }

    /// Given that `verbatim_start..self.pos` is the unclaimed slice and
    /// `self.pos` points at `{`, find where the rule head begins.
    /// The head extends from the rightmost preceding `;` or `}` (or
    /// `verbatim_start` if none) up to `self.pos`. We do NOT treat `>`
    /// as a terminator because it appears inside CSS type tokens like
    /// `<integer>` in `@function ... returns <integer>` heads; the
    /// generated CSS doesn't use the `a > b` child combinator.
    fn find_head_start(&self, verbatim_start: usize) -> usize {
        let mut i = self.pos;
        while i > verbatim_start {
            let b = self.bytes[i - 1];
            if b == b';' || b == b'}' {
                return i;
            }
            i -= 1;
        }
        verbatim_start
    }

    /// Skip past a string literal starting at `self.bytes[start] == quote`.
    /// Returns the position just past the closing quote, or `None` if
    /// the string isn't terminated.
    fn skip_string(&self, start: usize) -> Option<usize> {
        let quote = self.bytes[start];
        let mut i = start + 1;
        while i < self.bytes.len() {
            match self.bytes[i] {
                b'\\' => i += 2,
                c if c == quote => return Some(i + 1),
                _ => i += 1,
            }
        }
        None
    }

    /// Find the byte position of the matching `}` for the brace at
    /// `self.pos - 1` (already consumed), keeping `self.pos` updated
    /// to point just after the `}`.
    fn parse_rule_body(&mut self, head: &str) -> (String, Body, String) {
        // Find matching `}` while respecting nested braces, parens,
        // and string literals.
        let body_start = self.pos;
        let mut depth: i32 = 1;
        let mut i = self.pos;
        while i < self.bytes.len() && depth > 0 {
            match self.bytes[i] {
                b'"' | b'\'' => {
                    if let Some(end) = self.skip_string(i) {
                        i = end;
                        continue;
                    }
                    i += 1;
                }
                b'{' => {
                    depth += 1;
                    i += 1;
                }
                b'}' => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                    i += 1;
                }
                _ => i += 1,
            }
        }
        let body_end = i;
        // Skip past the closing `}` (if present).
        self.pos = body_end + if body_end < self.bytes.len() { 1 } else { 0 };

        let body_text = &self.input[body_start..body_end];
        let (open_pad, inner, close_pad) = trim_pads(body_text);
        let body = classify_body(head, inner);
        (open_pad.to_string(), body, close_pad.to_string())
    }
}

/// Extend `out` with a `Verbatim`, a `Decl`, or some combination based on
/// what `text` contains. If `text` has no `{` or `}` characters, we try
/// to parse it as a sequence of `--name: <value>;` declarations
/// interleaved with whitespace — this is how the bare decls wss emits
/// into the `.terminal { /*__WSS_LOGIC__*/ }` placeholder show up at
/// the top level of `logic_css`. Anything we can't classify falls back
/// to a single `Verbatim` chunk.
fn push_text_as_items(text: &str, out: &mut Vec<Item>) {
    if text.is_empty() {
        return;
    }
    if text.bytes().any(|b| b == b'{' || b == b'}') {
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

fn trim_pads(s: &str) -> (&str, &str, &str) {
    let leading_end = s
        .bytes()
        .position(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        .unwrap_or(s.len());
    let trailing_start = s
        .bytes()
        .rposition(|b| !matches!(b, b' ' | b'\t' | b'\n' | b'\r'))
        .map_or(s.len(), |i| i + 1);
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

fn classify_body(head: &str, inner: &str) -> Body {
    let trimmed_head = head.trim();
    // `@keyframes` and `@container` bodies contain nested rules.
    if trimmed_head.starts_with("@keyframes") || trimmed_head.starts_with("@container") {
        let mut p = DocParser::new(inner);
        return Body::Nested(p.parse_items_to_end());
    }
    // Try to parse as a decl list. If we see a `{` anywhere at depth 0
    // we can't be in a decl-only body; fall back to Verbatim.
    if contains_top_level_brace(inner) {
        return Body::Verbatim(inner.to_string());
    }
    Body::Decls(parse_decl_items(inner))
}

fn contains_top_level_brace(s: &str) -> bool {
    let mut depth: i32 = 0;
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'(' => depth += 1,
            b')' => depth -= 1,
            b'{' if depth == 0 => return true,
            _ => {}
        }
        i += 1;
    }
    false
}

/// Split `inner` into a sequence of `DeclItem`s: declarations
/// separated by verbatim whitespace / semicolons. Each declaration
/// reads up to (and including) the next top-level `;`.
fn parse_decl_items(inner: &str) -> Vec<DeclItem> {
    let bytes = inner.as_bytes();
    let mut items: Vec<DeclItem> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        // Eat leading whitespace as verbatim.
        let ws_start = i;
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t' | b'\n' | b'\r') {
            i += 1;
        }
        if i > ws_start {
            items.push(DeclItem::Verbatim(inner[ws_start..i].to_string()));
        }
        if i >= bytes.len() {
            break;
        }
        // Now we're at the start of a potential decl. Find the next
        // top-level `;` (or end of buffer).
        let decl_start = i;
        let mut depth: i32 = 0;
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
                b';' if depth == 0 => {
                    i += 1; // include the `;`
                    break;
                }
                b'"' | b'\'' => {
                    let quote = bytes[i];
                    i += 1;
                    while i < bytes.len() {
                        let b = bytes[i];
                        if b == b'\\' {
                            i = (i + 2).min(bytes.len());
                            continue;
                        }
                        i += 1;
                        if b == quote {
                            break;
                        }
                    }
                }
                _ => i += 1,
            }
        }
        let decl_text = &inner[decl_start..i];
        match parse_decl(decl_text) {
            Some(decl) => items.push(DeclItem::Decl(decl)),
            None => items.push(DeclItem::Verbatim(decl_text.to_string())),
        }
    }
    items
}

/// Parse a single `name: <value>;` slice. Returns `None` if the slice
/// doesn't fit the shape.
fn parse_decl(slice: &str) -> Option<Decl> {
    let trimmed = slice.trim_end_matches(|c: char| c.is_ascii_whitespace());
    let trimmed = trimmed.strip_suffix(';')?;
    let colon = find_decl_colon(trimmed)?;
    let name = trimmed[..colon].trim();
    if name.is_empty() {
        return None;
    }
    // Property names: ASCII letters, digits, `-`, `_`. Custom
    // properties start with `--`.
    if !name
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return None;
    }
    let value_str = trimmed[colon + 1..].trim();
    let value = super::parse(value_str);
    Some(Decl {
        name: name.to_string(),
        value,
    })
}

/// Find the first `:` at paren depth 0 that separates a property name
/// from its value. Returns `None` if not found (or only `::` for
/// pseudo-elements is present).
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

fn print_items(items: &[Item], out: &mut String) {
    for item in items {
        match item {
            Item::Verbatim(s) => out.push_str(s),
            Item::Rule(r) => print_rule(r, out),
            Item::Decl(d) => {
                out.push_str(&d.name);
                out.push_str(": ");
                out.push_str(&d.value.to_css());
                out.push(';');
            }
        }
    }
}

fn print_rule(r: &Rule, out: &mut String) {
    out.push_str(&r.head);
    out.push('{');
    out.push_str(&r.open_pad);
    match &r.body {
        Body::Decls(items) => {
            for di in items {
                match di {
                    DeclItem::Verbatim(s) => out.push_str(s),
                    DeclItem::Decl(d) => {
                        out.push_str(&d.name);
                        out.push_str(": ");
                        out.push_str(&d.value.to_css());
                        out.push(';');
                    }
                }
            }
        }
        Body::Nested(items) => print_items(items, out),
        Body::Verbatim(s) => out.push_str(s),
    }
    out.push_str(&r.close_pad);
    out.push('}');
}

// =====================================================================
// Reference counting + DCE
// =====================================================================

use std::collections::HashMap;

/// Count `var(--name)` and `style(--name: ...)` references across every
/// value node in the document, plus every nested rule body.
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
            // Belt-and-braces: also scan stray verbatim text for refs.
            // Anything we recognized as a decl is already classified
            // above; this catches `var(--name)` and `style(--name:
            // ...)` patterns hidden in HTML strings or unparseable CSS
            // we couldn't classify further.
            Item::Verbatim(s) => count_refs_in_text(s, counts),
        }
    }
}

fn walk_rule(rule: &Rule, counts: &mut HashMap<String, usize>) {
    match &rule.body {
        Body::Decls(items) => {
            for di in items {
                if let DeclItem::Decl(d) = di {
                    walk_value(&d.value, counts);
                }
            }
        }
        Body::Nested(items) => walk_items(items, counts),
        // Mixed content (a selector body that contains both `--name:
        // value;` decls AND nested rules like `@container` blocks)
        // currently falls back to Verbatim. Scan its text for refs so
        // we don't miss `var(--name)` and `style(--name: ...)`
        // patterns hiding inside.
        Body::Verbatim(s) => count_refs_in_text(s, counts),
    }
}

/// Walk a value Node and bump `counts[name]` for each Var or Style
/// reference encountered.
fn walk_value(node: &Node, counts: &mut HashMap<String, usize>) {
    match node {
        Node::Int(_) => {}
        // Values our grammar didn't recognize (`box-shadow` lists,
        // hex colors, `Npx` lengths, etc.) land here as raw strings.
        // Scan their text for `var(--name)` and `style(--name: ...)`
        // patterns so cross-references inside `box-shadow:
        // var(--mv-0), var(--mv-1), ...` still register.
        Node::Raw(s) => count_refs_in_text(s, counts),
        Node::Var { name, fallback } => {
            bump_ref(counts, name);
            if let Some(fb) = fallback {
                walk_value(fb, counts);
            }
        }
        Node::Style { prop, .. } => {
            bump_ref(counts, prop);
        }
        Node::Calc(inner) | Node::Paren(inner) => walk_value(inner, counts),
        Node::MathFn { args, .. } | Node::Fn { args, .. } => {
            for a in args {
                walk_value(a, counts);
            }
        }
        Node::Sum(terms) => {
            for t in terms {
                walk_value(&t.node, counts);
            }
        }
        Node::Product(factors) => {
            for f in factors {
                walk_value(f, counts);
            }
        }
        Node::Div(l, r) => {
            walk_value(l, counts);
            walk_value(r, counts);
        }
        Node::If { arms, default } => {
            for arm in arms {
                walk_value(&arm.cond, counts);
                walk_value(&arm.value, counts);
            }
            walk_value(default, counts);
        }
        Node::Or(conds) => {
            for c in conds {
                walk_value(c, counts);
            }
        }
    }
}

fn bump_ref(counts: &mut HashMap<String, usize>, full_name: &str) {
    let stripped = full_name.strip_prefix("--").unwrap_or(full_name);
    *counts.entry(stripped.to_string()).or_insert(0) += 1;
}

/// Walk the AST and drop every `Decl` whose name has zero readers in
/// `refs`. Iterates internally because dropping one decl can leave the
/// names it referenced with no other readers, making those decls dead
/// too. Returns the count of decls removed.
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
    // First, drop top-level `Item::Decl` entries whose name has zero
    // readers. Then recurse into Rule bodies. We can't use a single
    // `retain` here because the count needs to be returned for the
    // fixpoint driver in `dce`.
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
    // Don't DCE inside `@function` (the `result: ...` decl IS the
    // function body — without it the function call returns nothing)
    // or `@property` (the `syntax`/`initial-value`/`inherits` decls
    // define the property registration, not cascade values).
    let head_trim = rule.head.trim_start();
    let in_function = head_trim.starts_with("@function");
    let in_property = head_trim.starts_with("@property");
    match &mut rule.body {
        Body::Decls(items) if !in_function && !in_property => {
            items.retain(|di| {
                let DeclItem::Decl(d) = di else {
                    return true;
                };
                // Only DCE custom properties; bare CSS properties
                // (`display`, `content`, etc.) are kept regardless of
                // whether anything reads them.
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

/// Scan `s` for `var(--name)` and `style(--name: ...)` references and
/// merge them into `counts`. Used to seed the DCE counter with
/// references that live OUTSIDE the parsed document (e.g. in the
/// static `base.html` template, which isn't part of the generated
/// buffers we feed to `parse_doc`).
pub fn count_refs_in_text(s: &str, counts: &mut HashMap<String, usize>) {
    scan_text_for(s, b"var(", b',', b')', counts);
    scan_text_for(s, b"style(", b':', b':', counts);
}

// =====================================================================
// Single-use inlining
// =====================================================================

/// Inline custom-property declarations whose name has exactly one
/// `var(--name)` reader across the doc + seed. The decl's value
/// replaces the `Var` node at the single use site, then the decl
/// itself is dropped.
///
/// Skipped:
///  - Decls whose value references another inline candidate. Doing
///    those without dependency analysis risks cycles or producing
///    expanded shapes that re-trigger ref counting. They'll be picked
///    up on subsequent rounds (caller iterates until fixpoint).
///  - Decls referenced through `style(--name: …)` container queries
///    or through `Node::Raw` slices (we can't AST-substitute into
///    text we don't model).
///  - Decls in `@function` / `@property` bodies (those aren't cascade
///    values).
///  - Seeded names — they're externally referenced (`base.html`, JS).
///
/// Returns the number of decls inlined.
pub fn inline_single_use(doc: &mut Doc, seed_refs: &HashMap<String, usize>) -> usize {
    let var_refs = count_var_refs_only(doc);
    let mut total_refs = var_refs.clone();
    for (k, v) in count_text_refs_only(doc) {
        *total_refs.entry(k).or_insert(0) += v;
    }
    for (k, v) in seed_refs {
        *total_refs.entry(k.clone()).or_insert(0) += v;
    }

    // Gather candidate decls: name → value, where total refs == 1 and
    // the lone ref is a Var (not a Style or Raw text mention).
    let mut candidates: HashMap<String, Node> = HashMap::new();
    collect_inline_candidates(&doc.items, &total_refs, &var_refs, &mut candidates);
    if candidates.is_empty() {
        return 0;
    }

    // Remove candidates whose value references another candidate —
    // safer to skip cycles and handle those next round.
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

/// Like [`count_refs`] but only counts `Var` references — not the
/// `style(--name: ...)` container queries. Used by inlining to make
/// sure the lone ref is a `Var` we can structurally substitute, not
/// a `Style` predicate or a `Raw`-text mention.
fn count_var_refs_only(doc: &Doc) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    walk_items_var_only(&doc.items, &mut counts);
    counts
}

fn walk_items_var_only(items: &[Item], counts: &mut HashMap<String, usize>) {
    for item in items {
        match item {
            Item::Rule(r) => walk_rule_var_only(r, counts),
            Item::Decl(d) => walk_value_var_only(&d.value, counts),
            Item::Verbatim(_) => {}
        }
    }
}

fn walk_rule_var_only(rule: &Rule, counts: &mut HashMap<String, usize>) {
    match &rule.body {
        Body::Decls(items) => {
            for di in items {
                if let DeclItem::Decl(d) = di {
                    walk_value_var_only(&d.value, counts);
                }
            }
        }
        Body::Nested(items) => walk_items_var_only(items, counts),
        Body::Verbatim(_) => {}
    }
}

fn walk_value_var_only(node: &Node, counts: &mut HashMap<String, usize>) {
    match node {
        Node::Int(_) | Node::Raw(_) | Node::Style { .. } => {}
        Node::Var { name, fallback } => {
            bump_ref(counts, name);
            if let Some(fb) = fallback {
                walk_value_var_only(fb, counts);
            }
        }
        Node::Calc(inner) | Node::Paren(inner) => walk_value_var_only(inner, counts),
        Node::MathFn { args, .. } | Node::Fn { args, .. } => {
            for a in args {
                walk_value_var_only(a, counts);
            }
        }
        Node::Sum(terms) => {
            for t in terms {
                walk_value_var_only(&t.node, counts);
            }
        }
        Node::Product(factors) => {
            for f in factors {
                walk_value_var_only(f, counts);
            }
        }
        Node::Div(l, r) => {
            walk_value_var_only(l, counts);
            walk_value_var_only(r, counts);
        }
        Node::If { arms, default } => {
            for arm in arms {
                walk_value_var_only(&arm.cond, counts);
                walk_value_var_only(&arm.value, counts);
            }
            walk_value_var_only(default, counts);
        }
        Node::Or(conds) => {
            for c in conds {
                walk_value_var_only(c, counts);
            }
        }
    }
}

/// Count refs that come from `Node::Raw` text and `Body::Verbatim` /
/// top-level `Item::Verbatim` regions. Used to seed the inline-
/// candidate filter — anything mentioned in raw text is risky to
/// inline because we can't AST-substitute into it.
fn count_text_refs_only(doc: &Doc) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    walk_items_text_only(&doc.items, &mut counts);
    counts
}

fn walk_items_text_only(items: &[Item], counts: &mut HashMap<String, usize>) {
    for item in items {
        match item {
            Item::Rule(r) => walk_rule_text_only(r, counts),
            Item::Verbatim(s) => count_refs_in_text(s, counts),
            Item::Decl(d) => walk_value_text_only(&d.value, counts),
        }
    }
}

fn walk_rule_text_only(rule: &Rule, counts: &mut HashMap<String, usize>) {
    match &rule.body {
        Body::Decls(items) => {
            for di in items {
                match di {
                    DeclItem::Verbatim(s) => count_refs_in_text(s, counts),
                    DeclItem::Decl(d) => walk_value_text_only(&d.value, counts),
                }
            }
        }
        Body::Nested(items) => walk_items_text_only(items, counts),
        Body::Verbatim(s) => count_refs_in_text(s, counts),
    }
}

fn walk_value_text_only(node: &Node, counts: &mut HashMap<String, usize>) {
    // Var/Style contribute to var_refs above, not text_refs. We only
    // collect from Raw chunks and Style props here.
    match node {
        Node::Raw(s) => count_refs_in_text(s, counts),
        Node::Style { prop, .. } => bump_ref(counts, prop),
        Node::Int(_) => {}
        Node::Var { fallback, .. } => {
            if let Some(fb) = fallback {
                walk_value_text_only(fb, counts);
            }
        }
        Node::Calc(inner) | Node::Paren(inner) => walk_value_text_only(inner, counts),
        Node::MathFn { args, .. } | Node::Fn { args, .. } => {
            for a in args {
                walk_value_text_only(a, counts);
            }
        }
        Node::Sum(terms) => {
            for t in terms {
                walk_value_text_only(&t.node, counts);
            }
        }
        Node::Product(factors) => {
            for f in factors {
                walk_value_text_only(f, counts);
            }
        }
        Node::Div(l, r) => {
            walk_value_text_only(l, counts);
            walk_value_text_only(r, counts);
        }
        Node::If { arms, default } => {
            for arm in arms {
                walk_value_text_only(&arm.cond, counts);
                walk_value_text_only(&arm.value, counts);
            }
            walk_value_text_only(default, counts);
        }
        Node::Or(conds) => {
            for c in conds {
                walk_value_text_only(c, counts);
            }
        }
    }
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
    let head_trim = rule.head.trim_start();
    if head_trim.starts_with("@function") || head_trim.starts_with("@property") {
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
        Body::Verbatim(_) => {}
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
    // Both checks must agree: total refs == 1 (one mention anywhere)
    // and var refs == 1 (that mention is a Var we can structurally
    // substitute, not a Style/Raw text reference we'd have to splice
    // into raw bytes).
    if total_refs.get(key).copied().unwrap_or(0) != 1 {
        return;
    }
    if var_refs.get(key).copied().unwrap_or(0) != 1 {
        return;
    }
    // Skip pure-alias `--name: var(--other);` decls without
    // fallbacks. Net byte delta is roughly
    // `len(--name) - len(--other)` minus the dropped decl, so the
    // savings are bounded by name-length difference; the bigger cost
    // is that the public `--rN` registers (whose value is exactly
    // `var(--_1rN)`) lose their cascaded names and become opaque to
    // anyone introspecting the output. The Var-WITH-fallback case is
    // fair game — e.g. `--_1rN: var(--_2rN, 0);` carries a real
    // initializer and the inlined form is shorter overall.
    if matches!(&d.value, Node::Var { fallback: None, .. }) {
        return;
    }
    candidates.insert(key.to_string(), d.value.clone());
}

fn value_references_any(node: &Node, names: &std::collections::HashSet<String>) -> bool {
    match node {
        Node::Var { name, .. } => names.contains(name.strip_prefix("--").unwrap_or(name)),
        Node::Int(_) | Node::Raw(_) | Node::Style { .. } => false,
        Node::Calc(inner) | Node::Paren(inner) => value_references_any(inner, names),
        Node::MathFn { args, .. } | Node::Fn { args, .. } => {
            args.iter().any(|a| value_references_any(a, names))
        }
        Node::Sum(terms) => terms.iter().any(|t| value_references_any(&t.node, names)),
        Node::Product(factors) => factors.iter().any(|f| value_references_any(f, names)),
        Node::Div(l, r) => value_references_any(l, names) || value_references_any(r, names),
        Node::If { arms, default } => {
            arms.iter().any(|a| {
                value_references_any(&a.cond, names) || value_references_any(&a.value, names)
            }) || value_references_any(default, names)
        }
        Node::Or(conds) => conds.iter().any(|c| value_references_any(c, names)),
    }
}

fn substitute_in_items(
    items: &mut Vec<Item>,
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
    let head_trim = rule.head.trim_start();
    if head_trim.starts_with("@property") {
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
        Body::Verbatim(_) => {}
    }
}

fn substitute_in_node(
    node: &mut Node,
    candidates: &HashMap<String, Node>,
    substituted: &mut std::collections::HashSet<String>,
) {
    // Bottom-up: substitute children first so any nested replacements
    // happen before we look at the current node.
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
        Node::Int(_) | Node::Raw(_) | Node::Style { .. } => {}
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
        Body::Verbatim(_) => {}
    }
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
        assert_eq!(r.head, ".foo ");
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
}
