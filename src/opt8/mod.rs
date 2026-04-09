mod bool;
mod facts;
mod instcombine;
mod instsimplify;
mod rewrite;

#[cfg(test)]
mod tests;

use bool::combine_boolean_chains;
use instcombine::instcombine;
use instsimplify::InstSimplify;
use rewrite::*;

use std::collections::hash_map::DefaultHasher;
use std::collections::{HashMap, HashSet};
use std::hash::{Hash, Hasher};

fn filter_by_mask<T>(items: &mut Vec<T>, keep: &[bool]) {
    let mut i = 0;
    items.retain(|_| {
        let k = keep[i];
        i += 1;
        k
    });
}

use crate::ir8::{
    Addr, BasicBlock8, Inst8, Inst8Kind, Ir8Program, Pc, Terminator8, VREG_START, Val8, Word,
};

const MAX_OPT_RUN_ITERS: usize = 128;
const MAX_COPY_ELIM_FUNC_ITERS: usize = 64;
const PREFERRED_COPY_DST_LIMIT: u16 = VREG_START + 4;

pub fn run(prog: &mut Ir8Program) {
    let mut seen_states: HashSet<u64> = HashSet::new();
    for _ in 0..MAX_OPT_RUN_ITERS {
        let state_sig = program_state_sig(prog);
        if !seen_states.insert(state_sig) {
            break;
        }
        let mut changed = false;
        changed |= local_copy_propagation(prog);
        changed |= copy_elim(prog);
        changed |= instcombine(prog);
        changed |= combine_boolean_chains(prog);
        changed |= local_dead_mem_store_elim(prog);
        changed |= thread_empty_gotos(prog);
        changed |= dead_code_elim(prog);
        changed |= remove_unreachable_blocks(prog);
        changed |= coalesce_linear_blocks(prog);
        if !changed {
            return;
        }
    }
}

fn copy_elim(prog: &mut Ir8Program) -> bool {
    let mut changed = false;

    for func_id in 0..prog.func_blocks.len() {
        let mut inst_simplify = InstSimplify::default();
        for _ in 0..MAX_COPY_ELIM_FUNC_ITERS {
            let mut func_changed = false;
            func_changed |= eliminate_global_copies(&mut prog.func_blocks[func_id]);
            func_changed |= inst_simplify.run(&mut prog.func_blocks[func_id]);
            if !func_changed {
                break;
            }
            changed = true;
        }
    }

    changed
}

fn program_state_sig(prog: &Ir8Program) -> u64 {
    let mut hasher = DefaultHasher::new();
    prog.func_blocks.hash(&mut hasher);
    hasher.finish()
}

fn eliminate_global_copies(blocks: &mut [BasicBlock8]) -> bool {
    if blocks.is_empty() {
        return false;
    }

    let def_counts = collect_def_counts(blocks);
    let used_before_def = collect_regs_used_before_def(blocks);
    let mut subst: HashMap<Val8, Val8> = HashMap::new();

    for bb in blocks.iter() {
        for inst in &bb.insts {
            let (Some(dst), Inst8Kind::Copy(src)) = (inst.dst, inst.kind) else {
                continue;
            };
            if dst.expect_vreg() < VREG_START
                || def_counts.get(&dst).copied().unwrap_or(0) != 1
                || used_before_def.contains(&dst)
            {
                continue;
            }
            let canon = resolve(&subst, src);
            if canon == dst || used_before_def.contains(&canon) {
                continue;
            }
            if is_stable_source(canon, &def_counts) {
                if prefer_copy_dest(dst, canon) {
                    continue;
                }
                subst.insert(dst, canon);
            }
        }
    }

    let mut changed = false;
    if !subst.is_empty() {
        rewrite_blocks_with_subst(blocks, &subst);
        changed = true;
    }

    let use_counts = collect_use_counts(blocks);
    for bb in blocks.iter_mut() {
        let old_len = bb.insts.len();
        bb.insts.retain(|inst| {
            let Some(dst) = inst.dst else {
                return true;
            };
            match inst.kind {
                Inst8Kind::Copy(src) if src == dst => false,
                Inst8Kind::Copy(_)
                    if dst.expect_vreg() >= VREG_START
                        && use_counts.get(&dst).copied().unwrap_or(0) == 0 =>
                {
                    false
                }
                _ => true,
            }
        });
        changed |= bb.insts.len() != old_len;
    }

    changed
}

fn dead_code_elim(prog: &mut Ir8Program) -> bool {
    let mut changed = false;
    for blocks in &mut prog.func_blocks {
        changed |= dce_function(blocks);
    }
    changed
}

fn dce_function(blocks: &mut [BasicBlock8]) -> bool {
    let n = blocks.len();
    if n == 0 {
        return false;
    }

    let pc_to_idx: HashMap<Pc, usize> = blocks.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
    let succ: Vec<Vec<usize>> = blocks
        .iter()
        .map(|block| {
            block
                .terminator
                .successors()
                .into_iter()
                .filter_map(|pc| pc_to_idx.get(&pc).copied())
                .collect()
        })
        .collect();

    let mut live_in: Vec<HashSet<Val8>> = vec![HashSet::new(); n];
    let mut live_out: Vec<HashSet<Val8>> = vec![HashSet::new(); n];

    let mut changed = true;
    while changed {
        changed = false;
        for i in (0..n).rev() {
            let mut out = HashSet::new();
            for &to in &succ[i] {
                out.extend(live_in[to].iter().copied());
            }

            let mut live = out.clone();
            term_uses(&blocks[i].terminator, &mut live);
            for inst in blocks[i].insts.iter().rev() {
                if let Some(dst) = inst.dst {
                    live.remove(&dst);
                }
                inst_uses(&inst.kind, &mut live);
            }

            if live_in[i] != live || live_out[i] != out {
                live_in[i] = live;
                live_out[i] = out;
                changed = true;
            }
        }
    }

    let mut any_removed = false;
    for (i, bb) in blocks.iter_mut().enumerate() {
        let mut live = live_out[i].clone();
        term_uses(&bb.terminator, &mut live);

        let mut keep = vec![true; bb.insts.len()];
        for idx in (0..bb.insts.len()).rev() {
            let inst = &bb.insts[idx];
            let side = has_side_effect(&inst.kind);

            if let Some(dst) = inst.dst {
                if !live.contains(&dst) && !side {
                    keep[idx] = false;
                    continue;
                }
                live.remove(&dst);
            } else if !side {
                keep[idx] = false;
                continue;
            }

            inst_uses(&inst.kind, &mut live);
        }

        if keep.iter().all(|&k| k) {
            continue;
        }

        any_removed = true;
        filter_by_mask(&mut bb.insts, &keep);
    }

    any_removed
}

fn remove_unreachable_blocks(prog: &mut Ir8Program) -> bool {
    let mut changed = false;

    for func_id in 0..prog.func_blocks.len() {
        let entry_pc = prog
            .func_entries
            .get(func_id)
            .copied()
            .or_else(|| prog.func_blocks[func_id].first().map(|b| b.id));

        let blocks = &mut prog.func_blocks[func_id];
        if blocks.is_empty() {
            continue;
        }

        let pc_to_idx: HashMap<Pc, usize> =
            blocks.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
        let Some(entry_idx) = entry_pc.and_then(|pc| pc_to_idx.get(&pc).copied()) else {
            continue;
        };

        let mut visited = vec![false; blocks.len()];
        let mut stack = vec![entry_idx];
        while let Some(idx) = stack.pop() {
            if visited[idx] {
                continue;
            }
            visited[idx] = true;
            for succ_pc in blocks[idx].terminator.successors() {
                if let Some(&to) = pc_to_idx.get(&succ_pc)
                    && !visited[to]
                {
                    stack.push(to);
                }
            }
        }

        if visited.iter().all(|&v| v) {
            continue;
        }

        let old_len = blocks.len();
        filter_by_mask(blocks, &visited);
        changed |= blocks.len() != old_len;
    }

    changed
}

fn coalesce_linear_blocks(prog: &mut Ir8Program) -> bool {
    let mut changed = false;

    for func_id in 0..prog.func_blocks.len() {
        let entry_pc = prog
            .func_entries
            .get(func_id)
            .copied()
            .or_else(|| prog.func_blocks[func_id].first().map(|b| b.id));

        let blocks = &mut prog.func_blocks[func_id];
        if blocks.len() <= 1 {
            continue;
        }

        loop {
            let pc_to_idx: HashMap<Pc, usize> =
                blocks.iter().enumerate().map(|(i, b)| (b.id, i)).collect();
            let mut pred_counts = vec![0usize; blocks.len()];
            for bb in blocks.iter() {
                for succ_pc in bb.terminator.successors() {
                    if let Some(&to) = pc_to_idx.get(&succ_pc) {
                        pred_counts[to] += 1;
                    }
                }
            }

            let mut merged = false;
            for i in 0..blocks.len() {
                let Terminator8::Goto(target_pc) = blocks[i].terminator else {
                    continue;
                };
                if Some(target_pc) == entry_pc {
                    continue;
                }
                let Some(&j) = pc_to_idx.get(&target_pc) else {
                    continue;
                };
                if i == j || pred_counts[j] != 1 {
                    continue;
                }

                let mut succ = blocks.remove(j);
                let pred_idx = if j < i { i - 1 } else { i };
                blocks[pred_idx].insts.append(&mut succ.insts);
                blocks[pred_idx].terminator = succ.terminator;

                changed = true;
                merged = true;
                break;
            }

            if !merged {
                break;
            }
        }
    }

    changed
}

fn local_dead_mem_store_elim(prog: &mut Ir8Program) -> bool {
    let mut changed = false;
    for blocks in &mut prog.func_blocks {
        for bb in blocks {
            changed |= local_dead_mem_store_elim_block(bb);
        }
    }
    changed
}

fn local_dead_mem_store_elim_block(bb: &mut BasicBlock8) -> bool {
    if bb.insts.len() < 2 {
        return false;
    }

    let mut tracked_addr: Option<Addr> = None;
    let mut tracked_offsets: HashSet<u32> = HashSet::new();
    let mut keep = vec![true; bb.insts.len()];
    let mut changed = false;

    for i in (0..bb.insts.len()).rev() {
        let inst = &bb.insts[i];
        match inst.kind {
            Inst8Kind::StoreMem {
                base, addr, lane, ..
            } => {
                let offset = (base as u32) + (lane as u32);
                if tracked_addr == Some(addr) && tracked_offsets.contains(&offset) {
                    keep[i] = false;
                    changed = true;
                }
                if let Some(a) = tracked_addr
                    && a != addr
                {
                    tracked_offsets.clear();
                }
                tracked_addr = Some(addr);
                tracked_offsets.insert(offset);
            }
            Inst8Kind::LoadMem { base, addr, lane } => {
                if tracked_addr == Some(addr) {
                    let offset = (base as u32) + (lane as u32);
                    tracked_offsets.remove(&offset);
                    if tracked_offsets.is_empty() {
                        tracked_addr = None;
                    }
                } else if tracked_addr.is_some() {
                    tracked_offsets.clear();
                    tracked_addr = None;
                }
            }
            _ => {}
        }

        if let Some(dst) = inst.dst
            && let Some(a) = tracked_addr
            && (dst == a.lo || dst == a.hi)
        {
            tracked_offsets.clear();
            tracked_addr = None;
        }
    }

    if !changed {
        return false;
    }

    filter_by_mask(&mut bb.insts, &keep);
    true
}

fn local_copy_propagation(prog: &mut Ir8Program) -> bool {
    let mut changed = false;
    for blocks in &mut prog.func_blocks {
        for bb in blocks.iter_mut() {
            changed |= local_copy_propagation_block(bb);
        }
    }
    changed
}

fn local_copy_propagation_block(bb: &mut BasicBlock8) -> bool {
    let mut changed = false;
    let mut subst: HashMap<Val8, Val8> = HashMap::new();

    for inst in bb.insts.iter_mut() {
        let new = rewrite_inst(inst.clone(), &subst);
        changed |= new != *inst;
        *inst = new;

        if let Some(dst) = inst.dst {
            kill_aliases_for(&mut subst, dst);
        }

        let Some(dst) = inst.dst else {
            continue;
        };
        let Inst8Kind::Copy(src) = inst.kind else {
            continue;
        };
        if dst == src {
            continue;
        }

        if prefer_copy_dest(dst, src) {
            subst.insert(src, dst);
        } else {
            subst.insert(dst, src);
        }
        compress_subst(&mut subst);
    }

    let old_term = bb.terminator.clone();
    rewrite_term(&mut bb.terminator, &subst);
    changed |= bb.terminator != old_term;

    let old_len = bb.insts.len();
    bb.insts
        .retain(|inst| !matches!(inst.kind, Inst8Kind::Copy(src) if inst.dst == Some(src)));
    changed |= bb.insts.len() != old_len;

    changed
}

fn prefer_copy_dest(dst: Val8, src: Val8) -> bool {
    if src.is_imm() {
        return false;
    }
    if dst.is_imm() {
        return true;
    }
    match (dst.reg_index(), src.reg_index()) {
        // Keep the bias narrow: we want to favor the first post-return
        // register group, not every numerically smaller temp, which tends
        // to preserve extra copies.
        (Some(dst_idx), Some(src_idx)) => dst_idx < PREFERRED_COPY_DST_LIMIT && dst_idx < src_idx,
        _ => false,
    }
}

fn kill_aliases_for(subst: &mut HashMap<Val8, Val8>, reg: Val8) {
    subst.remove(&reg);
    subst.retain(|k, v| *k != reg && *v != reg);
    compress_subst(subst);
}

fn compress_subst(subst: &mut HashMap<Val8, Val8>) {
    let updates: Vec<(Val8, Val8)> = subst
        .keys()
        .copied()
        .map(|k| (k, resolve(subst, k)))
        .collect();
    for (k, r) in updates {
        if r == k {
            subst.remove(&k);
        } else {
            subst.insert(k, r);
        }
    }
}

fn thread_empty_gotos(prog: &mut Ir8Program) -> bool {
    let mut changed = false;

    for (func_id, blocks) in prog.func_blocks.iter_mut().enumerate() {
        if blocks.is_empty() {
            continue;
        }
        let entry_pc = prog.func_entries.get(func_id).copied();
        changed |= thread_empty_gotos_func(blocks, entry_pc);
    }

    changed
}

fn thread_empty_gotos_func(blocks: &mut [BasicBlock8], entry_pc: Option<Pc>) -> bool {
    let mut goto_map: HashMap<Pc, Pc> = HashMap::new();
    for bb in blocks.iter() {
        let Terminator8::Goto(target) = bb.terminator else {
            continue;
        };
        if bb.insts.is_empty() && Some(bb.id) != entry_pc && bb.id != target {
            goto_map.insert(bb.id, target);
        }
    }
    if goto_map.is_empty() {
        return false;
    }

    let mut changed = false;
    for bb in blocks.iter_mut() {
        match &mut bb.terminator {
            Terminator8::Goto(target) => {
                let next = resolve_goto_target(*target, &goto_map);
                if *target != next {
                    *target = next;
                    changed = true;
                }
            }
            Terminator8::Branch {
                if_true, if_false, ..
            } => {
                let t = resolve_goto_target(*if_true, &goto_map);
                let f = resolve_goto_target(*if_false, &goto_map);
                if *if_true != t || *if_false != f {
                    *if_true = t;
                    *if_false = f;
                    changed = true;
                }
            }
            Terminator8::Switch {
                targets, default, ..
            } => {
                for pc in targets.iter_mut() {
                    let next = resolve_goto_target(*pc, &goto_map);
                    if *pc != next {
                        *pc = next;
                        changed = true;
                    }
                }
                let d = resolve_goto_target(*default, &goto_map);
                if *default != d {
                    *default = d;
                    changed = true;
                }
            }
            Terminator8::CallSetup {
                callee_entry, cont, ..
            } => {
                let callee = match *callee_entry {
                    crate::ir8::CallTarget::Pc(pc) => {
                        crate::ir8::CallTarget::Pc(resolve_goto_target(pc, &goto_map))
                    }
                    crate::ir8::CallTarget::Builtin(builtin) => {
                        crate::ir8::CallTarget::Builtin(builtin)
                    }
                };
                let cont_pc = resolve_goto_target(*cont, &goto_map);
                if *callee_entry != callee || *cont != cont_pc {
                    *callee_entry = callee;
                    *cont = cont_pc;
                    changed = true;
                }
            }
            Terminator8::Return { .. } | Terminator8::Exit { .. } | Terminator8::Trap(_) => {}
        }
    }

    changed
}

fn resolve_goto_target(mut pc: Pc, goto_map: &HashMap<Pc, Pc>) -> Pc {
    let mut seen: HashSet<Pc> = HashSet::new();
    while let Some(&next) = goto_map.get(&pc) {
        if !seen.insert(pc) || next == pc {
            break;
        }
        pc = next;
    }
    pc
}
