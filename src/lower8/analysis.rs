use super::*;
use std::collections::HashSet;

pub(super) fn inst_uses(inst: &Inst) -> Vec<IrNode> {
    match inst {
        // TODO(i64): liveness model keys off an i32-only IR opcode set today.
        Inst::I32Const(_)
        | Inst::I64Const(_)
        | Inst::LocalGet(_)
        | Inst::GlobalGet(_)
        | Inst::MemorySize
        | Inst::TableSize(_)
        | Inst::Getchar
        | Inst::Drop
        | Inst::ExcSet { .. }
        | Inst::ExcClear
        | Inst::ExcFlagGet
        | Inst::ExcTagGet
        | Inst::ExcPayloadGet => Vec::new(),
        Inst::LocalSet(_, value_ref)
        | Inst::LocalTee(_, value_ref)
        | Inst::GlobalSet(_, value_ref)
        | Inst::Unary { val: value_ref, .. }
        | Inst::Putchar(value_ref)
        | Inst::Load {
            addr: value_ref, ..
        }
        | Inst::ExcPayloadSet(value_ref) => vec![*value_ref],
        Inst::Binary { lhs, rhs, .. }
        | Inst::Compare { lhs, rhs, .. }
        | Inst::Store {
            addr: lhs,
            val: rhs,
            ..
        } => {
            vec![*lhs, *rhs]
        }
        Inst::Select {
            cond,
            if_true,
            if_false,
            ..
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
        Terminator::Goto(_) | Terminator::Unreachable | Terminator::UncaughtExit => Vec::new(),
        Terminator::Branch { cond, .. } => vec![*cond],
        Terminator::Switch { index, .. } => vec![*index],
        Terminator::TailCall { args, .. } => args.clone(),
        Terminator::TailCallIndirect { index, args, .. } => {
            let mut out = Vec::with_capacity(args.len() + 1);
            out.push(*index);
            out.extend(args.iter().copied());
            out
        }
        Terminator::Return(value_ref) => value_ref.iter().copied().collect(),
    }
}

pub(super) fn collect_spill_words(
    live_after: &[IrNode],
    inst_map: &HashMap<IrNode, ValueWords>,
    local_vregs: &[ValueWords],
) -> Vec<Word> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for iref in live_after {
        if let Some(value) = inst_map.get(iref).copied() {
            if local_vregs.iter().any(|l| l == &value) {
                continue;
            }
            if seen.insert(value.lo) {
                out.push(value.lo);
            }
            if let Some(hi) = value.hi
                && seen.insert(hi)
            {
                out.push(hi);
            }
        }
    }
    out
}

pub(super) fn compute_local_live_after_by_block(
    body: &crate::module::IrFuncBody,
) -> Vec<Vec<Vec<u32>>> {
    let mut block_idx_by_id = HashMap::new();
    for (i, blk) in body.blocks().iter().enumerate() {
        block_idx_by_id.insert(blk.id, i);
    }

    let mut block_uses: Vec<HashSet<u32>> = vec![HashSet::new(); body.blocks().len()];
    let mut block_kills: Vec<HashSet<u32>> = vec![HashSet::new(); body.blocks().len()];
    for (block_idx, ir_block) in body.blocks().iter().enumerate() {
        let mut killed: HashSet<u32> = HashSet::new();
        for inst in &ir_block.insts {
            match inst {
                Inst::LocalGet(l) if !killed.contains(l) => {
                    block_uses[block_idx].insert(*l);
                }
                Inst::LocalSet(l, _) | Inst::LocalTee(l, _) => {
                    killed.insert(*l);
                }
                _ => {}
            }
        }
        block_kills[block_idx] = killed;
    }

    let mut live_in: Vec<HashSet<u32>> = vec![HashSet::new(); body.blocks().len()];
    let mut live_out: Vec<HashSet<u32>> = vec![HashSet::new(); body.blocks().len()];
    loop {
        let mut changed = false;
        for (block_idx, ir_block) in body.blocks().iter().enumerate().rev() {
            let mut out_new: HashSet<u32> = HashSet::new();
            for succ in ir_block.successors() {
                if let Some(&succ_idx) = block_idx_by_id.get(&succ) {
                    out_new.extend(live_in[succ_idx].iter().copied());
                }
            }
            let mut in_new = block_uses[block_idx].clone();
            in_new.extend(
                out_new
                    .iter()
                    .filter(|l| !block_kills[block_idx].contains(l))
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
        let mut live = live_out[block_idx].clone();
        let mut per_inst = vec![Vec::new(); ir_block.insts.len()];
        for i in (0..ir_block.insts.len()).rev() {
            let mut here: Vec<u32> = live.iter().copied().collect();
            here.sort_unstable();
            per_inst[i] = here;
            match &ir_block.insts[i] {
                Inst::LocalGet(l) => {
                    live.insert(*l);
                }
                Inst::LocalSet(l, _) | Inst::LocalTee(l, _) => {
                    live.remove(l);
                }
                _ => {}
            }
        }
        live_after_by_block[block_idx] = per_inst;
    }

    live_after_by_block
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
