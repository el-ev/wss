use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BoolOp {
    And,
    Or,
}

#[derive(Clone, Copy)]
struct DefLoc {
    kind: Inst8Kind,
}

pub(super) fn bool_op_and_operands(kind: Inst8Kind) -> Option<(BoolOp, Vec<Val8>)> {
    match kind {
        Inst8Kind::BoolAnd(op) => Some((BoolOp::And, op.as_slice().to_vec())),
        Inst8Kind::BoolOr(op) => Some((BoolOp::Or, op.as_slice().to_vec())),
        _ => None,
    }
}

pub(super) fn make_bool_kind(op: BoolOp, regs: &[Val8]) -> Option<Inst8Kind> {
    match regs {
        [] => None,
        [reg] => Some(Inst8Kind::Copy(*reg)),
        _ => {
            let nary = crate::ir8::BoolNary8::from_regs(regs)?;
            Some(match op {
                BoolOp::And => Inst8Kind::BoolAnd(nary),
                BoolOp::Or => Inst8Kind::BoolOr(nary),
            })
        }
    }
}

pub(super) fn combine_boolean_chains(prog: &mut Ir8Program) -> bool {
    let mut changed = false;
    for blocks in &mut prog.func_blocks {
        changed |= combine_boolean_chains_func(blocks);
    }
    changed
}

fn combine_boolean_chains_func(blocks: &mut [BasicBlock8]) -> bool {
    if blocks.is_empty() {
        return false;
    }

    let def_counts = collect_def_counts(blocks);
    let use_counts = collect_use_counts(blocks);

    let mut defs: HashMap<Val8, DefLoc> = HashMap::new();
    for bb in blocks.iter() {
        for inst in &bb.insts {
            let Some(dst) = inst.dst else {
                continue;
            };
            if def_counts.get(&dst).copied().unwrap_or(0) != 1 {
                continue;
            }
            if bool_op_and_operands(inst.kind).is_some() {
                defs.insert(dst, DefLoc { kind: inst.kind });
            }
        }
    }

    if defs.is_empty() {
        return false;
    }

    let mut changed = false;
    let mut drop_defs: HashSet<Val8> = HashSet::new();

    for bb in blocks.iter_mut() {
        for inst in &mut bb.insts {
            let Some(dst) = inst.dst else {
                continue;
            };
            let old = inst.kind;
            let Some((op, root_ops)) = bool_op_and_operands(old) else {
                continue;
            };

            let mut leaves = Vec::new();
            let mut consumed = Vec::new();
            let mut seen = HashSet::new();
            for reg in root_ops.clone() {
                collect_bool_leaves(
                    reg,
                    op,
                    dst,
                    &defs,
                    &use_counts,
                    &mut seen,
                    &mut leaves,
                    &mut consumed,
                );
            }

            let Some(new_kind) = make_bool_kind(op, &leaves) else {
                continue;
            };
            if new_kind == old {
                continue;
            }

            inst.kind = new_kind;
            for reg in consumed {
                drop_defs.insert(reg);
            }
            changed = true;
        }
    }

    if !drop_defs.is_empty() {
        for bb in blocks.iter_mut() {
            let old_len = bb.insts.len();
            bb.insts.retain(|inst| match inst.dst {
                Some(dst) => !drop_defs.contains(&dst),
                None => true,
            });
            changed |= bb.insts.len() != old_len;
        }
    }

    changed
}

#[allow(clippy::too_many_arguments)]
fn collect_bool_leaves(
    reg: Val8,
    op: BoolOp,
    root_dst: Val8,
    defs: &HashMap<Val8, DefLoc>,
    use_counts: &HashMap<Val8, usize>,
    seen: &mut HashSet<Val8>,
    leaves: &mut Vec<Val8>,
    consumed: &mut Vec<Val8>,
) {
    if !seen.insert(reg) {
        leaves.push(reg);
        return;
    }

    let Some(def_loc) = defs.get(&reg).copied() else {
        leaves.push(reg);
        return;
    };

    if reg == root_dst || use_counts.get(&reg).copied().unwrap_or(0) != 1 {
        leaves.push(reg);
        return;
    }

    let Some((inner_op, ops)) = bool_op_and_operands(def_loc.kind) else {
        leaves.push(reg);
        return;
    };
    if inner_op != op {
        leaves.push(reg);
        return;
    }

    consumed.push(reg);
    for reg in ops {
        collect_bool_leaves(reg, op, root_dst, defs, use_counts, seen, leaves, consumed);
    }
}
