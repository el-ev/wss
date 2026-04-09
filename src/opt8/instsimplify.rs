use super::bool::{BoolOp, make_bool_kind};
use super::facts::{RegFact, build_reg_facts, const_fact, imm_kind, is_bool_fact};
use super::*;
use crate::ir8::BoolNary8;

#[derive(Default)]
pub(super) struct InstSimplify {
    facts: HashMap<Val8, RegFact>,
}

impl InstSimplify {
    pub(super) fn run(&mut self, blocks: &mut [BasicBlock8]) -> bool {
        if blocks.is_empty() {
            self.facts.clear();
            return false;
        }

        self.facts = build_reg_facts(blocks);
        simplify_blocks_with_facts(blocks, &self.facts)
    }
}

fn simplify_blocks_with_facts(blocks: &mut [BasicBlock8], facts: &HashMap<Val8, RegFact>) -> bool {
    let mut changed = false;

    for bb in blocks.iter_mut() {
        for inst in &mut bb.insts {
            let new_kind = simplify_kind(inst.kind, facts);
            if new_kind != inst.kind {
                inst.kind = new_kind;
                changed = true;
            }
        }

        let old_len = bb.insts.len();
        bb.insts
            .retain(|inst| !matches!(inst.kind, Inst8Kind::Copy(src) if inst.dst == Some(src)));
        changed |= bb.insts.len() != old_len;

        let old_term = bb.terminator.clone();
        bb.terminator = simplify_term(old_term.clone(), facts);
        if bb.terminator != old_term {
            changed = true;
        }
    }

    changed
}

fn simplify_bool_nary(op: BoolOp, vals: &BoolNary8, facts: &HashMap<Val8, RegFact>) -> Inst8Kind {
    let const_of = |val: Val8| const_fact(facts, val);

    let mut kept = Vec::with_capacity(usize::from(vals.len));
    let mut seen = HashSet::new();
    for &val in vals.as_slice() {
        let cst = const_of(val);
        match op {
            BoolOp::And => {
                if matches!(cst, Some(0)) {
                    return imm_kind(0);
                }
                if cst.is_some() {
                    continue;
                }
            }
            BoolOp::Or => {
                if matches!(cst, Some(v) if v != 0) {
                    return imm_kind(1);
                }
                if matches!(cst, Some(0)) {
                    continue;
                }
            }
        }
        if seen.insert(val) {
            kept.push(val);
        }
    }

    if kept.is_empty() {
        return imm_kind(match op {
            BoolOp::And => 1,
            BoolOp::Or => 0,
        });
    }

    make_bool_kind(op, &kept).expect("non-empty bool op inputs should fit IR8 nary limit")
}

fn const_word_prefix(word: Word, lane: u8, facts: &HashMap<Val8, RegFact>) -> Option<u32> {
    let mut out = 0u32;
    for (idx, byte) in word
        .bytes()
        .into_iter()
        .enumerate()
        .take(usize::from(lane) + 1)
    {
        let value = const_fact(facts, byte)?;
        out |= u32::from(value) << (idx * 8);
    }
    Some(out)
}

fn simplify_kind(kind: Inst8Kind, facts: &HashMap<Val8, RegFact>) -> Inst8Kind {
    let const_of = |r: Val8| const_fact(facts, r);
    let is_bool = |r: Val8| is_bool_fact(facts, r);

    match kind {
        Inst8Kind::Getchar | Inst8Kind::GlobalGetByte { .. } => kind,
        Inst8Kind::Copy(_) => kind,
        Inst8Kind::Add32Byte { lhs, rhs, lane } => {
            match (
                const_word_prefix(lhs, lane, facts),
                const_word_prefix(rhs, lane, facts),
            ) {
                (Some(a), Some(b)) => {
                    imm_kind((((u64::from(a) + u64::from(b)) >> (lane * 8)) & 0xff) as u8)
                }
                _ => kind,
            }
        }
        Inst8Kind::Sub32Byte { lhs, rhs, lane } => {
            match (
                const_word_prefix(lhs, lane, facts),
                const_word_prefix(rhs, lane, facts),
            ) {
                (Some(a), Some(b)) => {
                    let bits = 8 * (u32::from(lane) + 1);
                    let modulus = 1i128 << bits;
                    let diff = (i128::from(a) - i128::from(b)).rem_euclid(modulus);
                    imm_kind(((diff >> (u32::from(lane) * 8)) & 0xff) as u8)
                }
                _ => kind,
            }
        }
        Inst8Kind::Sub32Borrow { lhs, rhs } => match (
            const_word_prefix(lhs, 3, facts),
            const_word_prefix(rhs, 3, facts),
        ) {
            (Some(a), Some(b)) => imm_kind((a < b) as u8),
            _ => kind,
        },
        Inst8Kind::Add(l, r) => match (const_of(l), const_of(r)) {
            (Some(a), Some(b)) => imm_kind(a.wrapping_add(b)),
            _ => kind,
        },
        Inst8Kind::Carry(l, r) => match (const_of(l), const_of(r)) {
            (Some(a), Some(b)) => imm_kind((((a as u16) + (b as u16)) >> 8) as u8),
            _ => kind,
        },
        Inst8Kind::Sub(l, r) => match (const_of(l), const_of(r)) {
            (Some(a), Some(b)) => imm_kind(a.wrapping_sub(b)),
            _ => kind,
        },
        Inst8Kind::MulLo(l, r) => match (const_of(l), const_of(r)) {
            (Some(a), Some(b)) => imm_kind(((a as u16 * b as u16) & 0xff) as u8),
            _ => kind,
        },
        Inst8Kind::MulHi(l, r) => match (const_of(l), const_of(r)) {
            (Some(a), Some(b)) => imm_kind(((a as u16 * b as u16) >> 8) as u8),
            _ => kind,
        },
        Inst8Kind::And8(l, r) => simplify_and8(facts, l, r),
        Inst8Kind::Or8(l, r) => simplify_or8(facts, l, r),
        Inst8Kind::Xor8(l, r) => match (const_of(l), const_of(r)) {
            (Some(a), Some(b)) => imm_kind(a ^ b),
            _ => kind,
        },
        Inst8Kind::Eq(l, r) => {
            if l == r {
                return imm_kind(1);
            }
            match (const_of(l), const_of(r)) {
                (Some(a), Some(b)) => imm_kind((a == b) as u8),
                (Some(0), _) if is_bool(r) => Inst8Kind::BoolNot(r),
                (Some(1), _) if is_bool(r) => Inst8Kind::Copy(r),
                (_, Some(0)) if is_bool(l) => Inst8Kind::BoolNot(l),
                (_, Some(1)) if is_bool(l) => Inst8Kind::Copy(l),
                _ => kind,
            }
        }
        Inst8Kind::Ne(l, r) => {
            if l == r {
                return imm_kind(0);
            }
            match (const_of(l), const_of(r)) {
                (Some(a), Some(b)) => imm_kind((a != b) as u8),
                (Some(0), _) if is_bool(r) => Inst8Kind::Copy(r),
                (Some(1), _) if is_bool(r) => Inst8Kind::BoolNot(r),
                (_, Some(0)) if is_bool(l) => Inst8Kind::Copy(l),
                (_, Some(1)) if is_bool(l) => Inst8Kind::BoolNot(l),
                _ => kind,
            }
        }
        Inst8Kind::LtU(l, r) => match (const_of(l), const_of(r)) {
            _ if l == r => imm_kind(0),
            (Some(a), Some(b)) => imm_kind((a < b) as u8),
            _ => kind,
        },
        Inst8Kind::GeU(l, r) => match (const_of(l), const_of(r)) {
            _ if l == r => imm_kind(1),
            (Some(a), Some(b)) => imm_kind((a >= b) as u8),
            _ => kind,
        },
        Inst8Kind::BoolAnd(op) => simplify_bool_nary(BoolOp::And, &op, facts),
        Inst8Kind::BoolOr(op) => simplify_bool_nary(BoolOp::Or, &op, facts),
        Inst8Kind::BoolNot(v) => match const_of(v) {
            Some(c) => imm_kind((c == 0) as u8),
            None => kind,
        },
        Inst8Kind::Sel(c, t, f) => match const_of(c) {
            Some(0) => Inst8Kind::Copy(f),
            Some(_) => Inst8Kind::Copy(t),
            None => {
                if t == f {
                    Inst8Kind::Copy(t)
                } else if matches!(const_of(t), Some(1))
                    && matches!(const_of(f), Some(0))
                    && is_bool(c)
                {
                    Inst8Kind::Copy(c)
                } else if matches!(const_of(t), Some(0))
                    && matches!(const_of(f), Some(1))
                    && is_bool(c)
                {
                    Inst8Kind::BoolNot(c)
                } else {
                    kind
                }
            }
        },
        Inst8Kind::GlobalSetByte {
            global_idx,
            lane,
            val,
        } => Inst8Kind::GlobalSetByte {
            global_idx,
            lane,
            val,
        },
        Inst8Kind::LoadMem { base, addr, lane } => Inst8Kind::LoadMem { base, addr, lane },
        Inst8Kind::StoreMem {
            base,
            addr,
            lane,
            val,
        } => Inst8Kind::StoreMem {
            base,
            addr,
            lane,
            val,
        },
        Inst8Kind::Putchar(v) => Inst8Kind::Putchar(v),
        Inst8Kind::CsStore { offset, val } => Inst8Kind::CsStore { offset, val },
        Inst8Kind::CsLoad { offset } => Inst8Kind::CsLoad { offset },
        Inst8Kind::CsStorePc { offset, val } => Inst8Kind::CsStorePc { offset, val },
        Inst8Kind::CsLoadPc { offset } => Inst8Kind::CsLoadPc { offset },
        Inst8Kind::CsAlloc(n) => Inst8Kind::CsAlloc(n),
        Inst8Kind::CsFree(n) => Inst8Kind::CsFree(n),
    }
}

fn simplify_term(term: Terminator8, facts: &HashMap<Val8, RegFact>) -> Terminator8 {
    match term {
        Terminator8::Branch {
            cond,
            if_true,
            if_false,
        } => {
            if if_true == if_false {
                return Terminator8::Goto(if_true);
            }
            match const_fact(facts, cond) {
                Some(0) => Terminator8::Goto(if_false),
                Some(_) => Terminator8::Goto(if_true),
                None => Terminator8::Branch {
                    cond,
                    if_true,
                    if_false,
                },
            }
        }
        Terminator8::Switch {
            index,
            targets,
            default,
        } => match const_fact(facts, index) {
            Some(v) => {
                let idx = usize::from(v);
                let target = targets.get(idx).copied().unwrap_or(default);
                Terminator8::Goto(target)
            }
            None => {
                if targets.iter().all(|&pc| pc == default) {
                    Terminator8::Goto(default)
                } else {
                    Terminator8::Switch {
                        index,
                        targets,
                        default,
                    }
                }
            }
        },
        _ => term,
    }
}

fn simplify_or8(facts: &HashMap<Val8, RegFact>, lhs: Val8, rhs: Val8) -> Inst8Kind {
    let const_of = |r: Val8| const_fact(facts, r);
    match (const_of(lhs), const_of(rhs)) {
        (Some(a), Some(b)) => imm_kind(a | b),
        (Some(0), _) => Inst8Kind::Copy(rhs),
        (_, Some(0)) => Inst8Kind::Copy(lhs),
        (Some(0xff), _) | (_, Some(0xff)) => imm_kind(0xff),
        _ => Inst8Kind::Or8(lhs, rhs),
    }
}

fn simplify_and8(facts: &HashMap<Val8, RegFact>, lhs: Val8, rhs: Val8) -> Inst8Kind {
    let const_of = |r: Val8| const_fact(facts, r);
    match (const_of(lhs), const_of(rhs)) {
        (Some(a), Some(b)) => imm_kind(a & b),
        (Some(0), _) | (_, Some(0)) => imm_kind(0),
        (Some(0xff), _) => Inst8Kind::Copy(rhs),
        (_, Some(0xff)) => Inst8Kind::Copy(lhs),
        _ => Inst8Kind::And8(lhs, rhs),
    }
}
