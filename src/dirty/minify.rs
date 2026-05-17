use std::collections::{HashMap, HashSet};

use super::{SplitMix64, fisher_yates};
use crate::css::{Arm, Body, DeclItem, Doc, Item, Node, parse_doc, print_doc, skip_css_string};

const RESERVED: &[&str] = &[
    "cop_a", "cop_a0", "cop_a1", "cop_a2", "cop_a3", "cop_b", "cop_b0", "cop_b1", "cop_b2",
    "cop_b3",
];

pub fn minify(html: &str, seed: u64) -> String {
    let mut counts: HashMap<String, usize> = HashMap::new();
    scan_names(html, &mut counts);

    let reserved: HashSet<&str> = RESERVED.iter().copied().collect();
    let mut names: Vec<(String, usize)> = counts
        .into_iter()
        .filter(|(n, _)| !reserved.contains(n.as_str()) && !n.starts_with("__WSS_"))
        .collect();
    names.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));

    let mut rng = SplitMix64::new(seed);
    let mut start = 0;
    while start < names.len() {
        let mut end = start + 1;
        while end < names.len() && names[end].1 == names[start].1 {
            end += 1;
        }
        let slice = &mut names[start..end];
        for i in (1..slice.len()).rev() {
            let j = (rng.next_u64() as usize) % (i + 1);
            slice.swap(i, j);
        }
        start = end;
    }

    let mut name_gen = ShortNameGen::new(&reserved);
    let mut rename: HashMap<String, String> = HashMap::with_capacity(names.len());
    for (old, _) in names {
        rename.insert(old, name_gen.next());
    }

    let renamed = rewrite_names(html, &rename);
    let stripped = strip_comments(&renamed);
    let sorted = sort_style_decls(&stripped, seed);
    flatten_style_whitespace(&sorted)
}

/// Drop CSS `/* … */` inside `<style>` blocks and HTML `<!-- … -->`
/// outside `<style>`/`<script>`. Defensive — KEEP markers are already
/// stripped by `apply_template_features`.
fn strip_comments(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let bytes = html.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some(rest) = html[i..].strip_prefix("<style") {
            let tag_end = rest.find('>').map(|p| i + 6 + p + 1).unwrap_or(bytes.len());
            let body_end = html[tag_end..]
                .find("</style>")
                .map(|p| tag_end + p)
                .unwrap_or(bytes.len());
            out.push_str(&html[i..tag_end]);
            strip_css_block_comments(&html[tag_end..body_end], &mut out);
            i = body_end;
            continue;
        }
        if let Some(_rest) = html[i..].strip_prefix("<script") {
            let close = html[i..]
                .find("</script>")
                .map(|p| i + p + "</script>".len())
                .unwrap_or(bytes.len());
            out.push_str(&html[i..close]);
            i = close;
            continue;
        }
        if html[i..].starts_with("<!--") {
            if let Some(end) = html[i + 4..].find("-->") {
                i += 4 + end + 3;
                continue;
            }
            break;
        }
        let next = bytes[i];
        out.push(next as char);
        i += 1;
    }
    out
}

fn strip_css_block_comments(s: &str, out: &mut String) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            let end = skip_css_string(bytes, i);
            out.push_str(&s[i..end]);
            i = end;
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
        out.push(b as char);
        i += 1;
    }
}

/// Shuffle every `if()` arm-list whose conditions are mutually
/// exclusive (see `mutually_exclusive_arms`).
pub fn shuffle_arms_in_styles(html: &str, seed: u64) -> String {
    transform_style_blocks(html, |doc| {
        let mut rng = SplitMix64::new(seed);
        shuffle_doc_if_arms(doc, &mut rng);
    })
}

/// Reorder operands of commutative ops: `Sum`, `Product`, `min`/`max`/
/// `hypot`, and `Or` chains.
pub fn shuffle_commutative_ops(html: &str, seed: u64) -> String {
    transform_style_blocks(html, |doc| {
        let mut rng = SplitMix64::new(seed);
        for item in &mut doc.items {
            shuffle_commutative_in_item(item, &mut rng);
        }
    })
}

fn shuffle_commutative_in_item(item: &mut Item, rng: &mut SplitMix64) {
    match item {
        Item::Rule(r) => shuffle_commutative_in_body(&mut r.body, rng),
        Item::Decl(d) => shuffle_commutative_in_node(&mut d.value, rng),
        Item::Verbatim(_) => {}
    }
}

fn shuffle_commutative_in_body(body: &mut Body, rng: &mut SplitMix64) {
    match body {
        Body::Decls(items) => {
            for it in items {
                if let DeclItem::Decl(d) = it {
                    shuffle_commutative_in_node(&mut d.value, rng);
                }
            }
        }
        Body::Nested(items) => {
            for it in items {
                shuffle_commutative_in_item(it, rng);
            }
        }
        Body::Verbatim(_) => {}
    }
}

fn shuffle_commutative_in_node(node: &mut Node, rng: &mut SplitMix64) {
    match node {
        Node::Sum(terms) => {
            for t in terms.iter_mut() {
                shuffle_commutative_in_node(&mut t.node, rng);
            }
            fisher_yates(terms, rng);
        }
        Node::Product(factors) => {
            for f in factors.iter_mut() {
                shuffle_commutative_in_node(f, rng);
            }
            fisher_yates(factors, rng);
        }
        Node::MathFn { name, args } => {
            for a in args.iter_mut() {
                shuffle_commutative_in_node(a, rng);
            }
            if is_commutative_math_fn(name) {
                fisher_yates(args, rng);
            }
        }
        Node::Or(conds) => {
            for c in conds.iter_mut() {
                shuffle_commutative_in_node(c, rng);
            }
            fisher_yates(conds, rng);
        }
        Node::If { arms, default } => {
            for arm in arms.iter_mut() {
                shuffle_commutative_in_node(&mut arm.cond, rng);
                shuffle_commutative_in_node(&mut arm.value, rng);
            }
            shuffle_commutative_in_node(default, rng);
        }
        Node::Fn { args, .. } => {
            for a in args.iter_mut() {
                shuffle_commutative_in_node(a, rng);
            }
        }
        Node::Calc(inner) | Node::Paren(inner) => shuffle_commutative_in_node(inner, rng),
        Node::Div(l, r) => {
            shuffle_commutative_in_node(l, rng);
            shuffle_commutative_in_node(r, rng);
        }
        Node::Var {
            fallback: Some(fb), ..
        } => shuffle_commutative_in_node(fb, rng),
        _ => {}
    }
}

fn is_commutative_math_fn(name: &str) -> bool {
    matches!(name, "min" | "max" | "hypot")
}

/// Permute every `@property` / `@function` definition among the
/// positions those at-rules originally occupied. Other rules and
/// verbatim text stay anchored.
pub fn shuffle_at_rule_order(html: &str, seed: u64) -> String {
    transform_style_blocks(html, |doc| {
        let mut rng = SplitMix64::new(seed);
        permute_at_rules(&mut doc.items, &mut rng);
        for item in &mut doc.items {
            if let Item::Rule(r) = item
                && let Body::Nested(inner) = &mut r.body
            {
                permute_at_rules(inner, &mut rng);
            }
        }
    })
}

fn permute_at_rules(items: &mut [Item], rng: &mut SplitMix64) {
    let positions: Vec<usize> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| is_shufflable_at_rule(it).then_some(i))
        .collect();
    if positions.len() < 2 {
        return;
    }
    let mut rules: Vec<Item> = positions.iter().map(|&i| items[i].clone()).collect();
    fisher_yates(&mut rules, rng);
    for (i, r) in positions.into_iter().zip(rules) {
        items[i] = r;
    }
}

fn is_shufflable_at_rule(item: &Item) -> bool {
    if let Item::Rule(r) = item {
        let head = r.head.trim_start();
        head.starts_with("@property") || head.starts_with("@function")
    } else {
        false
    }
}

/// Inject a random integer fallback into every unfallbacked
/// `var(--x)` whose `--x` is registered via `@property`. Unregistered
/// names are skipped — WSS reads the unset previous-stage shadow as a
/// sentinel; a fallback would replace it.
pub fn inject_var_fallbacks(html: &str, seed: u64) -> String {
    transform_style_blocks(html, |doc| {
        let registered = collect_registered_properties(doc);
        let mut rng = SplitMix64::new(seed);
        for item in &mut doc.items {
            inject_in_item(item, &registered, &mut rng);
        }
    })
}

fn collect_registered_properties(doc: &Doc) -> HashSet<String> {
    let mut out = HashSet::new();
    for item in &doc.items {
        if let Item::Rule(r) = item {
            let head = r.head.trim_start();
            if let Some(rest) = head.strip_prefix("@property") {
                let name = rest.trim_start().trim_end_matches('{').trim();
                if let Some(stripped) = name.strip_prefix("--") {
                    out.insert(stripped.to_string());
                }
            }
        }
    }
    out
}

fn inject_in_item(item: &mut Item, registered: &HashSet<String>, rng: &mut SplitMix64) {
    match item {
        Item::Rule(r) => {
            let head = r.head.trim_start();
            if head.starts_with("@property") {
                return;
            }
            inject_in_body(&mut r.body, registered, rng);
        }
        Item::Decl(d) => inject_in_node(&mut d.value, registered, rng),
        Item::Verbatim(_) => {}
    }
}

fn inject_in_body(body: &mut Body, registered: &HashSet<String>, rng: &mut SplitMix64) {
    match body {
        Body::Decls(items) => {
            for it in items {
                if let DeclItem::Decl(d) = it {
                    inject_in_node(&mut d.value, registered, rng);
                }
            }
        }
        Body::Nested(items) => {
            for it in items {
                inject_in_item(it, registered, rng);
            }
        }
        Body::Verbatim(_) => {}
    }
}

/// Inject unreachable decoy arms into every `<integer>`-returning
/// LUT-shaped `@function`. Decoys key on the same predicate with a
/// negative literal the runtime never passes; values read a real
/// registered integer property.
pub fn inject_lut_decoy_arms(html: &str, seed: u64) -> String {
    transform_style_blocks(html, |doc| {
        let candidates = collect_integer_var_candidates(doc);
        if candidates.is_empty() {
            return;
        }
        let mut rng = SplitMix64::new(seed);
        for item in &mut doc.items {
            bake_in_item(item, &candidates, &mut rng);
        }
    })
}

fn collect_integer_var_candidates(doc: &Doc) -> Vec<String> {
    let mut out = Vec::new();
    for item in &doc.items {
        let Item::Rule(r) = item else { continue };
        let head = r.head.trim_start();
        let Some(rest) = head.strip_prefix("@property") else {
            continue;
        };
        let name = rest.trim().trim_end_matches('{').trim();
        let Some(stripped) = name.strip_prefix("--") else {
            continue;
        };
        let body_is_int_typed = if let Body::Decls(items) = &r.body {
            items.iter().any(|di| {
                let DeclItem::Decl(d) = di else { return false };
                if d.name != "syntax" {
                    return false;
                }
                let v = d.value.to_css();
                v.contains("<integer>") || v.contains("<number>")
            })
        } else {
            false
        };
        if body_is_int_typed {
            out.push(stripped.to_string());
        }
    }
    out
}

fn bake_in_item(item: &mut Item, candidates: &[String], rng: &mut SplitMix64) {
    let Item::Rule(r) = item else { return };
    let head = r.head.trim_start();
    if !head.starts_with("@function") {
        return;
    }
    if !head.contains("returns <integer>") {
        return;
    }
    let Body::Decls(items) = &mut r.body else {
        return;
    };
    for di in items {
        let DeclItem::Decl(d) = di else { continue };
        if d.name != "result" {
            continue;
        }
        bake_into_result(&mut d.value, candidates, rng);
    }
}

fn bake_into_result(node: &mut Node, candidates: &[String], rng: &mut SplitMix64) {
    let Node::If { arms, .. } = node else { return };
    if arms.is_empty() {
        return;
    }
    let Some(prop) = lut_key_prop(arms) else {
        return;
    };
    let decoy_count = (arms.len() / 4).clamp(2, 12);
    let mut used: HashSet<i64> = HashSet::new();
    for _ in 0..decoy_count {
        let mut idx = -(((rng.next_u64() % 1024) + 1) as i64);
        for _ in 0..16 {
            if used.insert(idx) {
                break;
            }
            idx -= 1;
        }
        let var_name = &candidates[(rng.next_u64() as usize) % candidates.len()];
        let decoy = Arm {
            cond: Node::Style {
                prop: prop.clone(),
                value: idx.to_string(),
            },
            value: Node::Var {
                name: format!("--{}", var_name),
                fallback: None,
            },
        };
        let pos = (rng.next_u64() as usize) % (arms.len() + 1);
        arms.insert(pos, decoy);
    }
}

/// Split each PC-keyed `if()` chain into a parent decl plus helpers
/// linked through `else: var(--helperN)`. Lazy `var()` resolution
/// keeps total arm work unchanged. Helpers get `--__{N}` names that
/// `--minify-vars` then renames.
pub fn split_pc_branches(html: &str, seed: u64) -> String {
    transform_style_blocks(html, |doc| {
        let mut rng = SplitMix64::new(seed);
        let mut counter: u32 = 0;
        for item in &mut doc.items {
            split_pc_in_item(item, &mut counter, &mut rng);
        }
    })
}

fn split_pc_in_item(item: &mut Item, counter: &mut u32, rng: &mut SplitMix64) {
    match item {
        Item::Rule(r) => match &mut r.body {
            Body::Decls(items) => split_pc_in_decl_items(items, counter, rng),
            Body::Nested(inner) => {
                split_pc_in_nested(inner, counter, rng);
                for it in inner {
                    split_pc_in_item(it, counter, rng);
                }
            }
            Body::Verbatim(_) => {}
        },
        Item::Decl(_) | Item::Verbatim(_) => {}
    }
}

fn split_pc_in_nested(items: &mut Vec<Item>, counter: &mut u32, rng: &mut SplitMix64) {
    let mut new_items: Vec<Item> = Vec::with_capacity(items.len() * 2);
    for it in items.drain(..) {
        if let Item::Decl(ref d) = it
            && let Some(splits) = try_split_pc_decl(d, counter, rng)
        {
            for h in splits.helpers {
                new_items.push(Item::Decl(h));
                new_items.push(Item::Verbatim(" ".to_string()));
            }
            new_items.push(Item::Decl(splits.modified));
            continue;
        }
        new_items.push(it);
    }
    *items = new_items;
}

fn split_pc_in_decl_items(items: &mut Vec<DeclItem>, counter: &mut u32, rng: &mut SplitMix64) {
    let mut new_items: Vec<DeclItem> = Vec::with_capacity(items.len() * 2);
    for di in items.drain(..) {
        if let DeclItem::Decl(ref d) = di
            && let Some(splits) = try_split_pc_decl(d, counter, rng)
        {
            for h in splits.helpers {
                new_items.push(DeclItem::Decl(h));
                new_items.push(DeclItem::Verbatim(" ".to_string()));
            }
            new_items.push(DeclItem::Decl(splits.modified));
            continue;
        }
        new_items.push(di);
    }
    *items = new_items;
}

struct PcSplit {
    helpers: Vec<crate::css::Decl>,
    modified: crate::css::Decl,
}

fn try_split_pc_decl(
    d: &crate::css::Decl,
    counter: &mut u32,
    rng: &mut SplitMix64,
) -> Option<PcSplit> {
    let Node::If { arms, default } = &d.value else {
        return None;
    };
    if arms.len() < 3 {
        return None;
    }
    if !mutually_exclusive_arms(arms) {
        return None;
    }
    // Only branches keyed on a PC-shaped prop. We don't know post-rename
    // what name the PC takes, so we look at the candidate set of PC-like
    // names; if minify renamed them already, this pass should run first.
    let prop = lut_key_prop(arms)?;
    if !is_pc_prop(&prop) {
        return None;
    }
    // Random bucket count in [2, 4]; never more buckets than arms.
    let bucket_count = (2 + (rng.next_u64() as usize) % 3).min(arms.len());
    let mut chunks: Vec<Vec<Arm>> = chunk_arms(arms, bucket_count, rng);
    if chunks.len() < 2 {
        return None;
    }
    let first_chunk = chunks.remove(0);
    // Build helper chain in reverse so each helper falls back to the next.
    let mut tail_default: Node = (**default).clone();
    let mut helpers_rev: Vec<crate::css::Decl> = Vec::with_capacity(chunks.len());
    for chunk in chunks.into_iter().rev() {
        let helper_name = format!("--__{}", *counter);
        *counter += 1;
        let helper_decl = crate::css::Decl {
            name: helper_name.clone(),
            value: Node::If {
                arms: chunk,
                default: Box::new(tail_default.clone()),
            },
        };
        tail_default = Node::Var {
            name: helper_name,
            fallback: None,
        };
        helpers_rev.push(helper_decl);
    }
    helpers_rev.reverse();
    let modified = crate::css::Decl {
        name: d.name.clone(),
        value: Node::If {
            arms: first_chunk,
            default: Box::new(tail_default),
        },
    };
    Some(PcSplit {
        helpers: helpers_rev,
        modified,
    })
}

fn chunk_arms(arms: &[Arm], bucket_count: usize, rng: &mut SplitMix64) -> Vec<Vec<Arm>> {
    // Random partition: pick `bucket_count - 1` distinct split points
    // in `1..arms.len()`. Always emit chunks of >= 1 arm.
    if arms.len() < bucket_count || bucket_count < 2 {
        return vec![arms.to_vec()];
    }
    let mut splits: Vec<usize> = Vec::with_capacity(bucket_count - 1);
    let mut taken: HashSet<usize> = HashSet::new();
    while splits.len() < bucket_count - 1 {
        let p = 1 + ((rng.next_u64() as usize) % (arms.len() - 1));
        if taken.insert(p) {
            splits.push(p);
        }
    }
    splits.sort_unstable();
    let mut chunks = Vec::with_capacity(bucket_count);
    let mut start = 0;
    for end in splits {
        chunks.push(arms[start..end].to_vec());
        start = end;
    }
    chunks.push(arms[start..].to_vec());
    chunks
}

fn is_pc_prop(name: &str) -> bool {
    matches!(name, "--pc" | "--_0pc" | "--_1pc" | "--_2pc")
}

/// Strip comments and collapse whitespace inside `<script>` blocks.
/// Preserves string / template / regex literals; keeps newlines for ASI.
pub fn minify_embedded_js(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut search_from = 0;
    while let Some(rel) = html[search_from..].find("<script") {
        let open_tag = search_from + rel;
        let Some(gt_rel) = html[open_tag..].find('>') else {
            break;
        };
        let body_start = open_tag + gt_rel + 1;
        let Some(close_rel) = html[body_start..].find("</script>") else {
            break;
        };
        let body_end = body_start + close_rel;
        out.push_str(&html[search_from..body_start]);
        minify_js_run(&html[body_start..body_end], &mut out);
        search_from = body_end;
    }
    out.push_str(&html[search_from..]);
    out
}

fn minify_js_run(s: &str, out: &mut String) {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut last_sig: u8 = b'\n';
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' || b == b'`' {
            let end = skip_js_string(bytes, i);
            out.push_str(&s[i..end]);
            last_sig = b;
            i = end;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() {
            if bytes[i + 1] == b'/' {
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            if bytes[i + 1] == b'*' {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(bytes.len());
                continue;
            }
            if regex_allowed_after(last_sig) {
                let end = skip_js_regex(bytes, i);
                out.push_str(&s[i..end]);
                last_sig = b')';
                i = end;
                continue;
            }
        }
        if matches!(b, b' ' | b'\t' | b'\r') {
            let tail = out.as_bytes().last().copied();
            if !matches!(tail, Some(b' ') | Some(b'\n') | None) {
                out.push(' ');
            }
            i += 1;
            continue;
        }
        if b == b'\n' {
            out.truncate(out.trim_end_matches(' ').len());
            if !out.ends_with('\n') {
                out.push('\n');
            }
            i += 1;
            continue;
        }
        out.push(b as char);
        last_sig = b;
        i += 1;
    }
}

fn skip_js_string(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'\\' && i + 1 < bytes.len() {
            i += 2;
            continue;
        }
        if quote == b'`' && b == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // template interpolation — skip the `${...}` expression as
            // balanced braces, with nested strings recognised so a `}`
            // inside `"…}"` doesn't close the placeholder early.
            i += 2;
            let mut depth = 1;
            while i < bytes.len() && depth > 0 {
                match bytes[i] {
                    b'"' | b'\'' | b'`' => i = skip_js_string(bytes, i),
                    b'{' => {
                        depth += 1;
                        i += 1;
                    }
                    b'}' => {
                        depth -= 1;
                        i += 1;
                    }
                    _ => i += 1,
                }
            }
            continue;
        }
        if b == quote {
            return i + 1;
        }
        i += 1;
    }
    bytes.len()
}

fn skip_js_regex(bytes: &[u8], start: usize) -> usize {
    let mut i = start + 1;
    let mut in_class = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => i += 2,
            b'[' => {
                in_class = true;
                i += 1;
            }
            b']' if in_class => {
                in_class = false;
                i += 1;
            }
            b'/' if !in_class => {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
                    i += 1;
                }
                return i;
            }
            b'\n' => return i,
            _ => i += 1,
        }
    }
    i
}

/// Whether `/` at this point opens a regex literal (vs division),
/// decided by the preceding non-whitespace byte.
fn regex_allowed_after(prev: u8) -> bool {
    matches!(
        prev,
        b'(' | b','
            | b'='
            | b':'
            | b';'
            | b'!'
            | b'&'
            | b'|'
            | b'?'
            | b'+'
            | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'<'
            | b'>'
            | b'^'
            | b'~'
            | b'['
            | b'{'
            | b'\n'
    )
}

/// Shared `style(--p: K)` property all arm conditions key on, or
/// `None` if the shape isn't a single-prop LUT.
fn lut_key_prop(arms: &[Arm]) -> Option<String> {
    let mut prop: Option<String> = None;
    for arm in arms {
        for (p, _) in collect_style_terms(&arm.cond) {
            match &prop {
                None => prop = Some(p),
                Some(prev) if *prev == p => {}
                _ => return None,
            }
        }
    }
    prop
}

fn inject_in_node(node: &mut Node, registered: &HashSet<String>, rng: &mut SplitMix64) {
    match node {
        Node::Var { name, fallback } => {
            if let Some(fb) = fallback {
                inject_in_node(fb, registered, rng);
            } else {
                let key = name.strip_prefix("--").unwrap_or(name.as_str());
                if registered.contains(key) {
                    let n = (rng.next_u64() as i64) & 0xFFFF;
                    *fallback = Some(Box::new(Node::Int(n)));
                }
            }
        }
        Node::Calc(inner) | Node::Paren(inner) => inject_in_node(inner, registered, rng),
        Node::MathFn { args, .. } | Node::Fn { args, .. } => {
            for a in args {
                inject_in_node(a, registered, rng);
            }
        }
        Node::Sum(terms) => {
            for t in terms {
                inject_in_node(&mut t.node, registered, rng);
            }
        }
        Node::Product(factors) => {
            for f in factors {
                inject_in_node(f, registered, rng);
            }
        }
        Node::Div(l, r) => {
            inject_in_node(l, registered, rng);
            inject_in_node(r, registered, rng);
        }
        Node::If { arms, default } => {
            for arm in arms {
                inject_in_node(&mut arm.cond, registered, rng);
                inject_in_node(&mut arm.value, registered, rng);
            }
            inject_in_node(default, registered, rng);
        }
        Node::Or(conds) => {
            for c in conds {
                inject_in_node(c, registered, rng);
            }
        }
        _ => {}
    }
}

/// Replace every newline inside `<style>` blocks with a single space
/// (preserving string literals).
fn flatten_style_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut search_from = 0;
    while let Some(rel) = s[search_from..].find("<style") {
        let open_tag = search_from + rel;
        let Some(gt_rel) = s[open_tag..].find('>') else {
            break;
        };
        let body_start = open_tag + gt_rel + 1;
        let Some(close_rel) = s[body_start..].find("</style>") else {
            break;
        };
        let body_end = body_start + close_rel;
        out.push_str(&s[search_from..body_start]);
        flatten_css_run(&s[body_start..body_end], &mut out);
        search_from = body_end;
    }
    out.push_str(&s[search_from..]);
    out
}

fn flatten_css_run(s: &str, out: &mut String) {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut last_was_space = false;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            let end = skip_css_string(bytes, i);
            out.push_str(&s[i..end]);
            i = end;
            last_was_space = false;
            continue;
        }
        if matches!(b, b' ' | b'\t' | b'\n' | b'\r') {
            if !last_was_space {
                out.push(' ');
                last_was_space = true;
            }
            i += 1;
            continue;
        }
        out.push(b as char);
        last_was_space = false;
        i += 1;
    }
}

fn scan_names(s: &str, counts: &mut HashMap<String, usize>) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'-' && bytes[i + 1] == b'-' && is_ident_start(bytes[i + 2]) {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && is_ident_cont(bytes[j]) {
                j += 1;
            }
            *counts.entry(s[start..j].to_string()).or_insert(0) += 1;
            i = j;
        } else {
            i += 1;
        }
    }
}

fn rewrite_names(s: &str, rename: &HashMap<String, String>) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'-' && bytes[i + 1] == b'-' && is_ident_start(bytes[i + 2]) {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && is_ident_cont(bytes[j]) {
                j += 1;
            }
            if let Some(new) = rename.get(&s[start..j]) {
                out.push_str(&s[last..start]);
                out.push_str(new);
                last = j;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    out.push_str(&s[last..]);
    out
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

fn sort_style_decls(html: &str, seed: u64) -> String {
    let mut rng = SplitMix64::new(seed ^ 0xA5A5_5A5A_DEAD_BEEF);
    transform_style_blocks(html, |doc| sort_doc_decls(doc, &mut rng))
}

fn transform_style_blocks(html: &str, mut transform: impl FnMut(&mut Doc)) -> String {
    let mut out = String::with_capacity(html.len());
    let mut last = 0;
    let mut search_from = 0;
    while let Some(rel) = html[search_from..].find("<style") {
        let open_tag = search_from + rel;
        let Some(gt_rel) = html[open_tag..].find('>') else {
            break;
        };
        let body_start = open_tag + gt_rel + 1;
        let Some(close_rel) = html[body_start..].find("</style>") else {
            break;
        };
        let body_end = body_start + close_rel;
        out.push_str(&html[last..body_start]);
        let mut doc = parse_doc(&html[body_start..body_end]);
        transform(&mut doc);
        out.push_str(&print_doc(&doc));
        last = body_end;
        search_from = body_end;
    }
    out.push_str(&html[last..]);
    out
}

/// Walk every `Node::If` and shuffle arm-lists whose conditions are
/// mutually exclusive single-prop `style()` queries.
fn shuffle_doc_if_arms(doc: &mut Doc, rng: &mut SplitMix64) {
    for item in &mut doc.items {
        shuffle_in_item(item, rng);
    }
}

fn shuffle_in_item(item: &mut Item, rng: &mut SplitMix64) {
    match item {
        Item::Rule(r) => shuffle_in_body(&mut r.body, rng),
        Item::Decl(d) => shuffle_in_node(&mut d.value, rng),
        Item::Verbatim(_) => {}
    }
}

fn shuffle_in_body(body: &mut Body, rng: &mut SplitMix64) {
    match body {
        Body::Decls(items) => {
            for it in items {
                if let DeclItem::Decl(d) = it {
                    shuffle_in_node(&mut d.value, rng);
                }
            }
        }
        Body::Nested(items) => {
            for it in items {
                shuffle_in_item(it, rng);
            }
        }
        Body::Verbatim(_) => {}
    }
}

fn shuffle_in_node(node: &mut Node, rng: &mut SplitMix64) {
    match node {
        Node::If { arms, default } => {
            for arm in arms.iter_mut() {
                shuffle_in_node(&mut arm.cond, rng);
                shuffle_in_node(&mut arm.value, rng);
            }
            shuffle_in_node(default, rng);
            if mutually_exclusive_arms(arms) && arms.len() > 1 {
                for i in (1..arms.len()).rev() {
                    let j = (rng.next_u64() as usize) % (i + 1);
                    arms.swap(i, j);
                }
            }
        }
        Node::Calc(inner) | Node::Paren(inner) => shuffle_in_node(inner, rng),
        Node::MathFn { args, .. } | Node::Fn { args, .. } => {
            for a in args {
                shuffle_in_node(a, rng);
            }
        }
        Node::Sum(terms) => {
            for t in terms {
                shuffle_in_node(&mut t.node, rng);
            }
        }
        Node::Product(factors) => {
            for f in factors {
                shuffle_in_node(f, rng);
            }
        }
        Node::Div(l, r) => {
            shuffle_in_node(l, rng);
            shuffle_in_node(r, rng);
        }
        Node::Var {
            fallback: Some(fb), ..
        } => shuffle_in_node(fb, rng),
        Node::Or(conds) => {
            for c in conds {
                shuffle_in_node(c, rng);
            }
        }
        _ => {}
    }
}

/// True iff every arm condition is `style(--p: K)` (or `Or` of same)
/// keyed on a single shared `--p` with distinct `K`s — at most one arm
/// matches any input, so order is irrelevant.
fn mutually_exclusive_arms(arms: &[Arm]) -> bool {
    let mut prop: Option<String> = None;
    let mut seen: HashSet<String> = HashSet::new();
    for arm in arms {
        let styles = collect_style_terms(&arm.cond);
        if styles.is_empty() {
            return false;
        }
        for (p, v) in styles {
            match &prop {
                None => prop = Some(p.clone()),
                Some(prev) if prev == &p => {}
                _ => return false,
            }
            if !seen.insert(v) {
                return false;
            }
        }
    }
    true
}

fn collect_style_terms(node: &Node) -> Vec<(String, String)> {
    match node {
        Node::Style { prop, value } => vec![(prop.clone(), value.clone())],
        Node::Or(conds) => {
            let mut out = Vec::new();
            for c in conds {
                let inner = collect_style_terms(c);
                if inner.is_empty() {
                    return Vec::new();
                }
                out.extend(inner);
            }
            out
        }
        _ => Vec::new(),
    }
}

fn sort_doc_decls(doc: &mut Doc, rng: &mut SplitMix64) {
    for item in &mut doc.items {
        if let Item::Rule(rule) = item {
            let randomize = rule.head.trim_start().starts_with("@property");
            sort_body(&mut rule.body, randomize, rng);
        }
    }
}

fn sort_body(body: &mut Body, randomize: bool, rng: &mut SplitMix64) {
    match body {
        Body::Decls(items) => order_decl_items(items, randomize, rng),
        Body::Nested(items) => {
            sort_nested_items(items);
            for it in items {
                if let Item::Rule(r) = it {
                    let inner_randomize = r.head.trim_start().starts_with("@property");
                    sort_body(&mut r.body, inner_randomize, rng);
                }
            }
        }
        Body::Verbatim(_) => {}
    }
}

/// Sort `Item::Decl` entries alphabetically within each decl-region.
/// Nested rules act as barriers; verbatim slots stay put.
fn sort_nested_items(items: &mut [Item]) {
    let n = items.len();
    let mut start = 0;
    while start < n {
        let mut end = start;
        while end < n && !matches!(items[end], Item::Rule(_)) {
            end += 1;
        }
        if end > start {
            sort_decl_positions(&mut items[start..end]);
        }
        start = if end < n { end + 1 } else { end };
    }
}

fn sort_decl_positions(items: &mut [Item]) {
    let positions: Vec<usize> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| matches!(it, Item::Decl(_)).then_some(i))
        .collect();
    if positions.len() < 2 {
        return;
    }
    let mut decls: Vec<_> = positions
        .iter()
        .map(|&i| match &items[i] {
            Item::Decl(d) => d.clone(),
            _ => unreachable!(),
        })
        .collect();
    decls.sort_by(|a, b| a.name.cmp(&b.name));
    for (i, d) in positions.into_iter().zip(decls) {
        items[i] = Item::Decl(d);
    }
}

/// Reorder decls at their original positions: shuffled if `randomize`,
/// alphabetical otherwise.
fn order_decl_items(items: &mut [DeclItem], randomize: bool, rng: &mut SplitMix64) {
    let positions: Vec<usize> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| matches!(it, DeclItem::Decl(_)).then_some(i))
        .collect();
    if positions.len() < 2 {
        return;
    }
    let mut decls: Vec<_> = positions
        .iter()
        .map(|&i| match &items[i] {
            DeclItem::Decl(d) => d.clone(),
            _ => unreachable!(),
        })
        .collect();
    if randomize {
        fisher_yates(&mut decls, rng);
    } else {
        decls.sort_by(|a, b| a.name.cmp(&b.name));
    }
    for (i, d) in positions.into_iter().zip(decls) {
        items[i] = DeclItem::Decl(d);
    }
}

struct ShortNameGen<'a> {
    counter: usize,
    reserved: &'a HashSet<&'a str>,
}

impl<'a> ShortNameGen<'a> {
    fn new(reserved: &'a HashSet<&'a str>) -> Self {
        Self {
            counter: 0,
            reserved,
        }
    }

    fn next(&mut self) -> String {
        loop {
            let n = encode(self.counter);
            self.counter += 1;
            if !self.reserved.contains(n.as_str()) {
                return n;
            }
        }
    }
}

fn encode(mut n: usize) -> String {
    const ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let base = ALPHA.len();
    let mut buf = Vec::new();
    n += 1;
    while n > 0 {
        n -= 1;
        buf.push(ALPHA[n % base]);
        n /= base;
    }
    buf.reverse();
    String::from_utf8(buf).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encode_short_names() {
        assert_eq!(encode(0), "a");
        assert_eq!(encode(25), "z");
        assert_eq!(encode(26), "A");
        assert_eq!(encode(51), "Z");
        assert_eq!(encode(52), "aa");
        assert_eq!(encode(53), "ab");
        assert_eq!(encode(52 + 51), "aZ");
        assert_eq!(encode(52 + 52), "ba");
    }

    #[test]
    fn rename_skips_reserved() {
        let css = "<style>:root{--cop_a:1;--m0000:2;--m0000-x:3;--m0001:var(--m0000);}</style>";
        let out = minify(css, 0);
        assert!(out.contains("--cop_a"));
        assert!(!out.contains("--m0000"));
        assert!(out.contains("--a"));
    }

    #[test]
    fn rename_frequency_assigns_shortest_to_hottest() {
        let css = "<style>:root{--rare:1;--hot:var(--hot);--hot2:var(--hot);}</style>";
        let out = minify(css, 0);
        assert!(out.contains("--a:"));
        assert!(out.contains("var(--a)"));
    }

    #[test]
    fn sort_decls_alphabetical() {
        let css = "<style>:root{--z:1;--a:2;--m:3;}</style>";
        let out = minify(css, 0);
        let body_start = out.find('{').unwrap();
        let body_end = out.find('}').unwrap();
        let body = &out[body_start..body_end];
        let pa = body.find("--a:").unwrap();
        let pb = body.find("--b:").unwrap();
        let pc = body.find("--c:").unwrap();
        assert!(pa < pb && pb < pc, "expected sorted: {body}");
    }

    #[test]
    fn placeholder_markers_untouched() {
        let css = "<style>:root{--__WSS_KEEP_X__: 1;--foo:2;}</style>";
        let out = minify(css, 0);
        assert!(out.contains("--__WSS_KEEP_X__"));
    }
}
