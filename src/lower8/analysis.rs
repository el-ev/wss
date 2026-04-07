use super::*;
use std::collections::HashSet;

fn use_if_non_imm(value_ref: IrNode) -> Vec<IrNode> {
    if value_ref.is_imm() {
        Vec::new()
    } else {
        vec![value_ref]
    }
}

fn push_if_non_imm(out: &mut Vec<IrNode>, value_ref: IrNode) {
    if !value_ref.is_imm() {
        out.push(value_ref);
    }
}

fn collect_non_imm_uses(values: impl IntoIterator<Item = IrNode>) -> Vec<IrNode> {
    values
        .into_iter()
        .filter(|value_ref| !value_ref.is_imm())
        .collect()
}

pub(super) fn inst_uses(inst: &Inst) -> Vec<IrNode> {
    match inst {
        // TODO(i64): liveness model keys off an i32-only IR opcode set today.
        Inst::I32Const(_)
        | Inst::LocalGet(_)
        | Inst::GlobalGet(_)
        | Inst::MemorySize
        | Inst::TableSize(_)
        | Inst::Getchar
        | Inst::Drop => Vec::new(),
        Inst::LocalSet(_, value_ref)
        | Inst::LocalTee(_, value_ref)
        | Inst::GlobalSet(_, value_ref)
        | Inst::Unary(_, value_ref)
        | Inst::Putchar(value_ref)
        | Inst::Load {
            addr: value_ref, ..
        } => use_if_non_imm(*value_ref),
        Inst::Binary(_, lhs, rhs)
        | Inst::Compare(_, lhs, rhs)
        | Inst::Store {
            addr: lhs,
            val: rhs,
            ..
        } => {
            let mut out = Vec::with_capacity(2);
            push_if_non_imm(&mut out, *lhs);
            push_if_non_imm(&mut out, *rhs);
            out
        }
        Inst::Select {
            cond,
            if_true,
            if_false,
        } => vec![*cond, *if_true, *if_false],
        Inst::Call { args, .. } => args.clone(),
        Inst::CallIndirect { index, args, .. } => {
            let mut out = Vec::with_capacity(args.len() + 1);
            out.push(*index);
            out.extend(args.iter().copied());
            out
        }
    }
}

pub(super) fn term_uses(term: &Terminator) -> Vec<IrNode> {
    match term {
        Terminator::Goto(_) | Terminator::Unreachable => Vec::new(),
        Terminator::Branch { cond, .. } => use_if_non_imm(*cond),
        Terminator::Switch { index, .. } => use_if_non_imm(*index),
        Terminator::TailCall { args, .. } => collect_non_imm_uses(args.iter().copied()),
        Terminator::TailCallIndirect { index, args, .. } => {
            let mut out = Vec::with_capacity(args.len() + 1);
            push_if_non_imm(&mut out, *index);
            out.extend(collect_non_imm_uses(args.iter().copied()));
            out
        }
        Terminator::Return(value_ref) => collect_non_imm_uses(value_ref.iter().copied()),
    }
}

pub(super) fn collect_spill_words(
    live_after: &[IrNode],
    inst_map: &HashMap<IrNode, Word>,
    local_vregs: &[Word],
) -> Vec<Word> {
    let locals: HashSet<Word> = local_vregs.iter().copied().collect();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for iref in live_after {
        if let Some(w) = inst_map.get(iref).copied() {
            if locals.contains(&w) {
                continue;
            }
            if seen.insert(w) {
                out.push(w);
            }
        }
    }
    out
}

pub(super) fn compute_live_after_by_block(
    body: &crate::module::IrFuncBody,
) -> Vec<Vec<Vec<IrNode>>> {
    let mut block_idx_by_id = HashMap::new();
    for (i, blk) in body.blocks().iter().enumerate() {
        block_idx_by_id.insert(blk.id, i);
    }

    let mut block_defs = vec![HashSet::new(); body.blocks().len()];
    let mut block_uses = vec![HashSet::new(); body.blocks().len()];
    for (block_idx, ir_block) in body.blocks().iter().enumerate() {
        let ref_base = BasicBlock::ref_base(body.blocks(), block_idx);
        let mut seen_defs = HashSet::new();
        for (i, inst) in ir_block.insts.iter().enumerate() {
            for u in inst_uses(inst) {
                if !seen_defs.contains(&u) {
                    block_uses[block_idx].insert(u);
                }
            }
            let def = ref_base + i;
            seen_defs.insert(def);
            block_defs[block_idx].insert(def);
        }
        for u in term_uses(&ir_block.terminator) {
            if !seen_defs.contains(&u) {
                block_uses[block_idx].insert(u);
            }
        }
    }

    let mut live_in = vec![HashSet::new(); body.blocks().len()];
    let mut live_out = vec![HashSet::new(); body.blocks().len()];
    loop {
        let mut changed = false;
        for (block_idx, ir_block) in body.blocks().iter().enumerate().rev() {
            let mut out_new = HashSet::new();
            for succ in ir_block.successors() {
                if let Some(&succ_idx) = block_idx_by_id.get(&succ) {
                    out_new.extend(live_in[succ_idx].iter().copied());
                }
            }
            let mut in_new = block_uses[block_idx].clone();
            in_new.extend(
                out_new
                    .iter()
                    .filter(|r| !block_defs[block_idx].contains(r))
                    .copied(),
            );
            if out_new != live_out[block_idx] {
                live_out[block_idx] = out_new;
                changed = true;
            }
            if in_new != live_in[block_idx] {
                live_in[block_idx] = in_new;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }

    let mut live_after_by_block = vec![Vec::new(); body.blocks().len()];
    for (block_idx, ir_block) in body.blocks().iter().enumerate() {
        let ref_base = BasicBlock::ref_base(body.blocks(), block_idx);
        let mut live = live_out[block_idx].clone();
        for u in term_uses(&ir_block.terminator) {
            live.insert(u);
        }
        let mut live_after = vec![Vec::new(); ir_block.insts.len()];
        for i in (0..ir_block.insts.len()).rev() {
            let mut here = live.iter().copied().collect::<Vec<_>>();
            here.sort_unstable();
            live_after[i] = here;
            let def = ref_base + i;
            live.remove(&def);
            for u in inst_uses(&ir_block.insts[i]) {
                live.insert(u);
            }
        }
        live_after_by_block[block_idx] = live_after;
    }

    live_after_by_block
}
