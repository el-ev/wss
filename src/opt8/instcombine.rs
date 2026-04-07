use super::facts::{build_reg_facts, const_fact, imm_kind, is_bool_fact};
use super::*;

// Let's just pretend this is somewhat useful for now
#[derive(Default)]
pub(super) struct InstCombine {
    facts: HashMap<Val8, super::facts::RegFact>,
    defs: HashMap<Val8, Inst8Kind>,
}

impl InstCombine {
    pub(super) fn new() -> Self {
        Self::default()
    }

    pub(super) fn run(&mut self, blocks: &mut [BasicBlock8]) -> bool {
        if blocks.is_empty() {
            self.facts.clear();
            self.defs.clear();
            return false;
        }

        self.facts = build_reg_facts(blocks);
        self.defs = build_unique_defs(blocks);
        combine_blocks(blocks, &self.facts, &self.defs)
    }
}

pub(super) fn instcombine(prog: &mut Ir8Program) -> bool {
    let mut changed = false;
    for blocks in &mut prog.func_blocks {
        let mut pass = InstCombine::new();
        changed |= pass.run(blocks);
    }
    changed
}

fn build_unique_defs(blocks: &[BasicBlock8]) -> HashMap<Val8, Inst8Kind> {
    let def_counts = collect_def_counts(blocks);
    let mut defs = HashMap::new();

    for bb in blocks {
        for inst in &bb.insts {
            let Some(dst) = inst.dst else {
                continue;
            };
            if def_counts.get(&dst).copied().unwrap_or(0) == 1 {
                defs.insert(dst, inst.kind);
            }
        }
    }

    defs
}

fn combine_blocks(
    blocks: &mut [BasicBlock8],
    facts: &HashMap<Val8, super::facts::RegFact>,
    defs: &HashMap<Val8, Inst8Kind>,
) -> bool {
    let mut changed = false;

    for bb in blocks.iter_mut() {
        for inst in &mut bb.insts {
            let new_kind = combine_kind(inst.kind, facts, defs);
            if new_kind != inst.kind {
                inst.kind = new_kind;
                changed = true;
            }
        }

        let old_term = bb.terminator.clone();
        bb.terminator = combine_term(old_term.clone(), facts, defs);
        if bb.terminator != old_term {
            changed = true;
        }

        let old_len = bb.insts.len();
        bb.insts
            .retain(|inst| !matches!(inst.kind, Inst8Kind::Copy(src) if inst.dst == Some(src)));
        changed |= bb.insts.len() != old_len;
    }

    changed
}

fn combine_term(
    term: Terminator8,
    facts: &HashMap<Val8, super::facts::RegFact>,
    defs: &HashMap<Val8, Inst8Kind>,
) -> Terminator8 {
    match term {
        Terminator8::Branch {
            cond,
            if_true,
            if_false,
        } => match resolve_bool_view(cond, facts, defs) {
            Some((base, true)) => Terminator8::Branch {
                cond: base,
                if_true: if_false,
                if_false: if_true,
            },
            Some((base, false)) if base != cond => Terminator8::Branch {
                cond: base,
                if_true,
                if_false,
            },
            _ => Terminator8::Branch {
                cond,
                if_true,
                if_false,
            },
        },
        _ => term,
    }
}

fn combine_kind(
    kind: Inst8Kind,
    facts: &HashMap<Val8, super::facts::RegFact>,
    defs: &HashMap<Val8, Inst8Kind>,
) -> Inst8Kind {
    let const_of = |r: Val8| const_fact(facts, r);
    let is_bool = |r: Val8| is_bool_fact(facts, r);

    match kind {
        Inst8Kind::Add32Byte { lhs, rhs, lane } => {
            if word_prefix_is_zero(rhs, lane, facts) {
                Inst8Kind::Copy(lhs.byte(lane))
            } else if word_prefix_is_zero(lhs, lane, facts) {
                Inst8Kind::Copy(rhs.byte(lane))
            } else {
                kind
            }
        }
        Inst8Kind::Sub32Byte { lhs, rhs, lane } => {
            if lhs == rhs {
                imm_kind(0)
            } else if word_prefix_is_zero(rhs, lane, facts) {
                Inst8Kind::Copy(lhs.byte(lane))
            } else {
                kind
            }
        }
        Inst8Kind::Sub32Borrow { lhs, rhs } => {
            if lhs == rhs || word_prefix_is_zero(rhs, 3, facts) {
                imm_kind(0)
            } else {
                kind
            }
        }
        Inst8Kind::Add(lhs, rhs) => match (const_of(lhs), const_of(rhs)) {
            (Some(0), _) => Inst8Kind::Copy(rhs),
            (_, Some(0)) => Inst8Kind::Copy(lhs),
            _ => kind,
        },
        Inst8Kind::Carry(lhs, rhs) => match (const_of(lhs), const_of(rhs)) {
            (Some(0), _) | (_, Some(0)) => imm_kind(0),
            _ if lhs == rhs && matches!(const_of(lhs), Some(v) if v < 128) => imm_kind(0),
            _ => kind,
        },
        Inst8Kind::Sub(lhs, rhs) => match (const_of(lhs), const_of(rhs)) {
            (_, Some(0)) => Inst8Kind::Copy(lhs),
            _ if lhs == rhs => imm_kind(0),
            _ => kind,
        },
        Inst8Kind::MulLo(lhs, rhs) => match (const_of(lhs), const_of(rhs)) {
            (Some(0), _) | (_, Some(0)) => imm_kind(0),
            (Some(1), _) => Inst8Kind::Copy(rhs),
            (_, Some(1)) => Inst8Kind::Copy(lhs),
            _ => kind,
        },
        Inst8Kind::MulHi(lhs, rhs) => match (const_of(lhs), const_of(rhs)) {
            (Some(0), _) | (_, Some(0)) | (Some(1), _) | (_, Some(1)) => imm_kind(0),
            _ => kind,
        },
        Inst8Kind::And8(lhs, rhs) if lhs == rhs => Inst8Kind::Copy(lhs),
        Inst8Kind::Or8(lhs, rhs) if lhs == rhs => Inst8Kind::Copy(lhs),
        Inst8Kind::Xor8(lhs, rhs) => match (const_of(lhs), const_of(rhs)) {
            (Some(0), _) => Inst8Kind::Copy(rhs),
            (_, Some(0)) => Inst8Kind::Copy(lhs),
            _ if lhs == rhs => imm_kind(0),
            _ => kind,
        },
        Inst8Kind::BoolNot(src) => match defs.get(&src).copied() {
            Some(Inst8Kind::BoolNot(inner)) => Inst8Kind::Copy(inner),
            Some(Inst8Kind::Eq(lhs, rhs)) => Inst8Kind::Ne(lhs, rhs),
            Some(Inst8Kind::Ne(lhs, rhs)) => Inst8Kind::Eq(lhs, rhs),
            Some(Inst8Kind::LtU(lhs, rhs)) => Inst8Kind::GeU(lhs, rhs),
            Some(Inst8Kind::GeU(lhs, rhs)) => Inst8Kind::LtU(lhs, rhs),
            _ => kind,
        },
        Inst8Kind::Sel(cond, if_true, if_false) if is_bool(cond) => {
            let (cond, if_true, if_false) = match resolve_bool_view(cond, facts, defs) {
                Some((base, true)) => (base, if_false, if_true),
                Some((base, false)) => (base, if_true, if_false),
                None => (cond, if_true, if_false),
            };

            if (cond == if_true && matches!(const_of(if_false), Some(0)))
                || (cond == if_false && matches!(const_of(if_true), Some(1)))
            {
                Inst8Kind::Copy(cond)
            } else if cond == if_false && matches!(const_of(if_true), Some(0)) {
                imm_kind(0)
            } else if cond == if_true && matches!(const_of(if_false), Some(1)) {
                imm_kind(1)
            } else {
                Inst8Kind::Sel(cond, if_true, if_false)
            }
        }
        _ => kind,
    }
}

fn resolve_bool_view(
    reg: Val8,
    facts: &HashMap<Val8, super::facts::RegFact>,
    defs: &HashMap<Val8, Inst8Kind>,
) -> Option<(Val8, bool)> {
    if !is_bool_fact(facts, reg) {
        return None;
    }

    let mut cur = reg;
    let mut inverted = false;
    let mut seen = HashSet::new();

    while seen.insert(cur) {
        let Some(kind) = defs.get(&cur).copied() else {
            return Some((cur, inverted));
        };

        match kind {
            Inst8Kind::Copy(src) if is_bool_fact(facts, src) => {
                cur = src;
            }
            Inst8Kind::BoolNot(src) if is_bool_fact(facts, src) => {
                cur = src;
                inverted = !inverted;
            }
            Inst8Kind::Ne(lhs, rhs) => {
                if let Some((inner, flip)) = ne_bool_inner(lhs, rhs, facts) {
                    cur = inner;
                    inverted ^= flip;
                } else {
                    return Some((cur, inverted));
                }
            }
            _ => return Some((cur, inverted)),
        }
    }

    Some((cur, inverted))
}

fn word_prefix_is_zero(word: Word, lane: u8, facts: &HashMap<Val8, super::facts::RegFact>) -> bool {
    word.bytes()
        .into_iter()
        .take(usize::from(lane) + 1)
        .all(|byte| matches!(const_fact(facts, byte), Some(0)))
}

fn ne_bool_inner(
    lhs: Val8,
    rhs: Val8,
    facts: &HashMap<Val8, super::facts::RegFact>,
) -> Option<(Val8, bool)> {
    let const_of = |r: Val8| const_fact(facts, r);

    if is_bool_fact(facts, lhs) {
        match const_of(rhs) {
            Some(0) => return Some((lhs, false)),
            Some(1) => return Some((lhs, true)),
            _ => {}
        }
    }
    if is_bool_fact(facts, rhs) {
        match const_of(lhs) {
            Some(0) => return Some((rhs, false)),
            Some(1) => return Some((rhs, true)),
            _ => {}
        }
    }

    None
}
