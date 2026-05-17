use std::collections::{HashMap, HashSet};

use super::{SplitMix64, fisher_yates};
use crate::css::{
    Arm, Body, DeclItem, Doc, Item, Node, count_custom_names, is_custom_name_cont,
    is_custom_name_start, rename_custom_names, scrub_verbatim, skip_css_string,
};
use crate::page::Page;

const RESERVED: &[&str] = &[
    "cop_a", "cop_a0", "cop_a1", "cop_a2", "cop_a3", "cop_b", "cop_b0", "cop_b1", "cop_b2",
    "cop_b3",
];

// =====================================================================
// minify (variable renaming + cosmetic cleanup)
// =====================================================================

pub fn minify(page: &mut Page, seed: u64) {
    rename_short(page, seed);
    strip_comments_and_flatten(page);
    sort_decls(page, seed);
}

fn rename_short(page: &mut Page, seed: u64) {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for doc in page.style_docs() {
        count_custom_names(doc, &mut counts);
    }

    for s in page.text_and_scripts() {
        scan_names_raw(s, &mut counts);
    }

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

    for doc in page.style_docs_mut() {
        rename_custom_names(doc, &rename);
    }
    page.map_text_and_scripts(|s| rewrite_names_raw(s, &rename));
}

fn scan_names_raw(s: &str, counts: &mut HashMap<String, usize>) {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'-' && bytes[i + 1] == b'-' && is_custom_name_start(bytes[i + 2]) {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && is_custom_name_cont(bytes[j]) {
                j += 1;
            }
            *counts.entry(s[start..j].to_string()).or_insert(0) += 1;
            i = j;
        } else {
            i += 1;
        }
    }
}

fn rewrite_names_raw(s: &str, rename: &HashMap<String, String>) -> String {
    let bytes = s.as_bytes();
    let mut out = String::with_capacity(s.len());
    let mut last = 0;
    let mut i = 0;
    while i + 2 < bytes.len() {
        if bytes[i] == b'-' && bytes[i + 1] == b'-' && is_custom_name_start(bytes[i + 2]) {
            let start = i + 2;
            let mut j = start;
            while j < bytes.len() && is_custom_name_cont(bytes[j]) {
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

fn strip_comments_and_flatten(page: &mut Page) {
    for doc in page.style_docs_mut() {
        scrub_verbatim(doc, &strip_and_flatten_text);
    }
}

fn strip_and_flatten_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
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
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
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
    out
}

// =====================================================================
// shuffle / decoy / split-pc passes (each operates on every Doc)
// =====================================================================

pub fn shuffle_arms_in_styles(page: &mut Page, seed: u64) {
    let mut rng = SplitMix64::new(seed);
    for doc in page.style_docs_mut() {
        shuffle_doc_if_arms(doc, &mut rng);
    }
}

pub fn shuffle_commutative_ops(page: &mut Page, seed: u64) {
    let mut rng = SplitMix64::new(seed);
    for doc in page.style_docs_mut() {
        for item in &mut doc.items {
            shuffle_commutative_in_item(item, &mut rng);
        }
    }
}

pub fn shuffle_at_rule_order(page: &mut Page, seed: u64) {
    let mut rng = SplitMix64::new(seed);
    for doc in page.style_docs_mut() {
        permute_at_rules(&mut doc.items, &mut rng);
        for item in &mut doc.items {
            if let Item::Rule(r) = item
                && let Body::Nested(inner) = &mut r.body
            {
                permute_at_rules(inner, &mut rng);
            }
        }
    }
}

pub fn inject_var_fallbacks(page: &mut Page, seed: u64) {
    let mut registered: HashSet<String> = HashSet::new();
    for doc in page.style_docs() {
        collect_registered_properties(doc, &mut registered);
    }
    let mut rng = SplitMix64::new(seed);
    for doc in page.style_docs_mut() {
        for item in &mut doc.items {
            inject_in_item(item, &registered, &mut rng);
        }
    }
}

pub fn inject_lut_decoy_arms(page: &mut Page, seed: u64) {
    let mut candidates: Vec<String> = Vec::new();
    for doc in page.style_docs() {
        collect_integer_var_candidates(doc, &mut candidates);
    }
    if candidates.is_empty() {
        return;
    }
    let mut rng = SplitMix64::new(seed);
    for doc in page.style_docs_mut() {
        for item in &mut doc.items {
            bake_in_item(item, &candidates, &mut rng);
        }
    }
}

pub fn split_pc_branches(page: &mut Page, seed: u64) {
    let mut rng = SplitMix64::new(seed);
    let mut counter: u32 = 0;
    for doc in page.style_docs_mut() {
        for item in &mut doc.items {
            split_pc_in_item(item, &mut counter, &mut rng);
        }
    }
}

pub fn minify_embedded_js(page: &mut Page) {
    for body in page.scripts_mut() {
        let mut out = String::with_capacity(body.len());
        minify_js_run(body, &mut out);
        *body = out;
    }
}

// =====================================================================
// shuffle_commutative_ops
// =====================================================================

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

// =====================================================================
// shuffle_at_rule_order
// =====================================================================

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

// =====================================================================
// inject_var_fallbacks
// =====================================================================

fn collect_registered_properties(doc: &Doc, out: &mut HashSet<String>) {
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

// =====================================================================
// inject_lut_decoy_arms
// =====================================================================

fn collect_integer_var_candidates(doc: &Doc, out: &mut Vec<String>) {
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
                match &d.value {
                    Node::Raw(s) => s.contains("<integer>") || s.contains("<number>"),
                    _ => false,
                }
            })
        } else {
            false
        };
        if body_is_int_typed {
            out.push(stripped.to_string());
        }
    }
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

// =====================================================================
// split_pc_branches
// =====================================================================

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
    let prop = lut_key_prop(arms)?;
    if !is_pc_prop(&prop) {
        return None;
    }
    let bucket_count = (2 + (rng.next_u64() as usize) % 3).min(arms.len());
    let mut chunks: Vec<Vec<Arm>> = chunk_arms(arms, bucket_count, rng);
    if chunks.len() < 2 {
        return None;
    }
    let first_chunk = chunks.remove(0);
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

// =====================================================================
// minify_embedded_js
// =====================================================================

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

// =====================================================================
// shared helpers
// =====================================================================

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
            if mutually_exclusive_arms(arms) {
                fisher_yates(arms, rng);
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

// =====================================================================
// minify sort_decls
// =====================================================================

fn sort_decls(page: &mut Page, seed: u64) {
    let mut rng = SplitMix64::new(seed ^ 0xA5A5_5A5A_DEAD_BEEF);
    for doc in page.style_docs_mut() {
        sort_doc_decls(doc, &mut rng);
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

// =====================================================================
// short-name generator
// =====================================================================

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

// =====================================================================
// tests
// =====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn run(html: &str, mut f: impl FnMut(&mut Page)) -> String {
        let mut page = Page::from_html(html);
        f(&mut page);
        page.print()
    }

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
        let out = run(css, |p| minify(p, 0));
        assert!(out.contains("--cop_a"));
        assert!(!out.contains("--m0000"));
        assert!(out.contains("--a"));
    }

    #[test]
    fn rename_frequency_assigns_shortest_to_hottest() {
        let css = "<style>:root{--rare:1;--hot:var(--hot);--hot2:var(--hot);}</style>";
        let out = run(css, |p| minify(p, 0));
        assert!(out.contains("--a:"));
        assert!(out.contains("var(--a)"));
    }

    #[test]
    fn sort_decls_alphabetical() {
        let css = "<style>:root{--z:1;--a:2;--m:3;}</style>";
        let out = run(css, |p| minify(p, 0));
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
        let out = run(css, |p| minify(p, 0));
        assert!(out.contains("--__WSS_KEEP_X__"));
    }

    #[test]
    fn rename_propagates_into_script_string_literals() {
        // A name used in both <style> and a JS string literal must be
        // renamed in both places, so the JS still references the live
        // CSS property after minify.
        let css = "<style>:root{--myprop: 1;}</style>\
                   <script>const v = getComputedStyle(e).getPropertyValue('--myprop');</script>";
        let out = run(css, |p| minify(p, 0));
        // The original name has been renamed everywhere.
        assert!(!out.contains("--myprop"));
        // The new name appears in both segments — find it.
        let new_name = out
            .split("--")
            .nth(1)
            .and_then(|s| s.split(|c: char| !c.is_ascii_alphanumeric()).next())
            .unwrap();
        let token = format!("--{new_name}");
        let occurrences = out.matches(&token).count();
        assert!(occurrences >= 2, "expected >=2 occurrences in {out}");
    }

    #[test]
    fn rename_propagates_into_shell_html() {
        // CSS variables referenced from HTML attributes (counter-reset,
        // style attributes, etc.) must rename consistently.
        let css = "<style>:root{--foo: 1;}</style>\
                   <div style=\"counter-reset: ind-foo var(--foo); width: 10px;\">x</div>";
        let out = run(css, |p| minify(p, 0));
        assert!(!out.contains("--foo"));
    }

    #[test]
    fn strip_comments_removes_block_comments_in_style() {
        let css = "<style>/* gone */:root{--x: 1;/* also gone */}</style>";
        let out = run(css, |p| minify(p, 0));
        assert!(!out.contains("gone"));
    }

    #[test]
    fn strip_comments_preserves_strings() {
        let css = "<style>:root{--label: \"/* not a comment */\";}</style>";
        let out = run(css, |p| minify(p, 0));
        assert!(
            out.contains("/* not a comment */"),
            "string contents stripped: {out}"
        );
    }

    #[test]
    fn flatten_whitespace_collapses_runs() {
        let css = "<style>:root  {\n    --x:    1;\n}</style>";
        let out = run(css, |p| minify(p, 0));
        // The body should not contain consecutive spaces or any newlines
        // after flattening.
        assert!(!out.contains("  "), "double space: {out}");
        assert!(!out.contains('\n'), "newline left: {out}");
    }

    #[test]
    fn shuffle_arms_is_seed_deterministic() {
        let css = "<style>.a { --x: if(style(--p: 1): 1; style(--p: 2): 2; style(--p: 3): 3; style(--p: 4): 4; else: 0); }</style>";
        let a = run(css, |p| shuffle_arms_in_styles(p, 42));
        let b = run(css, |p| shuffle_arms_in_styles(p, 42));
        assert_eq!(a, b, "same seed must produce same output");
    }

    #[test]
    fn shuffle_arms_changes_with_seed() {
        // With enough arms, different seeds should produce different
        // orderings.
        let css = "<style>.a { --x: if(style(--p: 1): 1; style(--p: 2): 2; style(--p: 3): 3; style(--p: 4): 4; style(--p: 5): 5; style(--p: 6): 6; else: 0); }</style>";
        let a = run(css, |p| shuffle_arms_in_styles(p, 1));
        let b = run(css, |p| shuffle_arms_in_styles(p, 999));
        assert_ne!(a, b, "different seeds should differ");
    }

    #[test]
    fn shuffle_arms_preserves_arm_set() {
        // The bag of (cond, value) pairs is invariant under shuffle.
        let css = "<style>.a { --x: if(style(--p: 1): 10; style(--p: 2): 20; style(--p: 3): 30; else: 0); }</style>";
        let out = run(css, |p| shuffle_arms_in_styles(p, 7));
        for expected in [
            "style(--p: 1): 10",
            "style(--p: 2): 20",
            "style(--p: 3): 30",
        ] {
            assert!(out.contains(expected), "missing arm: {expected} in {out}");
        }
    }

    #[test]
    fn shuffle_commutative_keeps_set() {
        // For an addition, the operand set is invariant under shuffle.
        let css = "<style>.a { --x: calc(11 + 22 + 33 + 44); }</style>";
        let out = run(css, |p| shuffle_commutative_ops(p, 5));
        for n in ["11", "22", "33", "44"] {
            assert!(out.contains(n), "lost {n} in {out}");
        }
    }

    #[test]
    fn shuffle_at_rule_order_keeps_set() {
        let css = "<style>@property --a { syntax: \"<integer>\"; initial-value: 0; inherits: true; }\
                   @property --b { syntax: \"<integer>\"; initial-value: 0; inherits: true; }\
                   @property --c { syntax: \"<integer>\"; initial-value: 0; inherits: true; }</style>";
        let out = run(css, |p| shuffle_at_rule_order(p, 11));
        for n in ["--a", "--b", "--c"] {
            assert!(out.contains(n));
        }
    }

    #[test]
    fn inject_var_fallbacks_only_touches_registered() {
        let css = "<style>@property --reg { syntax: \"<integer>\"; initial-value: 0; inherits: true; }\
                   .x { --y: var(--reg); --z: var(--unreg); }</style>";
        let out = run(css, |p| inject_var_fallbacks(p, 3));
        // The registered Var must gain a fallback.
        assert!(out.contains("var(--reg, "), "no fallback on --reg: {out}");
        // The unregistered Var must NOT gain a fallback.
        assert!(out.contains("var(--unreg);"), "unreg got fallback: {out}");
    }

    #[test]
    fn inject_lut_decoy_arms_grows_function() {
        // A LUT-shaped @function with enough arms to satisfy the
        // decoy_count clamp must receive at least two new arms.
        let arms = (0..16)
            .map(|i| format!("style(--_1pc: {i}): {i};"))
            .collect::<Vec<_>>()
            .join(" ");
        let css = format!(
            "<style>@property --src {{ syntax: \"<integer>\"; initial-value: 0; inherits: true; }}\
             @function --lut() returns <integer> {{ result: if({arms} else: 0); }}</style>"
        );
        let out = run(&css, |p| inject_lut_decoy_arms(p, 17));
        // Count `style(--_1pc:` occurrences — should be >16 after
        // decoys are inserted.
        let occurrences = out.matches("style(--_1pc:").count();
        assert!(
            occurrences > 16,
            "expected decoy arms inserted; got {occurrences}: {out}"
        );
    }

    #[test]
    fn split_pc_branches_no_op_on_short_chain() {
        let css = "<style>.x { --pc: if(style(--pc: 0): 1; style(--pc: 1): 2; else: 0); }</style>";
        let out = run(css, |p| split_pc_branches(p, 5));
        // Two arms is below the split threshold; the helper prefix
        // `--__` must not appear.
        assert!(!out.contains("--__"), "unexpected helper: {out}");
    }

    #[test]
    fn split_pc_branches_emits_helpers() {
        // 5 arms with PC-shaped prop should split into multiple buckets.
        let css = "<style>.x { --pc: if(style(--pc: 0): 0; style(--pc: 1): 1; style(--pc: 2): 2; style(--pc: 3): 3; style(--pc: 4): 4; else: -1); }</style>";
        let out = run(css, |p| split_pc_branches(p, 1));
        assert!(out.contains("--__"), "expected helper decl: {out}");
    }

    #[test]
    fn minify_js_collapses_whitespace_in_script() {
        let html = "<script>\n  let   x   =   1;\n  let   y   =   2;\n</script>";
        let out = run(html, minify_embedded_js);
        assert!(!out.contains("   "), "whitespace not collapsed: {out}");
        assert!(out.contains("let x = 1;"));
    }

    #[test]
    fn minify_js_preserves_string_contents() {
        let html = "<script>const s = '  spaced  string  ';</script>";
        let out = run(html, minify_embedded_js);
        assert!(
            out.contains("'  spaced  string  '"),
            "string spacing lost: {out}"
        );
    }

    #[test]
    fn minify_js_strips_comments() {
        let html = "<script>// kill me\nvar x = 1; /* and me */ var y = 2;</script>";
        let out = run(html, minify_embedded_js);
        assert!(!out.contains("kill me"));
        assert!(!out.contains("and me"));
    }

    #[test]
    fn full_pipeline_chain_runs_each_pass_once() {
        // Sequence each pass on one Page and verify the page still
        // parses & prints cleanly. This is a smoke test that the new
        // pipeline doesn't corrupt the AST when passes are chained.
        let html = "<style>\
            @property --pc { syntax: \"<integer>\"; initial-value: 0; inherits: true; }\
            .x { --pc: if(style(--pc: 0): 0; style(--pc: 1): 1; style(--pc: 2): 2; style(--pc: 3): 3; else: -1); --y: calc(1 + 2 + 3); }\
        </style><script>const v = '--pc';</script>";
        let mut page = Page::from_html(html);
        split_pc_branches(&mut page, 1);
        shuffle_arms_in_styles(&mut page, 2);
        shuffle_commutative_ops(&mut page, 3);
        shuffle_at_rule_order(&mut page, 4);
        inject_var_fallbacks(&mut page, 5);
        minify(&mut page, 6);
        minify_embedded_js(&mut page);
        let out = page.print();
        // Sanity: must still be a well-formed HTML+style fragment.
        assert!(out.contains("<style>") && out.contains("</style>"));
        assert!(out.contains("<script>") && out.contains("</script>"));
    }
}
