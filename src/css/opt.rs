//! Fact-based rewrites over the emitted CSS.
//!
//! An interval + integrality analysis runs over every declaration (a
//! fixpoint across the whole cascade, animations included). Properties it
//! proves constant are replaced by the constant, and properties proven to
//! always hold the same value (partition refinement over their definitions)
//! are read through one representative. Both make whole register / memory
//! chains dead: their declarations, shadow copies and animated keyframe
//! entries are then dropped by DCE, which is where the time goes in Blink
//! (every declaration is re-resolved on every style recalc).

use std::collections::{HashMap, HashSet};

use super::doc::{Body, DeclItem, Doc, Item, Rule, is_custom_name_cont, visit_value_mut};
use super::fold::BOOL_HELPERS;
use super::{Node, Sign};

/// Interval of possible values plus an "always an integer" flag. Unknown
/// bounds are infinite; integrality is tracked independently of bounds
/// (every registered `<integer>` property is integral whatever its range).
#[derive(Clone, Copy, Debug, PartialEq)]
struct Range {
    lo: f64,
    hi: f64,
    int: bool,
}

const TOP: Range = Range {
    lo: f64::NEG_INFINITY,
    hi: f64::INFINITY,
    int: false,
};

/// No value at all (a property no declaration has assigned yet during the
/// optimistic fixpoint): the identity of `union`.
const EMPTY: Range = Range {
    lo: f64::INFINITY,
    hi: f64::NEG_INFINITY,
    int: true,
};

const INT_TOP: Range = Range {
    lo: f64::NEG_INFINITY,
    hi: f64::INFINITY,
    int: true,
};

impl Range {
    fn point(v: f64) -> Self {
        Self {
            lo: v,
            hi: v,
            int: v.fract() == 0.0,
        }
    }
    fn new(lo: f64, hi: f64, int: bool) -> Self {
        if lo.is_nan() || hi.is_nan() {
            return Self { int, ..TOP };
        }
        Self { lo, hi, int }
    }
    fn union(self, o: Self) -> Self {
        Self {
            lo: self.lo.min(o.lo),
            hi: self.hi.max(o.hi),
            int: self.int && o.int,
        }
    }
    fn within(self, lo: f64, hi: f64) -> bool {
        self.lo >= lo && self.hi <= hi
    }
    fn is_empty(self) -> bool {
        self.lo > self.hi
    }
    fn as_point_int(self) -> Option<i64> {
        (self.int && self.lo == self.hi && self.lo.abs() < 1e15).then_some(self.lo as i64)
    }
    fn round_to_int(self) -> Self {
        Self::new(self.lo.round(), self.hi.round(), true)
    }
}

fn signum0(v: f64) -> f64 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        0.0
    }
}

fn mul_bound(a: f64, b: f64) -> f64 {
    // 0 * inf contributes 0 to the product interval.
    if a == 0.0 || b == 0.0 { 0.0 } else { a * b }
}

type Facts = HashMap<String, Range>;

/// Static knowledge about properties written outside the emitted rules
/// (from JS or the base template).
fn external_range(name: &str) -> Range {
    match name {
        "--kb" => Range::new(-1.0, 255.0, true),
        "--clk" => Range::new(0.0, 3.0, true),
        n if n.starts_with("--rng") || n.starts_with("--cop_o") => Range::new(-1.0, 255.0, true),
        _ => TOP,
    }
}

/// Bindings visible inside an `@function` body.
#[derive(Default, Clone)]
struct Scope {
    /// Parameter / local name → integral by declared type.
    names: HashMap<String, bool>,
}

struct Analyzer<'a> {
    facts: &'a Facts,
    /// Result ranges of user `@function`s.
    fn_ranges: &'a HashMap<String, Range>,
    scope: &'a Scope,
}

impl Analyzer<'_> {
    fn var(&self, name: &str) -> Range {
        if let Some(&int) = self.scope.names.get(name) {
            return if int { INT_TOP } else { TOP };
        }
        match self.facts.get(name) {
            Some(r) => *r,
            None => external_range(name),
        }
    }

    fn range(&self, node: &Node) -> Range {
        match node {
            Node::Int(n) => Range::point(*n as f64),
            Node::Var { name, fallback } => {
                let r = self.var(name);
                match fallback {
                    None => r,
                    Some(fb) => r.union(self.range(fb)),
                }
            }
            Node::Calc(inner) | Node::Paren(inner) => self.range(inner),
            Node::Sum(terms) => {
                let mut acc = Range::point(0.0);
                for t in terms {
                    let r = self.range(&t.node);
                    if r.is_empty() {
                        return EMPTY;
                    }
                    acc = match t.sign {
                        Sign::Pos => Range::new(acc.lo + r.lo, acc.hi + r.hi, acc.int && r.int),
                        Sign::Neg => Range::new(acc.lo - r.hi, acc.hi - r.lo, acc.int && r.int),
                    };
                }
                acc
            }
            Node::Product(factors) => {
                let mut acc = Range::point(1.0);
                for f in factors {
                    let r = self.range(f);
                    if r.is_empty() {
                        return EMPTY;
                    }
                    let c = [
                        mul_bound(acc.lo, r.lo),
                        mul_bound(acc.lo, r.hi),
                        mul_bound(acc.hi, r.lo),
                        mul_bound(acc.hi, r.hi),
                    ];
                    let lo = c.iter().copied().fold(f64::INFINITY, f64::min);
                    let hi = c.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                    acc = Range::new(lo, hi, acc.int && r.int);
                }
                acc
            }
            Node::Div(l, r) => {
                let l = self.range(l);
                let r = self.range(r);
                if l.is_empty() || r.is_empty() {
                    return EMPTY;
                }
                match r.as_point_int() {
                    Some(k) if k > 0 => {
                        let k = k as f64;
                        Range::new(l.lo / k, l.hi / k, k == 1.0 && l.int)
                    }
                    _ => TOP,
                }
            }
            Node::MathFn { name, args } | Node::Fn { name, args } => {
                // Each argument's range is computed once and handed down:
                // re-deriving it per use is exponential in nesting depth.
                let rs: Vec<Range> = args.iter().map(|a| self.range(a)).collect();
                if rs.iter().any(|r| r.is_empty()) {
                    return EMPTY;
                }
                if matches!(node, Node::MathFn { .. }) {
                    self.math_fn_range(name, args, &rs)
                } else {
                    self.fn_range(name, &rs)
                }
            }
            Node::If { arms, default } => {
                let mut acc = self.range(default);
                for arm in arms {
                    acc = acc.union(self.range(&arm.value));
                }
                acc
            }
            Node::Raw(_) | Node::Style { .. } | Node::Or(_) | Node::Invalid => TOP,
        }
    }

    /// Ranges of `@function` calls: user functions analyzed from their
    /// bodies, plus the fixed helpers defined by the base template.
    fn fn_range(&self, name: &str, args: &[Range]) -> Range {
        if let Some(r) = self.fn_ranges.get(name) {
            return *r;
        }
        let bool_r = Range::new(0.0, 1.0, true);
        let byte_r = Range::new(0.0, 255.0, true);
        match (name, args) {
            (n, _) if BOOL_HELPERS.contains(&n) => bool_r,
            ("--mpar", _) => bool_r,
            ("--mlo" | "--mhi" | "--mload", _) => byte_r,
            ("--mhalf", [b]) => Range::new((b.lo / 2.0).floor(), (b.hi / 2.0).floor(), true),
            ("--sel", [_, t, f]) => t.union(*f),
            _ => TOP,
        }
    }

    fn math_fn_range(&self, name: &str, args: &[Node], rs: &[Range]) -> Range {
        match (name, rs) {
            ("mod", [x, m]) => match m.as_point_int() {
                Some(m) if m > 0 => {
                    if x.int && x.within(0.0, (m - 1) as f64) {
                        *x
                    } else {
                        Range::new(0.0, (m - 1) as f64, x.int)
                    }
                }
                _ => TOP,
            },
            ("round", [_, x]) => {
                let strategy = match args.first() {
                    Some(Node::Raw(s)) => s.as_str(),
                    _ => return TOP,
                };
                let (lo, hi) = match strategy {
                    "down" => (x.lo.floor(), x.hi.floor()),
                    "up" => (x.lo.ceil(), x.hi.ceil()),
                    "to-zero" => (x.lo.trunc(), x.hi.trunc()),
                    _ => (x.lo.round(), x.hi.round()),
                };
                Range::new(lo, hi, true)
            }
            ("round", [x]) => x.round_to_int(),
            ("abs", [x]) => {
                let (lo, hi) = if x.lo >= 0.0 {
                    (x.lo, x.hi)
                } else if x.hi <= 0.0 {
                    (-x.hi, -x.lo)
                } else {
                    (0.0, x.hi.max(-x.lo))
                };
                Range::new(lo, hi, x.int)
            }
            ("sign", [x]) => Range::new(signum0(x.lo), signum0(x.hi), true),
            ("min" | "max", _) if !rs.is_empty() => {
                let int = rs.iter().all(|r| r.int);
                let lo = rs.iter().map(|r| r.lo);
                let hi = rs.iter().map(|r| r.hi);
                if name == "min" {
                    Range::new(
                        lo.fold(f64::INFINITY, f64::min),
                        hi.fold(f64::INFINITY, f64::min),
                        int,
                    )
                } else {
                    Range::new(
                        lo.fold(f64::NEG_INFINITY, f64::max),
                        hi.fold(f64::NEG_INFINITY, f64::max),
                        int,
                    )
                }
            }
            _ => TOP,
        }
    }
}

// ---------------------------------------------------------------------
// Doc traversal
// ---------------------------------------------------------------------

/// A declaration together with the `@function` it lives in, if any.
struct DeclRef<'a> {
    name: &'a str,
    value: &'a Node,
    function: Option<String>,
}

/// `@function --name(--p <type>, ...)` → (name, [(param, integral)]).
fn function_head_info(head: &str) -> Option<(String, Vec<(String, bool)>)> {
    let rest = head.trim().strip_prefix("@function")?.trim_start();
    let open = rest.find('(')?;
    let name = rest[..open].trim().to_string();
    let close = rest[open..].find(')')? + open;
    let params = rest[open + 1..close]
        .split(',')
        .filter_map(|p| {
            let mut it = p.split_whitespace();
            let name = it.next()?;
            name.starts_with("--")
                .then(|| (name.to_string(), it.next() == Some("<integer>")))
        })
        .collect();
    Some((name, params))
}

fn collect_decls<'a>(items: &'a [Item], function: Option<&str>, out: &mut Vec<DeclRef<'a>>) {
    for item in items {
        match item {
            Item::Decl(d) => out.push(DeclRef {
                name: &d.name,
                value: &d.value,
                function: function.map(str::to_string),
            }),
            Item::Rule(r) => collect_rule_decls(r, function, out),
            Item::Verbatim(_) => {}
        }
    }
}

fn collect_rule_decls<'a>(rule: &'a Rule, function: Option<&str>, out: &mut Vec<DeclRef<'a>>) {
    let fn_name = function_head_info(&rule.head).map(|(n, _)| n);
    let scope = fn_name.as_deref().or(function);
    match &rule.body {
        Body::Decls(items) => {
            for di in items {
                if let DeclItem::Decl(d) = di {
                    out.push(DeclRef {
                        name: &d.name,
                        value: &d.value,
                        function: scope.map(str::to_string),
                    });
                }
            }
        }
        Body::Nested(items) => collect_decls(items, scope, out),
        Body::Verbatim(_) => {}
    }
}

/// Function name → every name bound inside it (parameters and locals).
fn collect_function_scopes(items: &[Item], out: &mut HashMap<String, Scope>) {
    for item in items {
        if let Item::Rule(r) = item {
            if let Some((name, params)) = function_head_info(&r.head) {
                let mut scope = Scope::default();
                scope.names.extend(params);
                if let Body::Decls(decls) = &r.body {
                    for di in decls {
                        if let DeclItem::Decl(d) = di
                            && d.name.starts_with("--")
                        {
                            scope.names.insert(d.name.clone(), false);
                        }
                    }
                }
                out.insert(name, scope);
            }
            if let Body::Nested(inner) = &r.body {
                collect_function_scopes(inner, out);
            }
        }
    }
}

fn rewrite_items(
    items: &mut [Item],
    function: Option<&str>,
    f: &mut impl FnMut(Option<&str>, Node) -> Node,
) {
    for item in items {
        match item {
            Item::Decl(d) => d.value = f(function, std::mem::replace(&mut d.value, Node::Int(0))),
            Item::Rule(r) => {
                let fn_name = function_head_info(&r.head).map(|(n, _)| n);
                let scope = fn_name.as_deref().or(function);
                match &mut r.body {
                    Body::Decls(decls) => {
                        for di in decls {
                            if let DeclItem::Decl(d) = di {
                                d.value = f(scope, std::mem::replace(&mut d.value, Node::Int(0)));
                            }
                        }
                    }
                    Body::Nested(inner) => rewrite_items(inner, scope, f),
                    Body::Verbatim(_) => {}
                }
            }
            Item::Verbatim(_) => {}
        }
    }
}

// ---------------------------------------------------------------------
// Fact computation
// ---------------------------------------------------------------------

/// `@property` registrations: name → (is `<integer>` syntax, initial value).
pub type Registrations = HashMap<String, (bool, Option<i64>)>;

pub fn parse_registrations(props_css: &str) -> Registrations {
    let mut out = Registrations::new();
    for chunk in props_css.split("@property ").skip(1) {
        let Some(name) = chunk.split_whitespace().next() else {
            continue;
        };
        let int = chunk.contains("syntax: \"<integer>\"");
        let init = chunk
            .split("initial-value:")
            .nth(1)
            .and_then(|r| r.split(';').next())
            .and_then(|v| v.trim().parse::<i64>().ok());
        out.insert(name.to_string(), (int, init));
    }
    out
}

const MAX_FACT_ITERS: usize = 96;

/// Fixpoint over every declaration: a property's range is the union of
/// its initial value and every value any declaration can assign to it.
/// Starts optimistic (unassigned = no contribution) and widens to the
/// type's top when a range keeps growing.
fn compute_facts(
    decls: &[DeclRef],
    scopes: &HashMap<String, Scope>,
    regs: &Registrations,
    external_names: &HashSet<String>,
) -> Facts {
    let is_int = |name: &str| regs.get(name).is_some_and(|(int, _)| *int);
    let top_of = |name: &str| if is_int(name) { INT_TOP } else { TOP };

    // Unregistered properties start at `EMPTY`: bottom (no writer has
    // produced a value yet) contributes nothing.
    let mut cur: Facts = HashMap::new();
    for d in decls {
        if d.function.is_none() && d.name.starts_with("--") {
            cur.entry(d.name.to_string()).or_insert_with(|| {
                if external_names.contains(d.name) {
                    return top_of(d.name);
                }
                match regs.get(d.name) {
                    Some((_, Some(init))) => Range::point(*init as f64),
                    Some((_, None)) => top_of(d.name),
                    None => EMPTY,
                }
            });
        }
    }
    let mut fn_ranges: HashMap<String, Range> = HashMap::new();
    let empty = Scope::default();

    for iter in 0..=MAX_FACT_ITERS {
        let facts = cur.clone();
        let mut changed = false;
        let mut next_fn: HashMap<String, Range> = HashMap::new();
        for d in decls {
            let scope = d
                .function
                .as_deref()
                .and_then(|f| scopes.get(f))
                .unwrap_or(&empty);
            let a = Analyzer {
                facts: &facts,
                fn_ranges: &fn_ranges,
                scope,
            };
            let r = a.range(d.value);
            match d.function.as_deref() {
                Some(f) if d.name == "result" => {
                    let e = next_fn.entry(f.to_string()).or_insert(r);
                    *e = e.union(r);
                }
                Some(_) => {}
                None if d.name.starts_with("--") => {
                    let r = if is_int(d.name) { r.round_to_int() } else { r };
                    let slot = cur.get_mut(d.name).expect("every declared name is seeded");
                    let merged = slot.union(r);
                    if *slot != merged {
                        *slot = if iter >= MAX_FACT_ITERS * 3 / 4 {
                            top_of(d.name)
                        } else {
                            merged
                        };
                        changed = true;
                    }
                }
                None => {}
            }
        }
        for (name, r) in next_fn {
            if fn_ranges.get(&name) != Some(&r) {
                fn_ranges.insert(name, r);
                changed = true;
            }
        }
        if !changed {
            break;
        }
        if iter == MAX_FACT_ITERS {
            // Did not converge: give up on everything.
            for (n, r) in cur.iter_mut() {
                *r = top_of(n);
            }
        }
    }
    cur
}

// ---------------------------------------------------------------------
// Constant propagation and equivalent-property merging
// ---------------------------------------------------------------------

/// Where a custom property is defined; definitions only interchange within
/// the same context (the element's own rule vs. one animation family).
fn decl_contexts<'a>(docs: &'a [&mut Doc]) -> HashMap<String, Vec<(String, &'a Node)>> {
    fn walk<'a>(items: &'a [Item], ctx: &str, out: &mut HashMap<String, Vec<(String, &'a Node)>>) {
        for item in items {
            match item {
                Item::Decl(d) if d.name.starts_with("--") => out
                    .entry(d.name.clone())
                    .or_default()
                    .push((ctx.to_string(), &d.value)),
                Item::Rule(r) => {
                    let head = r.head.trim();
                    if head.starts_with("@function") || head.starts_with("@property") {
                        continue;
                    }
                    // `store0`/`store1`/... are one family running in lockstep.
                    let inner = match head.strip_prefix("@keyframes") {
                        Some(name) => format!(
                            "kf:{}",
                            name.trim().trim_end_matches(|c: char| c.is_ascii_digit())
                        ),
                        None => format!("{ctx}>{head}"),
                    };
                    match &r.body {
                        Body::Decls(decls) => {
                            for di in decls {
                                if let DeclItem::Decl(d) = di
                                    && d.name.starts_with("--")
                                {
                                    out.entry(d.name.clone())
                                        .or_default()
                                        .push((inner.clone(), &d.value));
                                }
                            }
                        }
                        Body::Nested(nested) => walk(nested, &inner, out),
                        Body::Verbatim(_) => {}
                    }
                }
                _ => {}
            }
        }
    }
    let mut out = HashMap::new();
    for doc in docs {
        walk(&doc.items, "top", &mut out);
    }
    out
}

/// Partition refinement: two properties stay equivalent while they share a
/// context, registration and initial value, and their definitions are equal
/// once every reference is replaced by its equivalence class. The coarsest
/// stable partition is a bisimulation: such properties hold the same value
/// on every frame, so readers can all use one representative.
fn equivalence_classes(
    defs: &HashMap<String, Vec<(String, &Node)>>,
    regs: &Registrations,
    external_names: &HashSet<String>,
) -> HashMap<String, String> {
    let mut names: Vec<&String> = defs
        .iter()
        .filter(|(n, d)| d.len() == 1 && !external_names.contains(*n))
        .map(|(n, _)| n)
        .collect();
    names.sort();
    let index: HashMap<&str, usize> = names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    // Each definition as a template (its CSS with every reference to a
    // candidate cut out) plus the candidates it references, in order: two
    // definitions are equal under a partition when their templates match
    // and their references are pairwise in the same class.
    let mut templates: HashMap<String, usize> = HashMap::new();
    let shapes: Vec<(usize, Vec<usize>)> = names
        .iter()
        .map(|n| {
            let mut def = defs[*n][0].1.clone();
            let mut refs = Vec::new();
            visit_value_mut(&mut def, &mut |node| {
                if let Node::Var { name, .. } | Node::Style { prop: name, .. } = node
                    && let Some(&i) = index.get(name.as_str())
                {
                    refs.push(i);
                    *name = "--#".to_string();
                }
            });
            let next = templates.len();
            (*templates.entry(def.to_css()).or_insert(next), refs)
        })
        .collect();
    let mut keys: HashMap<String, usize> = HashMap::new();
    let mut class: Vec<usize> = names
        .iter()
        .map(|n| {
            let key = format!("{}|{:?}", defs[*n][0].0, regs.get(*n));
            let next = keys.len();
            *keys.entry(key).or_insert(next)
        })
        .collect();
    let mut class_count = keys.len();
    loop {
        let mut keys: HashMap<(usize, usize, Vec<usize>), usize> = HashMap::new();
        let next_class: Vec<usize> = shapes
            .iter()
            .zip(&class)
            .map(|((template, refs), &c)| {
                let key = (c, *template, refs.iter().map(|&r| class[r]).collect());
                let next = keys.len();
                *keys.entry(key).or_insert(next)
            })
            .collect();
        class = next_class;
        if keys.len() == class_count {
            break;
        }
        class_count = keys.len();
    }
    let mut rep: HashMap<usize, &String> = HashMap::new();
    let mut out = HashMap::new();
    for (n, c) in names.iter().zip(&class) {
        match rep.get(c) {
            Some(r) => {
                out.insert((*n).clone(), (*r).clone());
            }
            None => {
                rep.insert(*c, n);
            }
        }
    }
    out
}

/// Replace reads of properties proven constant (range facts) by the
/// constant, and redirect reads of provably equal properties to one
/// representative. The now-unread definitions are left for DCE.
pub fn propagate_state_facts(
    docs: &mut [&mut Doc],
    regs: &Registrations,
    external_names: &HashSet<String>,
) {
    let mut scopes: HashMap<String, Scope> = HashMap::new();
    let mut decls: Vec<DeclRef> = Vec::new();
    for doc in docs.iter() {
        collect_function_scopes(&doc.items, &mut scopes);
        collect_decls(&doc.items, None, &mut decls);
    }
    let facts = compute_facts(&decls, &scopes, regs, external_names);
    drop(decls);
    let constants: HashMap<String, i64> = facts
        .iter()
        .filter(|(n, _)| !external_names.contains(*n))
        .filter_map(|(n, r)| r.as_point_int().map(|v| (n.clone(), v)))
        .collect();
    let equiv = equivalence_classes(&decl_contexts(docs), regs, external_names);
    for doc in docs.iter_mut() {
        rewrite_items(&mut doc.items, None, &mut |function, mut value| {
            let locals = function.and_then(|f| scopes.get(f));
            let shadowed = |n: &str| locals.is_some_and(|s| s.names.contains_key(n));
            visit_value_mut(&mut value, &mut |n| match n {
                Node::Var { name, .. } if !shadowed(name.as_str()) => {
                    if let Some(&v) = constants.get(name.as_str()) {
                        *n = Node::Int(v);
                    } else if let Some(rep) = equiv.get(name.as_str()) {
                        *name = rep.clone();
                    }
                }
                Node::Style { prop, .. } if !shadowed(prop.as_str()) => {
                    if let Some(rep) = equiv.get(prop.as_str()) {
                        *prop = rep.clone();
                    }
                }
                // Values the AST does not model (e.g. the visualizers'
                // `box-shadow` lists) still read cells by name.
                Node::Raw(text) => *text = substitute_text(text, &constants, &equiv),
                _ => {}
            });
            super::fold(value)
        });
        super::scrub_verbatim(doc, &|text| substitute_text(text, &constants, &equiv));
    }
}

/// Apply constants / representatives to `var(--name)` references in
/// unparsed CSS text.
fn substitute_text(
    text: &str,
    constants: &HashMap<String, i64>,
    equiv: &HashMap<String, String>,
) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("var(") {
        out.push_str(&rest[..i + 4]);
        rest = &rest[i + 4..];
        let end = rest
            .bytes()
            .position(|b| !is_custom_name_cont(b))
            .unwrap_or(rest.len());
        let name = &rest[..end];
        let closes = rest[end..].starts_with(')');
        match (constants.get(name), equiv.get(name)) {
            (Some(v), _) if closes => {
                out.truncate(out.len() - 4);
                out.push_str(&v.to_string());
                rest = &rest[end + 1..];
            }
            (_, Some(rep)) => {
                out.push_str(rep);
                rest = &rest[end..];
            }
            _ => {
                out.push_str(name);
                rest = &rest[end..];
            }
        }
    }
    out.push_str(rest);
    fold_literal_calls(&out)
}

/// Evaluate base-template helper calls whose arguments are all integer
/// literals (what constant substitution leaves in unparsed text, e.g. the
/// visualizers' `rgb(--mlo(27944), ...)`).
fn fold_literal_calls(text: &str) -> String {
    fn eval(name: &str, a: &[i64]) -> Option<i64> {
        let b = |c: bool| i64::from(c);
        Some(match (name, a) {
            ("--mlo", [w]) => w.rem_euclid(256),
            ("--mhi", [w]) => w.div_euclid(256).rem_euclid(256),
            ("--mhalf", [x]) => x.div_euclid(2),
            ("--mpar", [x]) => x.rem_euclid(2),
            ("--eq", [x, y]) => b(x == y),
            ("--ne", [x, y]) => b(x != y),
            ("--eqz", [x]) => b(*x == 0),
            ("--nez", [x]) => b(*x != 0),
            ("--eq1", [x]) => b(*x == 1),
            ("--lt", [x, y]) => b(x < y),
            ("--ge", [x, y]) => b(x >= y),
            _ => return None,
        })
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(i) = rest.find("--") {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let name_end = rest[2..]
            .bytes()
            .position(|b| !is_custom_name_cont(b))
            .map_or(rest.len(), |e| e + 2);
        let name = &rest[..name_end];
        let folded = rest[name_end..].strip_prefix('(').and_then(|args| {
            let close = args.find(')')?;
            let vals: Option<Vec<i64>> = args[..close]
                .split(',')
                .map(|v| v.trim().parse().ok())
                .collect();
            eval(name, &vals?).map(|v| (v, name_end + 1 + close + 1))
        });
        match folded {
            Some((v, used)) => {
                out.push_str(&v.to_string());
                rest = &rest[used..];
            }
            None => {
                out.push_str(name);
                rest = &rest[name_end..];
            }
        }
    }
    out.push_str(rest);
    out
}
