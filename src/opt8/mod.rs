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
    Addr, BasicBlock8, BoolNary8, Inst8, Inst8Kind, Ir8Program, Pc, Terminator8, VREG_START, Val8,
    Word,
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
        changed |= back_copy_coalesce(prog);
        changed |= multi_def_zero_fold(prog);
        changed |= instcombine(prog);
        changed |= combine_boolean_chains(prog);
        changed |= predicate_branch_putchar(prog);
        changed |= store_to_load_forwarding(prog);
        changed |= local_dead_mem_store_elim(prog);
        changed |= thread_empty_gotos(prog);
        changed |= dead_code_elim(prog);
        changed |= remove_unreachable_blocks(prog);
        changed |= coalesce_linear_blocks(prog);
        if !changed {
            break;
        }
    }
    unroll_printf_loop(prog);
}

fn back_copy_coalesce(prog: &mut Ir8Program) -> bool {
    let mut changed = false;
    for blocks in &mut prog.func_blocks {
        changed |= back_copy_coalesce_func(blocks);
    }
    changed
}

fn back_copy_coalesce_func(blocks: &mut [BasicBlock8]) -> bool {
    let def_counts = collect_def_counts(blocks);
    let use_counts = collect_use_counts(blocks);
    let used_before_def = collect_regs_used_before_def(blocks);

    let mut rewrites: Vec<(usize, usize, usize, Val8)> = Vec::new();
    let mut srcs_taken: HashSet<Val8> = HashSet::new();
    let mut dsts_taken: HashSet<Val8> = HashSet::new();

    for (bb_idx, bb) in blocks.iter().enumerate() {
        let mut local_def_idx: HashMap<Val8, usize> = HashMap::new();
        for (i, inst) in bb.insts.iter().enumerate() {
            if let Some(dst) = inst.dst {
                local_def_idx.entry(dst).or_insert(i);
            }
        }

        for (copy_idx, inst) in bb.insts.iter().enumerate() {
            let (Some(dst), Inst8Kind::Copy(src)) = (inst.dst, inst.kind) else {
                continue;
            };
            if dst.expect_vreg() < VREG_START {
                continue;
            }
            if src.is_imm() || dst == src {
                continue;
            }
            if def_counts.get(&dst).copied().unwrap_or(0) != 1 {
                continue;
            }
            if used_before_def.contains(&dst) {
                continue;
            }
            if use_counts.get(&src).copied().unwrap_or(0) != 1 {
                continue;
            }
            if def_counts.get(&src).copied().unwrap_or(0) != 1 {
                continue;
            }
            let Some(&def_idx) = local_def_idx.get(&src) else {
                continue;
            };
            if def_idx >= copy_idx {
                continue;
            }
            if srcs_taken.contains(&src)
                || dsts_taken.contains(&dst)
                || srcs_taken.contains(&dst)
                || dsts_taken.contains(&src)
            {
                continue;
            }
            srcs_taken.insert(src);
            dsts_taken.insert(dst);
            rewrites.push((bb_idx, def_idx, copy_idx, dst));
        }
    }

    if rewrites.is_empty() {
        return false;
    }

    // Apply in (block, copy_idx desc) order so remove() doesn't shift earlier indices.
    rewrites.sort_by(|a, b| a.0.cmp(&b.0).then_with(|| b.2.cmp(&a.2)));
    for (bb_idx, def_idx, copy_idx, dst) in rewrites {
        blocks[bb_idx].insts[def_idx].dst = Some(dst);
        blocks[bb_idx].insts.remove(copy_idx);
    }

    true
}

fn multi_def_zero_fold(prog: &mut Ir8Program) -> bool {
    let mut changed = false;
    for blocks in &mut prog.func_blocks {
        changed |= multi_def_zero_fold_func(blocks);
    }
    changed
}

fn multi_def_zero_fold_func(blocks: &mut [BasicBlock8]) -> bool {
    let used_before_def = collect_regs_used_before_def(blocks);

    let mut const_def: HashMap<Val8, u8> = HashMap::new();
    let mut conflicted: HashSet<Val8> = HashSet::new();

    let mut note_def = |dst: Val8, imm: Option<u8>, conflicted: &mut HashSet<Val8>| {
        if dst.is_imm() || dst.expect_vreg() < VREG_START || conflicted.contains(&dst) {
            return;
        }
        match imm {
            Some(v) => match const_def.get(&dst).copied() {
                Some(prev) if prev == v => {}
                Some(_) => {
                    conflicted.insert(dst);
                    const_def.remove(&dst);
                }
                None => {
                    const_def.insert(dst, v);
                }
            },
            None => {
                conflicted.insert(dst);
                const_def.remove(&dst);
            }
        }
    };

    for bb in blocks.iter() {
        for inst in &bb.insts {
            let Some(dst) = inst.dst else {
                continue;
            };
            let imm = match inst.kind {
                Inst8Kind::Copy(src) => src.imm_value(),
                _ => None,
            };
            note_def(dst, imm, &mut conflicted);
        }
        for r in bb.terminator.defs() {
            note_def(r, None, &mut conflicted);
        }
    }

    let subst: HashMap<Val8, Val8> = const_def
        .into_iter()
        .filter(|(dst, _)| !used_before_def.contains(dst))
        .map(|(dst, v)| (dst, Val8::imm(v)))
        .collect();

    if subst.is_empty() {
        return false;
    }

    rewrite_blocks_with_subst(blocks, &subst);

    let use_counts = collect_use_counts(blocks);
    for bb in blocks.iter_mut() {
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
    }

    true
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

fn run_block_pass(prog: &mut Ir8Program, pass: fn(&mut BasicBlock8) -> bool) -> bool {
    let mut changed = false;
    for blocks in &mut prog.func_blocks {
        for bb in blocks {
            changed |= pass(bb);
        }
    }
    changed
}

fn mem_byte_offset(base: u16, lane: u8) -> u32 {
    u32::from(base) + u32::from(lane)
}

fn addr_uses_reg(addr: Addr, reg: Val8) -> bool {
    reg == addr.lo || reg == addr.hi
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

// If a conditional branch arm consists of exactly one `Putchar(v)` followed by
// a `Goto(join)` and the arm has no other predecessors, hoist the putchar into
// the predecessor as a `PutcharIf { val: v, enable: cond }` (or with a
// `BoolNot` of cond when the put arm is the `if_false` side). The arm becomes
// unreachable; `remove_unreachable_blocks` cleans it up. This unlocks
// putchar batching across the (now-collapsed) diamond.
fn predicate_branch_putchar(prog: &mut Ir8Program) -> bool {
    let mut changed = false;
    for func_id in 0..prog.func_blocks.len() {
        loop {
            if !predicate_branch_putchar_once(&mut prog.func_blocks[func_id]) {
                break;
            }
            changed = true;
        }
    }
    changed
}

fn predicate_branch_putchar_once(blocks: &mut [BasicBlock8]) -> bool {
    if blocks.len() < 2 {
        return false;
    }
    let pc_to_idx: HashMap<Pc, usize> = blocks.iter().enumerate().map(|(i, b)| (b.id, i)).collect();

    let mut term_pred_count = vec![0usize; blocks.len()];
    let mut cs_store_pc_ref: HashSet<Pc> = HashSet::new();
    for bb in blocks.iter() {
        for succ in bb.terminator.successors() {
            if let Some(&to) = pc_to_idx.get(&succ) {
                term_pred_count[to] += 1;
            }
        }
        for inst in &bb.insts {
            if let Inst8Kind::CsStorePc { val, .. } = inst.kind {
                cs_store_pc_ref.insert(val);
            }
        }
    }

    for i in 0..blocks.len() {
        let Terminator8::Branch {
            cond,
            if_true,
            if_false,
        } = blocks[i].terminator
        else {
            continue;
        };

        // Try both arms: (arm_pc, join_pc, negate_cond).
        let candidates = [(if_true, if_false, false), (if_false, if_true, true)];
        for (arm_pc, join_pc, negate) in candidates {
            let Some(&arm_idx) = pc_to_idx.get(&arm_pc) else {
                continue;
            };
            if arm_idx == i {
                continue;
            }
            if term_pred_count[arm_idx] != 1 || cs_store_pc_ref.contains(&arm_pc) {
                continue;
            }
            let arm = &blocks[arm_idx];
            let Terminator8::Goto(arm_target) = arm.terminator else {
                continue;
            };
            if arm_target != join_pc {
                continue;
            }
            if arm.insts.len() != 1 {
                continue;
            }
            let Inst8Kind::Putchar(val) = arm.insts[0].kind else {
                continue;
            };

            let enable = if negate {
                let new_reg = next_vreg(blocks);
                blocks[i]
                    .insts
                    .push(Inst8::with_dst(new_reg, Inst8Kind::BoolNot(cond)));
                new_reg
            } else {
                cond
            };
            blocks[i]
                .insts
                .push(Inst8::no_dst(Inst8Kind::PutcharIf { val, enable }));
            blocks[i].terminator = Terminator8::Goto(join_pc);
            return true;
        }
    }

    false
}

fn next_vreg(blocks: &[BasicBlock8]) -> Val8 {
    let mut max = VREG_START;
    for bb in blocks {
        for inst in &bb.insts {
            if let Some(dst) = inst.dst
                && let Some(idx) = dst.reg_index()
            {
                max = max.max(idx + 1);
            }
            for r in inst.uses() {
                if let Some(idx) = r.reg_index() {
                    max = max.max(idx + 1);
                }
            }
        }
        for r in bb.terminator.uses().into_iter().chain(bb.terminator.defs()) {
            if let Some(idx) = r.reg_index() {
                max = max.max(idx + 1);
            }
        }
    }
    Val8::reg(max)
}

const MAX_UNROLLS_PER_FUNC: usize = 8;

fn unroll_printf_loop(prog: &mut Ir8Program) -> bool {
    let mut changed = false;
    let mut next_vreg = prog.num_vregs.max(VREG_START);
    for blocks in &mut prog.func_blocks {
        let mut count = 0;
        let mut pending: Vec<(usize, PrintLoopShape)> = Vec::new();
        for (i, bb) in blocks.iter().enumerate() {
            if count >= MAX_UNROLLS_PER_FUNC {
                break;
            }
            if let Some(shape) = analyze_print_loop(bb) {
                pending.push((i, shape));
                count += 1;
            }
        }
        let mut new_blocks: Vec<BasicBlock8> = Vec::new();
        let mut next_pc = blocks.iter().map(|b| b.id.index() + 1).max().unwrap_or(0);
        for (idx, shape) in &pending {
            let base = next_vreg;
            let drain_pc = if shape.put_before_load {
                let pc = Pc::new(next_pc);
                next_pc += 1;
                Some(pc)
            } else {
                None
            };
            let (rewritten, rw) =
                rewrite_print_loop(&blocks[*idx], shape, &mut next_vreg, drain_pc);
            if next_vreg != base {
                blocks[*idx] = rewritten;
                changed = true;
                if shape.put_before_load {
                    let drain_pc = drain_pc.unwrap();
                    init_predecessors(blocks, &new_blocks, shape);
                    new_blocks.push(make_drain_block(drain_pc, shape, &rw, &mut next_vreg));
                }
            }
        }
        blocks.extend(new_blocks);
    }
    if changed {
        prog.num_vregs = next_vreg;
    }
    changed
}

fn init_predecessors(blocks: &mut [BasicBlock8], extra: &[BasicBlock8], shape: &PrintLoopShape) {
    let loop_pc = shape.self_pc;
    let loop_bb = blocks
        .iter()
        .chain(extra.iter())
        .find(|b| b.id == loop_pc)
        .expect("loop block must exist");
    let enable_regs: Vec<Val8> = loop_bb
        .insts
        .iter()
        .filter_map(|inst| match inst.kind {
            Inst8Kind::PutcharIf { enable, .. } => Some(enable),
            _ => None,
        })
        .collect();
    if enable_regs.is_empty() {
        return;
    }
    let zero = Val8::imm(0);
    for bb in blocks.iter_mut() {
        if bb.id == loop_pc {
            continue;
        }
        if !bb.terminator.successors().contains(&loop_pc) {
            continue;
        }
        for &e in &enable_regs {
            bb.insts.push(Inst8::with_dst(e, Inst8Kind::Copy(zero)));
        }
    }
}

fn make_drain_block(
    drain_pc: Pc,
    shape: &PrintLoopShape,
    rw: &RewriteVregs,
    next_vreg: &mut u16,
) -> BasicBlock8 {
    debug_assert!(shape.put_before_load);
    let mk_and = |dst: Val8, vals: &[Val8]| {
        Inst8::with_dst(dst, bool::make_bool_kind(bool::BoolOp::And, vals).unwrap())
    };
    let mut insts = Vec::new();
    let alive = [rw.n1, rw.e2, rw.e3, rw.e4];
    for (&base_alive, slot_vals) in alive.iter().zip(rw.slot_vals.iter()) {
        for pe in &shape.putchar_entries {
            let val = rename_val(pe.val, &slot_vals.rename);
            let enable = match pe.enable {
                Some(en) => {
                    let renamed_en = rename_val(en, &slot_vals.rename);
                    let g = Val8::reg(*next_vreg);
                    *next_vreg += 1;
                    insts.push(mk_and(g, &[base_alive, renamed_en]));
                    g
                }
                None => base_alive,
            };
            insts.push(Inst8::no_dst(Inst8Kind::PutcharIf { val, enable }));
        }
    }
    BasicBlock8 {
        id: drain_pc,
        insts,
        terminator: Terminator8::Goto(shape.exit_pc),
    }
}

struct PrintLoopShape {
    load_idx: usize,
    load_base: u16,
    load_addr: Addr,
    load_dst: Val8,
    alias_load_idxs: Vec<usize>,
    putchar_entries: Vec<PutcharEntry>,
    first_putchar_idx: usize,
    dep_idxs: Vec<usize>,
    counter_inc_idxs: [usize; 4],
    counter: Word,
    self_pc: Pc,
    exit_pc: Pc,
    branch_cond: Val8,
    put_before_load: bool,
    bound_end: Option<Word>,
}

struct PutcharEntry {
    idx: usize,
    val: Val8,
    enable: Option<Val8>,
}

struct SlotVals {
    loaded: Val8,
    rename: HashMap<Val8, Val8>,
}

struct RewriteVregs {
    n1: Val8,
    e2: Val8,
    e3: Val8,
    e4: Val8,
    slot_vals: [SlotVals; 4],
}

fn rename_val(v: Val8, rename: &HashMap<Val8, Val8>) -> Val8 {
    rename.get(&v).copied().unwrap_or(v)
}

fn rename_inst_kind(kind: &Inst8Kind, rename: &HashMap<Val8, Val8>) -> Inst8Kind {
    let r = |v: Val8| rename_val(v, rename);
    let rw = |w: Word| Word::new(r(w.b0), r(w.b1), r(w.b2), r(w.b3));
    let ra = |a: Addr| Addr::new(r(a.lo), r(a.hi));
    match kind {
        Inst8Kind::Copy(v) => Inst8Kind::Copy(r(*v)),
        Inst8Kind::BoolNot(v) => Inst8Kind::BoolNot(r(*v)),
        Inst8Kind::Add(a, b) => Inst8Kind::Add(r(*a), r(*b)),
        Inst8Kind::Carry(a, b) => Inst8Kind::Carry(r(*a), r(*b)),
        Inst8Kind::Sub(a, b) => Inst8Kind::Sub(r(*a), r(*b)),
        Inst8Kind::MulLo(a, b) => Inst8Kind::MulLo(r(*a), r(*b)),
        Inst8Kind::MulHi(a, b) => Inst8Kind::MulHi(r(*a), r(*b)),
        Inst8Kind::And8(a, b) => Inst8Kind::And8(r(*a), r(*b)),
        Inst8Kind::Or8(a, b) => Inst8Kind::Or8(r(*a), r(*b)),
        Inst8Kind::Xor8(a, b) => Inst8Kind::Xor8(r(*a), r(*b)),
        Inst8Kind::Eq(a, b) => Inst8Kind::Eq(r(*a), r(*b)),
        Inst8Kind::Ne(a, b) => Inst8Kind::Ne(r(*a), r(*b)),
        Inst8Kind::LtU(a, b) => Inst8Kind::LtU(r(*a), r(*b)),
        Inst8Kind::GeU(a, b) => Inst8Kind::GeU(r(*a), r(*b)),
        Inst8Kind::Sel(c, t, f) => Inst8Kind::Sel(r(*c), r(*t), r(*f)),
        Inst8Kind::BoolAnd(nary) => Inst8Kind::BoolAnd(nary.map_vals(&r)),
        Inst8Kind::BoolOr(nary) => Inst8Kind::BoolOr(nary.map_vals(r)),
        Inst8Kind::Add32Byte { lhs, rhs, lane } => Inst8Kind::Add32Byte {
            lhs: rw(*lhs),
            rhs: rw(*rhs),
            lane: *lane,
        },
        Inst8Kind::Sub32Byte { lhs, rhs, lane } => Inst8Kind::Sub32Byte {
            lhs: rw(*lhs),
            rhs: rw(*rhs),
            lane: *lane,
        },
        Inst8Kind::Sub32Borrow { lhs, rhs } => Inst8Kind::Sub32Borrow {
            lhs: rw(*lhs),
            rhs: rw(*rhs),
        },
        Inst8Kind::LoadMem { base, addr, lane } => Inst8Kind::LoadMem {
            base: *base,
            addr: ra(*addr),
            lane: *lane,
        },
        other => *other,
    }
}

fn analyze_print_loop(bb: &BasicBlock8) -> Option<PrintLoopShape> {
    let Terminator8::Branch {
        cond,
        if_true,
        if_false,
    } = bb.terminator
    else {
        return None;
    };
    if if_true != bb.id {
        return None;
    }

    let mut putchar_entries: Vec<PutcharEntry> = Vec::new();
    let mut load_info: Option<(usize, u16, Addr, Val8)> = None;
    let mut alias_load_idxs: Vec<usize> = Vec::new();
    let mut adds_plus1: Vec<(usize, u8, Word, Val8)> = Vec::new();

    for (idx, inst) in bb.insts.iter().enumerate() {
        match inst.kind {
            Inst8Kind::Putchar(v) => {
                putchar_entries.push(PutcharEntry {
                    idx,
                    val: v,
                    enable: None,
                });
            }
            Inst8Kind::PutcharIf { val, enable } => {
                putchar_entries.push(PutcharEntry {
                    idx,
                    val,
                    enable: Some(enable),
                });
            }
            Inst8Kind::Getchar => return None,
            Inst8Kind::LoadMem {
                base,
                addr,
                lane: 0,
            } => {
                if let Some((_, lb, la, _)) = load_info {
                    if base == lb && addr == la {
                        alias_load_idxs.push(idx);
                    } else {
                        return None;
                    }
                } else {
                    load_info = Some((idx, base, addr, inst.dst?));
                }
            }
            Inst8Kind::LoadMem { .. } | Inst8Kind::StoreMem { .. } => return None,
            Inst8Kind::Add32Byte { lhs, rhs, lane } if rhs == Word::from_u32_imm(1) => {
                adds_plus1.push((idx, lane, lhs, inst.dst?));
            }
            _ => {
                if rewrite::has_side_effect(&inst.kind) {
                    return None;
                }
            }
        }
    }

    if putchar_entries.is_empty() {
        return None;
    }
    let (load_idx, load_base, load_addr, load_dst) = load_info?;

    // At least one putchar must print the loaded byte (directly, alias, or copy).
    let alias_dsts: Vec<Val8> = alias_load_idxs
        .iter()
        .filter_map(|&i| bb.insts[i].dst)
        .collect();
    let is_load_val = |v: Val8| -> bool {
        v == load_dst
            || alias_dsts.contains(&v)
            || bb.insts.iter().any(|inst| {
                let Inst8Kind::Copy(src) = inst.kind else {
                    return false;
                };
                (src == load_dst || alias_dsts.contains(&src)) && inst.dst == Some(v)
            })
    };
    let prints_loaded = putchar_entries.iter().any(|pe| is_load_val(pe.val));
    if !prints_loaded {
        return None;
    }

    if adds_plus1.len() != 4 {
        return None;
    }
    let counter = adds_plus1[0].2;
    if !adds_plus1.iter().all(|(_, _, lhs, _)| *lhs == counter) {
        return None;
    }
    let mut counter_inc_idxs = [0usize; 4];
    let mut seen_lanes = [false; 4];
    for &(idx, lane, _, _) in &adds_plus1 {
        if lane > 3 || seen_lanes[lane as usize] {
            return None;
        }
        seen_lanes[lane as usize] = true;
        counter_inc_idxs[lane as usize] = idx;
    }

    // The load address must depend on the counter.
    let defs: HashMap<Val8, &Inst8> = bb
        .insts
        .iter()
        .filter_map(|inst| inst.dst.map(|d| (d, inst)))
        .collect();
    let counter_bytes = [counter.b0, counter.b1, counter.b2, counter.b3];
    let depends_on_counter = |v: Val8| -> bool {
        if counter_bytes.contains(&v) {
            return true;
        }
        if let Some(inst) = defs.get(&v) {
            inst.uses().iter().any(|u| counter_bytes.contains(u))
        } else {
            false
        }
    };
    if !depends_on_counter(load_addr.lo) && !depends_on_counter(load_addr.hi) {
        return None;
    }

    // Build set of "special" indices (load, aliases, putchars, counter incs).
    let mut special_idxs: HashSet<usize> = HashSet::new();
    special_idxs.insert(load_idx);
    for &ai in &alias_load_idxs {
        special_idxs.insert(ai);
    }
    for pe in &putchar_entries {
        special_idxs.insert(pe.idx);
    }
    for &ci in &counter_inc_idxs {
        special_idxs.insert(ci);
    }

    // Identify instructions that transitively depend on load_dst.
    let mut dep_set: HashSet<Val8> = HashSet::new();
    dep_set.insert(load_dst);
    for &ad in &alias_dsts {
        dep_set.insert(ad);
    }
    let mut dep_idxs: Vec<usize> = Vec::new();
    for (idx, inst) in bb.insts.iter().enumerate() {
        if special_idxs.contains(&idx) {
            continue;
        }
        if inst.uses().iter().any(|u| dep_set.contains(u)) {
            dep_idxs.push(idx);
            if let Some(dst) = inst.dst {
                dep_set.insert(dst);
            }
        }
    }

    // Detect bounded loop: branch condition from lt_u(counter, end).
    let bound_end = detect_bounded_loop(bb, cond, &counter_bytes);

    let first_putchar_idx = putchar_entries.iter().map(|pe| pe.idx).min().unwrap();

    Some(PrintLoopShape {
        load_idx,
        load_base,
        load_addr,
        load_dst,
        alias_load_idxs,
        putchar_entries,
        first_putchar_idx,
        dep_idxs,
        counter_inc_idxs,
        counter,
        self_pc: if_true,
        exit_pc: if_false,
        branch_cond: cond,
        put_before_load: first_putchar_idx < load_idx,
        bound_end,
    })
}

fn detect_bounded_loop(
    bb: &BasicBlock8,
    branch_cond: Val8,
    counter_bytes: &[Val8; 4],
) -> Option<Word> {
    let counter_set: HashSet<Val8> = counter_bytes.iter().copied().collect();

    // Collect the post-increment copies: counter_byte -> post_inc_val.
    let mut post_inc: HashMap<Val8, Val8> = HashMap::new();
    for inst in &bb.insts {
        if matches!(inst.kind, Inst8Kind::Copy(_))
            && let Some(dst) = inst.dst
            && counter_set.contains(&dst)
        {
            post_inc.insert(dst, dst);
        }
    }

    // Find lt_u instructions comparing a counter/post-inc byte against something.
    let all_counter: HashSet<Val8> = counter_set
        .iter()
        .chain(post_inc.values())
        .copied()
        .collect();
    let mut end_for_lane: [Option<Val8>; 4] = [None; 4];
    for inst in &bb.insts {
        if let Inst8Kind::LtU(a, b) = inst.kind {
            for (lane, cb) in counter_bytes.iter().enumerate() {
                if (a == *cb || post_inc.get(cb) == Some(&a)) && !all_counter.contains(&b) {
                    end_for_lane[lane] = Some(b);
                }
            }
        }
    }

    // Need at least byte 0 to form an end word.
    let b0 = end_for_lane[0]?;
    let b1 = end_for_lane[1].unwrap_or(Val8::imm(0));
    let b2 = end_for_lane[2].unwrap_or(Val8::imm(0));
    let b3 = end_for_lane[3].unwrap_or(Val8::imm(0));

    // Verify the branch condition transitively depends on these lt_u results.
    let defs: HashMap<Val8, &Inst8> = bb
        .insts
        .iter()
        .filter_map(|i| i.dst.map(|d| (d, i)))
        .collect();
    let mut dep_on_ltu = false;
    let mut stack = vec![branch_cond];
    let mut visited: HashSet<Val8> = HashSet::new();
    while let Some(v) = stack.pop() {
        if !visited.insert(v) {
            continue;
        }
        if let Some(inst) = defs.get(&v) {
            if matches!(inst.kind, Inst8Kind::LtU(a, _) if all_counter.contains(&a)) {
                dep_on_ltu = true;
                break;
            }
            stack.extend(inst.uses());
        }
    }
    if !dep_on_ltu {
        return None;
    }

    Some(Word::new(b0, b1, b2, b3))
}

fn rewrite_print_loop(
    bb: &BasicBlock8,
    shape: &PrintLoopShape,
    next_vreg: &mut u16,
    drain_pc: Option<Pc>,
) -> (BasicBlock8, RewriteVregs) {
    let mut alloc = || {
        let v = Val8::reg(*next_vreg);
        *next_vreg += 1;
        v
    };

    let c2 = alloc();
    let c3 = alloc();
    let c4 = alloc();
    let n1 = alloc();
    let e2 = alloc();
    let e3 = alloc();
    let e4 = alloc();
    let combined = alloc();

    let zero = Val8::imm(0);
    let four = Word::from_u32_imm(4);
    let mk_and = |dst: Val8, vals: &[Val8]| {
        Inst8::with_dst(dst, bool::make_bool_kind(bool::BoolOp::And, vals).unwrap())
    };

    // Build rename maps for each slot.
    let alias_dsts: Vec<Val8> = shape
        .alias_load_idxs
        .iter()
        .filter_map(|&i| bb.insts[i].dst)
        .collect();

    let mut slot_vals: [SlotVals; 4] = std::array::from_fn(|_| SlotVals {
        loaded: zero,
        rename: HashMap::new(),
    });
    let mut slot0_rename = HashMap::new();
    for &ad in &alias_dsts {
        slot0_rename.insert(ad, shape.load_dst);
    }
    slot_vals[0] = SlotVals {
        loaded: shape.load_dst,
        rename: slot0_rename,
    };
    slot_vals[1] = SlotVals {
        loaded: c2,
        rename: HashMap::new(),
    };
    slot_vals[2] = SlotVals {
        loaded: c3,
        rename: HashMap::new(),
    };
    slot_vals[3] = SlotVals {
        loaded: c4,
        rename: HashMap::new(),
    };

    // For slots 1-3, build rename map: load_dst -> c_i, aliases -> c_i.
    for sv in slot_vals.iter_mut().skip(1) {
        sv.rename.insert(shape.load_dst, sv.loaded);
        for &ad in &alias_dsts {
            sv.rename.insert(ad, sv.loaded);
        }
    }

    // Clone dependent instructions for slots 1-3 (slot 0 uses originals).
    let mut slot_dep_insts: [Vec<Inst8>; 4] = std::array::from_fn(|_| Vec::new());
    for slot in 1..4 {
        for &dep_idx in &shape.dep_idxs {
            let inst = &bb.insts[dep_idx];
            let new_dst = alloc();
            let new_kind = rename_inst_kind(&inst.kind, &slot_vals[slot].rename);
            slot_dep_insts[slot].push(Inst8::with_dst(new_dst, new_kind));
            if let Some(old_dst) = inst.dst {
                slot_vals[slot].rename.insert(old_dst, new_dst);
            }
        }
    }

    // Build set of special indices to skip during pass-through.
    let mut special: HashSet<usize> = HashSet::new();
    special.insert(shape.load_idx);
    for &ai in &shape.alias_load_idxs {
        special.insert(ai);
    }
    for pe in &shape.putchar_entries {
        special.insert(pe.idx);
    }
    for &ci in &shape.counter_inc_idxs {
        special.insert(ci);
    }

    let emit_loads = |new_insts: &mut Vec<Inst8>| {
        let base = shape.load_base;
        new_insts.push(Inst8::with_dst(
            shape.load_dst,
            Inst8Kind::LoadMem {
                base,
                addr: shape.load_addr,
                lane: 0,
            },
        ));
        for &(dst, off) in &[(c2, 1u16), (c3, 2), (c4, 3)] {
            new_insts.push(Inst8::with_dst(
                dst,
                Inst8Kind::LoadMem {
                    base: base + off,
                    addr: shape.load_addr,
                    lane: 0,
                },
            ));
        }
    };

    let emit_alive_null_term = |new_insts: &mut Vec<Inst8>, next_vreg: &mut u16| {
        let n2 = Val8::reg(*next_vreg);
        *next_vreg += 1;
        let n3 = Val8::reg(*next_vreg);
        *next_vreg += 1;
        let n4 = Val8::reg(*next_vreg);
        *next_vreg += 1;
        new_insts.push(Inst8::with_dst(n1, Inst8Kind::Ne(shape.load_dst, zero)));
        new_insts.push(Inst8::with_dst(n2, Inst8Kind::Ne(c2, zero)));
        new_insts.push(Inst8::with_dst(n3, Inst8Kind::Ne(c3, zero)));
        new_insts.push(Inst8::with_dst(n4, Inst8Kind::Ne(c4, zero)));
        new_insts.push(mk_and(e2, &[n1, n2]));
        new_insts.push(mk_and(e3, &[n1, n2, n3]));
        new_insts.push(mk_and(e4, &[n1, n2, n3, n4]));
    };

    let emit_alive_bounded = |new_insts: &mut Vec<Inst8>, next_vreg: &mut u16, end_word: Word| {
        // remaining = end - counter (16-bit)
        let rem_lo = Val8::reg(*next_vreg);
        *next_vreg += 1;
        let rem_hi = Val8::reg(*next_vreg);
        *next_vreg += 1;
        new_insts.push(Inst8::with_dst(
            rem_lo,
            Inst8Kind::Sub32Byte {
                lhs: end_word,
                rhs: shape.counter,
                lane: 0,
            },
        ));
        new_insts.push(Inst8::with_dst(
            rem_hi,
            Inst8Kind::Sub32Byte {
                lhs: end_word,
                rhs: shape.counter,
                lane: 1,
            },
        ));
        let hi_nz = Val8::reg(*next_vreg);
        *next_vreg += 1;
        new_insts.push(Inst8::with_dst(hi_nz, Inst8Kind::Ne(rem_hi, zero)));
        // alive_i = hi_nz OR (rem_lo >= i+1)
        // rem_lo >= k  <==>  NOT lt_u(rem_lo, k)
        for (alive_dst, k) in [(e2, 2u8), (e3, 3), (e4, 4)] {
            let lt_k = Val8::reg(*next_vreg);
            *next_vreg += 1;
            let ge_k = Val8::reg(*next_vreg);
            *next_vreg += 1;
            new_insts.push(Inst8::with_dst(lt_k, Inst8Kind::LtU(rem_lo, Val8::imm(k))));
            new_insts.push(Inst8::with_dst(ge_k, Inst8Kind::Eq(lt_k, zero)));
            new_insts.push(Inst8::with_dst(
                alive_dst,
                Inst8Kind::BoolOr(BoolNary8::from_vals(&[hi_nz, ge_k]).unwrap()),
            ));
        }
        // n1 is always true for bounded loops (loop entry guarantees slot 0).
        new_insts.push(Inst8::with_dst(n1, Inst8Kind::Copy(Val8::imm(1))));
    };

    let emit_putchar_group = |new_insts: &mut Vec<Inst8>,
                              next_vreg: &mut u16,
                              slot_dep_insts: &[Vec<Inst8>; 4]| {
        let alive = [n1, e2, e3, e4];
        for slot in 0..4usize {
            // Emit cloned dependent instructions for this slot.
            if slot > 0 {
                for inst in &slot_dep_insts[slot] {
                    new_insts.push(inst.clone());
                }
            }
            for pe in &shape.putchar_entries {
                let val = rename_val(pe.val, &slot_vals[slot].rename);
                if slot == 0 {
                    // Slot 0: emit as original (no alive gating).
                    match pe.enable {
                        Some(en) => {
                            new_insts.push(Inst8::no_dst(Inst8Kind::PutcharIf { val, enable: en }));
                        }
                        None => {
                            new_insts.push(Inst8::no_dst(Inst8Kind::Putchar(val)));
                        }
                    }
                } else {
                    let base_alive = alive[slot];
                    let enable = match pe.enable {
                        Some(en) => {
                            let renamed_en = rename_val(en, &slot_vals[slot].rename);
                            let g = Val8::reg(*next_vreg);
                            *next_vreg += 1;
                            new_insts.push(mk_and(g, &[base_alive, renamed_en]));
                            g
                        }
                        None => base_alive,
                    };
                    new_insts.push(Inst8::no_dst(Inst8Kind::PutcharIf { val, enable }));
                }
            }
        }
    };

    let mut new_insts: Vec<Inst8> = Vec::with_capacity(bb.insts.len() + 40);
    for (idx, inst) in bb.insts.iter().enumerate() {
        if idx == shape.load_idx {
            emit_loads(&mut new_insts);
            match shape.bound_end {
                Some(end_word) => {
                    emit_alive_bounded(&mut new_insts, next_vreg, end_word);
                }
                None => {
                    emit_alive_null_term(&mut new_insts, next_vreg);
                }
            }
            continue;
        }
        if idx == shape.first_putchar_idx {
            emit_putchar_group(&mut new_insts, next_vreg, &slot_dep_insts);
            continue;
        }
        if special.contains(&idx) {
            if let Inst8Kind::Add32Byte { lane, .. } = inst.kind {
                new_insts.push(Inst8::with_dst(
                    inst.dst.unwrap(),
                    Inst8Kind::Add32Byte {
                        lhs: shape.counter,
                        rhs: four,
                        lane,
                    },
                ));
            }
            continue;
        }
        new_insts.push(inst.clone());
    }

    // Combined loop exit condition.
    if shape.bound_end.is_some() {
        // For bounded loops, the original branch condition computes
        // (post_inc_counter < end). After changing +1 to +4, this
        // already tests (ptr+4 < end). AND with e4 is NOT needed
        // because alive flags gate the putchars in-body.
        // However, we still need to ensure the original branch_cond is
        // used (it references the post-increment counter registers which
        // are now ptr+4).
        // No combined — just use the original branch condition.
    } else {
        new_insts.push(mk_and(combined, &[shape.branch_cond, e4]));
    }

    let exit_target = drain_pc.unwrap_or(shape.exit_pc);

    let rw = RewriteVregs {
        n1,
        e2,
        e3,
        e4,
        slot_vals,
    };

    let branch_cond_final = if shape.bound_end.is_some() {
        shape.branch_cond
    } else {
        combined
    };

    (
        BasicBlock8 {
            id: bb.id,
            insts: new_insts,
            terminator: Terminator8::Branch {
                cond: branch_cond_final,
                if_true: shape.self_pc,
                if_false: exit_target,
            },
        },
        rw,
    )
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

fn store_to_load_forwarding(prog: &mut Ir8Program) -> bool {
    run_block_pass(prog, stlf_block)
}

fn stlf_block(bb: &mut BasicBlock8) -> bool {
    let mut changed = false;
    let mut tracked_addr: Option<Addr> = None;
    let mut forwarded: HashMap<u32, Val8> = HashMap::new();

    for inst in bb.insts.iter_mut() {
        match inst.kind {
            Inst8Kind::LoadMem { base, addr, lane } if tracked_addr == Some(addr) => {
                let offset = mem_byte_offset(base, lane);
                if let Some(&stored_val) = forwarded.get(&offset) {
                    inst.kind = Inst8Kind::Copy(stored_val);
                    changed = true;
                }
            }
            Inst8Kind::StoreMem {
                base,
                addr,
                lane,
                val,
            } => {
                let offset = mem_byte_offset(base, lane);
                if tracked_addr.is_some() && tracked_addr != Some(addr) {
                    forwarded.clear();
                }
                tracked_addr = Some(addr);
                forwarded.insert(offset, val);
            }
            _ => {}
        }

        if let Some(dst) = inst.dst
            && let Some(a) = tracked_addr
        {
            if addr_uses_reg(a, dst) {
                forwarded.clear();
                tracked_addr = None;
            } else {
                forwarded.retain(|_, val| *val != dst);
                if forwarded.is_empty() {
                    tracked_addr = None;
                }
            }
        }
    }

    changed
}

fn local_dead_mem_store_elim(prog: &mut Ir8Program) -> bool {
    run_block_pass(prog, local_dead_mem_store_elim_block)
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
                let offset = mem_byte_offset(base, lane);
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
                    let offset = mem_byte_offset(base, lane);
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
            && addr_uses_reg(a, dst)
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
    run_block_pass(prog, local_copy_propagation_block)
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
