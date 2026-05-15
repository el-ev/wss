use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RegFact {
    Const(u8),
    Bool,
}

#[inline]
pub(super) fn imm_kind(v: u8) -> Inst8Kind {
    Inst8Kind::Copy(Val8::imm(v))
}

#[derive(Clone, Copy)]
enum BoolEdge {
    AnyBool(Val8, Val8),
    AllBool(Val8, Val8),
}

pub(super) fn build_reg_facts(blocks: &[BasicBlock8]) -> HashMap<Val8, RegFact> {
    let def_counts = collect_def_counts(blocks);
    let used_before_def = collect_regs_used_before_def(blocks);
    let mut facts: HashMap<Val8, RegFact> = HashMap::new();
    let mut copy_edges: Vec<(Val8, Val8)> = Vec::new();
    let mut bool_edges: Vec<(Val8, BoolEdge)> = Vec::new();

    for bb in blocks {
        for inst in &bb.insts {
            let Some(dst) = inst.dst else {
                continue;
            };
            if def_counts.get(&dst).copied().unwrap_or(0) != 1 || used_before_def.contains(&dst) {
                continue;
            }

            match inst.kind {
                Inst8Kind::Copy(src) => {
                    if let Some(v) = src.imm_value() {
                        facts.insert(dst, RegFact::Const(v));
                    } else if !used_before_def.contains(&src) {
                        copy_edges.push((dst, src));
                    }
                }
                Inst8Kind::Carry(_, _)
                | Inst8Kind::Sub32Borrow { .. }
                | Inst8Kind::Eq(_, _)
                | Inst8Kind::Ne(_, _)
                | Inst8Kind::LtU(_, _)
                | Inst8Kind::GeU(_, _)
                | Inst8Kind::BoolAnd(_)
                | Inst8Kind::BoolOr(_)
                | Inst8Kind::BoolNot(_) => {
                    facts.insert(dst, RegFact::Bool);
                }
                Inst8Kind::And8(l, r) => {
                    bool_edges.push((dst, BoolEdge::AnyBool(l, r)));
                }
                Inst8Kind::Or8(l, r) | Inst8Kind::Xor8(l, r) => {
                    bool_edges.push((dst, BoolEdge::AllBool(l, r)));
                }
                Inst8Kind::Sel(_, t, f) => {
                    bool_edges.push((dst, BoolEdge::AllBool(t, f)));
                }
                _ => {}
            }
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for &(dst, src) in &copy_edges {
            if facts.contains_key(&dst) {
                continue;
            }
            let Some(&src_fact) = facts.get(&src) else {
                continue;
            };
            facts.insert(dst, src_fact);
            changed = true;
        }
        for &(dst, edge) in &bool_edges {
            if facts.contains_key(&dst) {
                continue;
            }
            let is_bool = match edge {
                BoolEdge::AnyBool(a, b) => is_bool_fact(&facts, a) || is_bool_fact(&facts, b),
                BoolEdge::AllBool(a, b) => is_bool_fact(&facts, a) && is_bool_fact(&facts, b),
            };
            if is_bool {
                facts.insert(dst, RegFact::Bool);
                changed = true;
            }
        }
    }

    facts
}

pub(super) fn const_fact(facts: &HashMap<Val8, RegFact>, val: Val8) -> Option<u8> {
    if let Some(v) = val.imm_value() {
        return Some(v);
    }
    match facts.get(&val) {
        Some(RegFact::Const(v)) => Some(*v),
        _ => None,
    }
}

pub(super) fn is_bool_fact(facts: &HashMap<Val8, RegFact>, val: Val8) -> bool {
    if let Some(v) = val.imm_value() {
        return v <= 1;
    }
    match facts.get(&val) {
        Some(RegFact::Bool) => true,
        Some(RegFact::Const(v)) => *v <= 1,
        None => false,
    }
}
