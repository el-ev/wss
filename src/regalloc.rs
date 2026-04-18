use std::collections::{BTreeSet, HashMap, HashSet};

use anyhow::{Context, bail};

use crate::ir8::{BasicBlock8, Inst8Kind, Ir8Program, Pc, Terminator8, VREG_START, Val8, Word};

type FuncId = usize;
type GroupId = u16;

#[derive(Clone, Debug, Default)]
struct LiveInfo {
    live_in: HashSet<Val8>,
    live_out: HashSet<Val8>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Interval {
    start: u32,
    end: u32,
}

#[derive(Clone, Debug, Default)]
struct IntervalBuild {
    intervals: HashMap<Val8, Interval>,
    block_ranges: Vec<(u32, u32)>,
}

#[derive(Clone, Debug, Default)]
struct AllocationResult {
    group_to_phys: HashMap<GroupId, u16>,
    max_phys_group: u16,
}

#[derive(Clone, Debug)]
struct OwnerInfo {
    owner_by_vreg: HashMap<Val8, FuncId>,
}

impl OwnerInfo {
    fn owner_of(&self, reg: Val8) -> Option<FuncId> {
        self.owner_by_vreg.get(&reg).copied()
    }
}

pub fn regalloc(mut ir8: Ir8Program, max_phys_regs: u16) -> anyhow::Result<Ir8Program> {
    if max_phys_regs < VREG_START {
        bail!(
            "max physical register limit must be >= reserved register count ({}), got {}",
            VREG_START,
            max_phys_regs
        );
    }

    let owners = build_owner_info(&ir8)?;
    let referenced_groups = collect_referenced_groups(&ir8, &owners)?;

    let mut allocations: Vec<HashMap<GroupId, u16>> = vec![HashMap::new(); ir8.func_blocks.len()];
    let mut per_func_max_phys: Vec<u16> = vec![0; ir8.func_blocks.len()];

    for (func_id, blocks) in ir8.func_blocks.iter().enumerate() {
        if blocks.is_empty() {
            continue;
        }

        let live_infos = liveness(blocks, func_id, &owners);
        let build = build_intervals(blocks, &live_infos, func_id, &owners);
        debug_assert_eq!(build.block_ranges.len(), blocks.len());

        let mut grouped = group_intervals(&build.intervals);
        for &group in &referenced_groups[func_id] {
            grouped
                .entry(group)
                .or_insert(Interval { start: 0, end: 0 });
        }

        let alloc = linear_scan(grouped);
        per_func_max_phys[func_id] = alloc.max_phys_group;
        allocations[func_id] = alloc.group_to_phys;
    }

    // Calls execute callee code on the same register file. To keep caller
    // values (including non-local temporaries) stable across calls, assign
    // each function a disjoint physical register-group bank.
    let mut func_group_offsets = vec![0u16; ir8.func_blocks.len()];
    let mut next_group_base = 0u16;
    for (func_id, max_group) in per_func_max_phys.iter().copied().enumerate() {
        func_group_offsets[func_id] = next_group_base;
        next_group_base = next_group_base.saturating_add(max_group);
    }

    rewrite_vregs(&mut ir8, &owners, &allocations, &func_group_offsets)?;
    compact_physical_vregs(&mut ir8, max_phys_regs)?;
    ir8.cycles.clear();

    Ok(ir8)
}

fn build_owner_info(ir8: &Ir8Program) -> anyhow::Result<OwnerInfo> {
    let mut owner_by_vreg: HashMap<Val8, FuncId> = HashMap::new();

    let mut local_cursor = VREG_START;
    for (func_id, &num_locals) in ir8.func_num_locals.iter().enumerate() {
        // TODO(i64): local vreg ownership currently allocates 4 registers per value word.
        let local_bytes = num_locals as u16 * 4;
        for i in 0..local_bytes {
            owner_by_vreg.insert(Val8::reg(local_cursor + i), func_id);
        }
        local_cursor = local_cursor.saturating_add(local_bytes);
    }
    let fresh_start = local_cursor;

    for (func_id, blocks) in ir8.func_blocks.iter().enumerate() {
        for bb in blocks {
            for inst in &bb.insts {
                for reg in inst
                    .defs()
                    .into_iter()
                    .chain(inst.uses())
                    .filter(|reg| reg.expect_vreg() >= fresh_start)
                {
                    assign_owner(&mut owner_by_vreg, reg, func_id)?;
                }
            }

            for reg in bb
                .terminator
                .defs()
                .into_iter()
                .chain(bb.terminator.uses())
                .filter(|reg| reg.expect_vreg() >= fresh_start)
            {
                assign_owner(&mut owner_by_vreg, reg, func_id)?;
            }
        }
    }

    Ok(OwnerInfo { owner_by_vreg })
}

fn assign_owner(
    owner_by_vreg: &mut HashMap<Val8, FuncId>,
    reg: Val8,
    func_id: FuncId,
) -> anyhow::Result<()> {
    if let Some(prev) = owner_by_vreg.insert(reg, func_id)
        && prev != func_id
    {
        bail!(
            "vreg r{} is referenced by multiple functions ({} and {})",
            reg.expect_vreg(),
            prev,
            func_id
        );
    }
    Ok(())
}

fn collect_referenced_groups(
    ir8: &Ir8Program,
    owners: &OwnerInfo,
) -> anyhow::Result<Vec<HashSet<GroupId>>> {
    let mut referenced = vec![HashSet::new(); ir8.func_blocks.len()];

    for blocks in &ir8.func_blocks {
        for bb in blocks {
            for reg in bb
                .insts
                .iter()
                .flat_map(|i| i.defs().into_iter().chain(i.uses()))
            {
                if reg.expect_vreg() < VREG_START {
                    continue;
                }
                let func_id = owners
                    .owner_of(reg)
                    .with_context(|| format!("missing owner for vreg r{}", reg.expect_vreg()))?;
                referenced[func_id].insert(group_of(reg));
            }

            for reg in bb.terminator.defs().into_iter().chain(bb.terminator.uses()) {
                if reg.expect_vreg() < VREG_START {
                    continue;
                }
                let func_id = owners
                    .owner_of(reg)
                    .with_context(|| format!("missing owner for vreg r{}", reg.expect_vreg()))?;
                referenced[func_id].insert(group_of(reg));
            }
        }
    }

    Ok(referenced)
}

fn liveness(blocks: &[BasicBlock8], func_id: FuncId, owners: &OwnerInfo) -> Vec<LiveInfo> {
    let n = blocks.len();
    if n == 0 {
        return Vec::new();
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

    let (uses, defs): (Vec<HashSet<Val8>>, Vec<HashSet<Val8>>) = blocks
        .iter()
        .map(|b| block_use_def(b, func_id, owners))
        .unzip();

    let order = rpo_indices(&succ);

    let mut live_in: Vec<HashSet<Val8>> = vec![HashSet::new(); n];
    let mut live_out: Vec<HashSet<Val8>> = vec![HashSet::new(); n];

    let mut changed = true;
    while changed {
        changed = false;
        for &i in order.iter().rev() {
            let mut new_out = HashSet::new();
            for &j in &succ[i] {
                new_out.extend(live_in[j].iter().copied());
            }

            let mut new_in = new_out.clone();
            new_in.retain(|v| !defs[i].contains(v));
            new_in.extend(uses[i].iter().copied());

            if new_out != live_out[i] || new_in != live_in[i] {
                changed = true;
                live_out[i] = new_out;
                live_in[i] = new_in;
            }
        }
    }

    (0..n)
        .map(|i| LiveInfo {
            live_in: std::mem::take(&mut live_in[i]),
            live_out: std::mem::take(&mut live_out[i]),
        })
        .collect()
}

fn block_use_def(
    block: &BasicBlock8,
    func_id: FuncId,
    owners: &OwnerInfo,
) -> (HashSet<Val8>, HashSet<Val8>) {
    let mut uses = HashSet::new();
    let mut defs = HashSet::new();

    let note_use = |reg: Val8, uses: &mut HashSet<Val8>, defs: &HashSet<Val8>| {
        if belongs_to_func(reg, func_id, owners) && !defs.contains(&reg) {
            uses.insert(reg);
        }
    };
    let note_def = |reg: Val8, defs: &mut HashSet<Val8>| {
        if belongs_to_func(reg, func_id, owners) {
            defs.insert(reg);
        }
    };

    for inst in &block.insts {
        for reg in inst.uses() {
            note_use(reg, &mut uses, &defs);
        }
        for reg in inst.defs() {
            note_def(reg, &mut defs);
        }
    }

    for reg in block.terminator.uses() {
        note_use(reg, &mut uses, &defs);
    }
    for reg in block.terminator.defs() {
        note_def(reg, &mut defs);
    }

    (uses, defs)
}

fn rpo_indices(succ: &[Vec<usize>]) -> Vec<usize> {
    fn dfs(node: usize, succ: &[Vec<usize>], vis: &mut [bool], post: &mut Vec<usize>) {
        if vis[node] {
            return;
        }
        vis[node] = true;
        for &next in &succ[node] {
            dfs(next, succ, vis, post);
        }
        post.push(node);
    }

    let n = succ.len();
    if n == 0 {
        return Vec::new();
    }

    let mut vis = vec![false; n];
    let mut post = Vec::with_capacity(n);

    dfs(0, succ, &mut vis, &mut post);
    for i in 0..n {
        if !vis[i] {
            dfs(i, succ, &mut vis, &mut post);
        }
    }

    post.reverse();
    post
}

fn build_intervals(
    blocks: &[BasicBlock8],
    live_infos: &[LiveInfo],
    func_id: FuncId,
    owners: &OwnerInfo,
) -> IntervalBuild {
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
    let order = rpo_indices(&succ);

    let mut intervals: HashMap<Val8, Interval> = HashMap::new();
    let mut block_ranges = vec![(0u32, 0u32); blocks.len()];

    let mut pos: u32 = 0;
    for &idx in &order {
        let block = &blocks[idx];
        let start = pos;

        for inst in &block.insts {
            let p = pos;
            pos += 1;

            for reg in inst
                .uses()
                .into_iter()
                .filter(|&reg| belongs_to_func(reg, func_id, owners))
            {
                touch_use(&mut intervals, reg, p);
            }
            for reg in inst
                .defs()
                .into_iter()
                .filter(|&reg| belongs_to_func(reg, func_id, owners))
            {
                touch_def(&mut intervals, reg, p);
            }
        }

        let p = pos;
        pos += 1;

        for reg in block
            .terminator
            .uses()
            .into_iter()
            .filter(|&reg| belongs_to_func(reg, func_id, owners))
        {
            touch_use(&mut intervals, reg, p);
        }
        for reg in block
            .terminator
            .defs()
            .into_iter()
            .filter(|&reg| belongs_to_func(reg, func_id, owners))
        {
            touch_def(&mut intervals, reg, p);
        }

        let end = pos.saturating_sub(1);
        block_ranges[idx] = (start, end);
    }

    for (i, live) in live_infos.iter().enumerate() {
        let (start, end) = block_ranges[i];

        for &reg in &live.live_in {
            let it = intervals
                .entry(reg)
                .or_insert(Interval { start, end: start });
            it.start = it.start.min(start);
            it.end = it.end.max(start);
        }

        for &reg in &live.live_out {
            let it = intervals.entry(reg).or_insert(Interval { start, end });
            it.start = it.start.min(start);
            it.end = it.end.max(end);
        }
    }

    IntervalBuild {
        intervals,
        block_ranges,
    }
}

fn touch_use(intervals: &mut HashMap<Val8, Interval>, reg: Val8, pos: u32) {
    let it = intervals.entry(reg).or_insert(Interval {
        start: pos,
        end: pos,
    });
    it.end = it.end.max(pos);
}

fn touch_def(intervals: &mut HashMap<Val8, Interval>, reg: Val8, pos: u32) {
    let it = intervals.entry(reg).or_insert(Interval {
        start: pos,
        end: pos,
    });
    it.start = it.start.min(pos);
    it.end = it.end.max(pos);
}

fn group_intervals(intervals: &HashMap<Val8, Interval>) -> HashMap<GroupId, Interval> {
    let mut grouped = HashMap::new();

    for (&reg, &it) in intervals {
        let group = group_of(reg);
        let g = grouped.entry(group).or_insert(it);
        g.start = g.start.min(it.start);
        g.end = g.end.max(it.end);
    }

    grouped
}

fn linear_scan(group_intervals: HashMap<GroupId, Interval>) -> AllocationResult {
    if group_intervals.is_empty() {
        return AllocationResult::default();
    }

    let mut intervals: Vec<(GroupId, Interval)> = group_intervals.into_iter().collect();
    intervals.sort_by_key(|(group, it)| (it.start, it.end, *group));

    #[derive(Clone, Copy)]
    struct Active {
        group: GroupId,
        end: u32,
        phys_group: u16,
    }

    let mut active: Vec<Active> = Vec::new();
    let mut free_phys = BTreeSet::new();
    let mut next_phys: u16 = 1;

    let mut group_to_phys = HashMap::new();
    let mut max_phys_group = 0u16;

    for (group, it) in intervals {
        let mut i = 0;
        while i < active.len() {
            if active[i].end < it.start {
                free_phys.insert(active[i].phys_group);
                active.swap_remove(i);
            } else {
                i += 1;
            }
        }

        let phys_group = if let Some(&p) = free_phys.iter().next() {
            free_phys.remove(&p);
            p
        } else {
            let p = next_phys;
            next_phys = next_phys.saturating_add(1);
            p
        };

        max_phys_group = max_phys_group.max(phys_group);
        group_to_phys.insert(group, phys_group);
        active.push(Active {
            group,
            end: it.end,
            phys_group,
        });
        active.sort_by_key(|a| (a.end, a.group));
    }

    AllocationResult {
        group_to_phys,
        max_phys_group,
    }
}

fn rewrite_vregs(
    prog: &mut Ir8Program,
    owners: &OwnerInfo,
    allocations: &[HashMap<GroupId, u16>],
    func_group_offsets: &[u16],
) -> anyhow::Result<()> {
    remap_program_vregs(prog, &mut |v| {
        map_vreg(v, owners, allocations, func_group_offsets)
    })
}

fn map_vreg(
    reg: Val8,
    owners: &OwnerInfo,
    allocations: &[HashMap<GroupId, u16>],
    func_group_offsets: &[u16],
) -> anyhow::Result<Val8> {
    if reg.is_imm() {
        return Ok(reg);
    }
    let reg_idx = reg.expect_vreg();
    if reg_idx < VREG_START {
        return Ok(reg);
    }

    let owner = owners
        .owner_of(reg)
        .with_context(|| format!("missing owner for vreg r{reg_idx}"))?;
    // TODO(i64): group/lane decomposition assumes 4 lanes per logical value.
    let group = reg_idx / 4;
    let lane = reg_idx % 4;

    let local_phys_group = allocations[owner]
        .get(&group)
        .copied()
        .with_context(|| format!("missing allocation for group {} (vreg r{})", group, reg_idx))?;
    let phys_group = func_group_offsets[owner].saturating_add(local_phys_group);

    Ok(Val8::reg(phys_group * 4 + lane))
}

fn map_word_vals(
    w: &mut Word,
    f: &mut impl FnMut(Val8) -> anyhow::Result<Val8>,
) -> anyhow::Result<()> {
    w.b0 = f(w.b0)?;
    w.b1 = f(w.b1)?;
    w.b2 = f(w.b2)?;
    w.b3 = f(w.b3)?;
    Ok(())
}

fn map_word_vals_through_lane(
    w: &mut Word,
    lane: u8,
    f: &mut impl FnMut(Val8) -> anyhow::Result<Val8>,
) -> anyhow::Result<()> {
    for idx in 0..=lane {
        let slot = match idx {
            0 => &mut w.b0,
            1 => &mut w.b1,
            2 => &mut w.b2,
            3 => &mut w.b3,
            _ => unreachable!("word lane must be in 0..=3"),
        };
        *slot = f(*slot)?;
    }
    Ok(())
}

fn map_inst_kind_vals(
    kind: &mut Inst8Kind,
    f: &mut impl FnMut(Val8) -> anyhow::Result<Val8>,
) -> anyhow::Result<()> {
    match kind {
        Inst8Kind::Getchar
        | Inst8Kind::CsStorePc { .. }
        | Inst8Kind::CsLoadPc { .. }
        | Inst8Kind::CsAlloc(_)
        | Inst8Kind::CsFree(_)
        | Inst8Kind::GlobalGetByte { .. }
        | Inst8Kind::CsLoad { .. } => {}

        Inst8Kind::Copy(s) | Inst8Kind::BoolNot(s) | Inst8Kind::Putchar(s) => {
            *s = f(*s)?;
        }
        Inst8Kind::Add32Byte { lhs, rhs, lane } | Inst8Kind::Sub32Byte { lhs, rhs, lane } => {
            map_word_vals_through_lane(lhs, *lane, f)?;
            map_word_vals_through_lane(rhs, *lane, f)?;
        }
        Inst8Kind::Sub32Borrow { lhs, rhs } => {
            map_word_vals(lhs, f)?;
            map_word_vals(rhs, f)?;
        }

        Inst8Kind::Add(l, r)
        | Inst8Kind::Carry(l, r)
        | Inst8Kind::Sub(l, r)
        | Inst8Kind::MulLo(l, r)
        | Inst8Kind::MulHi(l, r)
        | Inst8Kind::And8(l, r)
        | Inst8Kind::Or8(l, r)
        | Inst8Kind::Xor8(l, r)
        | Inst8Kind::Eq(l, r)
        | Inst8Kind::Ne(l, r)
        | Inst8Kind::LtU(l, r)
        | Inst8Kind::GeU(l, r) => {
            *l = f(*l)?;
            *r = f(*r)?;
        }
        Inst8Kind::BoolAnd(op) | Inst8Kind::BoolOr(op) => {
            *op = op.try_map_vals(&mut *f)?;
        }
        Inst8Kind::Sel(c, l, r) => {
            *l = f(*l)?;
            *r = f(*r)?;
            *c = f(*c)?;
        }
        Inst8Kind::GlobalSetByte { val, .. } => {
            *val = f(*val)?;
        }
        Inst8Kind::LoadMem { addr, .. } => {
            addr.lo = f(addr.lo)?;
            addr.hi = f(addr.hi)?;
        }
        Inst8Kind::StoreMem { addr, val, .. } => {
            addr.lo = f(addr.lo)?;
            addr.hi = f(addr.hi)?;
            *val = f(*val)?;
        }
        Inst8Kind::CsStore { val, .. } => {
            *val = f(*val)?;
        }

        Inst8Kind::ExcFlagSet { val }
        | Inst8Kind::ExcTagSet { val, .. }
        | Inst8Kind::ExcPayloadSet { val, .. } => {
            *val = f(*val)?;
        }
        Inst8Kind::ExcFlagGet
        | Inst8Kind::ExcTagGet { .. }
        | Inst8Kind::ExcPayloadGet { .. } => {}
    }
    Ok(())
}

fn map_terminator_vals(
    term: &mut Terminator8,
    f: &mut impl FnMut(Val8) -> anyhow::Result<Val8>,
) -> anyhow::Result<()> {
    match term {
        Terminator8::Goto(_) | Terminator8::Trap(_) => {}
        Terminator8::Branch { cond, .. } => {
            *cond = f(*cond)?;
        }
        Terminator8::Switch { index, .. } => {
            *index = f(*index)?;
        }
        Terminator8::CallSetup {
            args,
            callee_arg_vregs,
            ..
        } => {
            for w in args {
                map_word_vals(w, f)?;
            }
            for w in callee_arg_vregs {
                map_word_vals(w, f)?;
            }
        }
        Terminator8::Return { val } | Terminator8::Exit { val } => {
            if let Some(w) = val {
                map_word_vals(w, f)?;
            }
        }
    }
    Ok(())
}

fn remap_program_vregs(
    prog: &mut Ir8Program,
    f: &mut impl FnMut(Val8) -> anyhow::Result<Val8>,
) -> anyhow::Result<()> {
    for blocks in &mut prog.func_blocks {
        for bb in blocks {
            for inst in &mut bb.insts {
                if let Some(dst) = inst.dst {
                    inst.dst = Some(f(dst)?);
                }
                map_inst_kind_vals(&mut inst.kind, f)?;
            }
            map_terminator_vals(&mut bb.terminator, f)?;
        }
    }
    Ok(())
}

fn belongs_to_func(reg: Val8, func_id: FuncId, owners: &OwnerInfo) -> bool {
    matches!(reg.reg_index(), Some(idx) if idx >= VREG_START)
        && owners.owner_of(reg) == Some(func_id)
}

fn group_of(reg: Val8) -> GroupId {
    // TODO(i64): register groups are currently packed as 4-byte words.
    reg.expect_vreg() / 4
}

fn compact_physical_vregs(prog: &mut Ir8Program, max_phys_regs: u16) -> anyhow::Result<()> {
    let mut active = BTreeSet::new();
    for blocks in &prog.func_blocks {
        for bb in blocks {
            for inst in &bb.insts {
                if let Some(dst) = inst.dst
                    && dst.expect_vreg() >= VREG_START
                {
                    active.insert(dst.expect_vreg());
                }
                for r in inst.uses() {
                    if r.expect_vreg() >= VREG_START {
                        active.insert(r.expect_vreg());
                    }
                }
            }
            for r in bb.terminator.defs().into_iter().chain(bb.terminator.uses()) {
                if r.expect_vreg() >= VREG_START {
                    active.insert(r.expect_vreg());
                }
            }
        }
    }

    let active_count: u16 = active
        .len()
        .try_into()
        .context("too many active physical registers to index in u16")?;
    let num_vregs = VREG_START.saturating_add(active_count);
    if num_vregs > max_phys_regs {
        bail!(
            "physical register limit exceeded: {} > {} (active={}, reserved={})",
            num_vregs,
            max_phys_regs,
            active_count,
            VREG_START
        );
    }

    if active.is_empty() {
        prog.num_vregs = VREG_START;
        return Ok(());
    }

    let mut dense_map = HashMap::new();
    for (i, reg) in active.into_iter().enumerate() {
        dense_map.insert(Val8::reg(reg), Val8::reg(VREG_START + i as u16));
    }

    remap_program_vregs(prog, &mut |v| remap_dense_reg(v, &dense_map))?;
    prog.num_vregs = num_vregs;
    Ok(())
}

fn remap_dense_reg(val: Val8, dense_map: &HashMap<Val8, Val8>) -> anyhow::Result<Val8> {
    let reg = match val {
        Val8::Imm(_) => return Ok(val),
        Val8::VReg(reg) => reg,
    };
    if reg < VREG_START {
        return Ok(val);
    }
    dense_map
        .get(&val)
        .copied()
        .with_context(|| format!("missing dense remap for physical vreg r{}", reg))
}

#[cfg(test)]
mod tests {
    use std::collections::{HashMap, HashSet};

    use super::regalloc;
    use crate::constants::DEFAULT_MAX_PHYS_REGS;
    use crate::ir8::{
        Addr, BasicBlock8, CallTarget, Inst8, Inst8Kind, Ir8Program, Pc, Terminator8, Val8, Word,
    };

    fn regs_in_func(blocks: &[BasicBlock8]) -> HashSet<Val8> {
        let mut out = HashSet::new();
        for bb in blocks {
            for inst in &bb.insts {
                out.extend(inst.defs());
                out.extend(inst.uses());
            }
            out.extend(bb.terminator.defs());
            out.extend(bb.terminator.uses());
        }
        out
    }

    fn regs_in_program(prog: &Ir8Program) -> HashSet<Val8> {
        let mut out = HashSet::new();
        for blocks in &prog.func_blocks {
            out.extend(regs_in_func(blocks));
        }
        out
    }

    #[test]
    fn regalloc_linear_scan_tie_breaks_by_group_id() {
        let mut grouped = HashMap::new();
        grouped.insert(20, super::Interval { start: 1, end: 10 });
        grouped.insert(10, super::Interval { start: 1, end: 10 });
        let alloc = super::linear_scan(grouped);
        assert_eq!(alloc.group_to_phys.get(&10), Some(&1));
        assert_eq!(alloc.group_to_phys.get(&20), Some(&2));
    }

    #[test]
    fn regalloc_separates_physical_banks_per_function() {
        let f0_r0 = Val8::reg(4);
        let f0_r1 = Val8::reg(5);
        let f0_r2 = Val8::reg(6);
        let f0_r3 = Val8::reg(7);
        let f1_addr_lo = Val8::reg(8);
        let f1_addr_hi = Val8::reg(9);

        let func0 = vec![BasicBlock8 {
            id: Pc::new(0),
            insts: vec![
                Inst8::with_dst(f0_r0, Inst8Kind::Copy(Val8::imm(1))),
                Inst8::with_dst(f0_r1, Inst8Kind::Copy(Val8::imm(0))),
                Inst8::with_dst(f0_r2, Inst8Kind::Copy(Val8::imm(0))),
                Inst8::with_dst(f0_r3, Inst8Kind::Copy(Val8::imm(0))),
            ],
            terminator: Terminator8::Return {
                val: Some(Word::new(f0_r0, f0_r1, f0_r2, f0_r3)),
            },
        }];

        let func1 = vec![
            BasicBlock8 {
                id: Pc::new(1000),
                insts: vec![
                    Inst8::with_dst(f1_addr_lo, Inst8Kind::Copy(Val8::imm(0))),
                    Inst8::with_dst(f1_addr_hi, Inst8Kind::Copy(Val8::imm(0))),
                ],
                terminator: Terminator8::CallSetup {
                    callee_entry: CallTarget::Pc(Pc::new(0)),
                    cont: Pc::new(1001),
                    args: vec![],
                    callee_arg_vregs: vec![],
                },
            },
            BasicBlock8 {
                id: Pc::new(1001),
                insts: vec![Inst8::no_dst(Inst8Kind::StoreMem {
                    base: 12,
                    addr: Addr::new(f1_addr_lo, f1_addr_hi),
                    lane: 0,
                    val: Val8::reg(0),
                })],
                terminator: Terminator8::Exit { val: None },
            },
        ];

        let prog = Ir8Program {
            entry_func: 1,
            num_vregs: 12,
            func_blocks: vec![func0, func1],
            func_entries: vec![Pc::new(0), Pc::new(1000)],
            func_num_locals: vec![0, 0],
            cycles: vec![],
            memory_end: 64,
            init_bytes: vec![0; 64],
            global_init: vec![],
        };

        let out = regalloc(prog, DEFAULT_MAX_PHYS_REGS).expect("regalloc should succeed");

        let mut f0_regs = regs_in_func(&out.func_blocks[0]);
        let mut f1_regs = regs_in_func(&out.func_blocks[1]);
        f0_regs.retain(|r| r.expect_vreg() >= 4);
        f1_regs.retain(|r| r.expect_vreg() >= 4);

        let overlap: HashSet<Val8> = f0_regs.intersection(&f1_regs).copied().collect();
        assert!(
            overlap.is_empty(),
            "functions should not share physical regs: overlap={overlap:?}"
        );
    }

    #[test]
    fn regalloc_compacts_physical_vregs_to_dense_range() {
        let r_a = Val8::reg(4);
        let r_b = Val8::reg(5);
        let r_c = Val8::reg(16);
        let r_d = Val8::reg(17);

        let prog = Ir8Program {
            entry_func: 0,
            num_vregs: 24,
            func_blocks: vec![vec![
                BasicBlock8 {
                    id: Pc::new(0),
                    insts: vec![
                        Inst8::with_dst(r_a, Inst8Kind::Copy(Val8::imm(1))),
                        Inst8::with_dst(r_b, Inst8Kind::Copy(Val8::imm(2))),
                        Inst8::with_dst(r_c, Inst8Kind::Add(r_a, r_b)),
                    ],
                    terminator: Terminator8::Goto(Pc::new(1)),
                },
                BasicBlock8 {
                    id: Pc::new(1),
                    insts: vec![Inst8::with_dst(r_d, Inst8Kind::Copy(r_c))],
                    terminator: Terminator8::Return {
                        val: Some(Word::new(r_d, Val8::reg(0), Val8::reg(1), Val8::reg(2))),
                    },
                },
            ]],
            func_entries: vec![Pc::new(0)],
            func_num_locals: vec![0],
            cycles: vec![],
            memory_end: 0,
            init_bytes: vec![],
            global_init: vec![],
        };

        let out = regalloc(prog, DEFAULT_MAX_PHYS_REGS).expect("regalloc should succeed");
        let mut regs: Vec<u16> = regs_in_program(&out)
            .into_iter()
            .map(|r| r.expect_vreg())
            .filter(|idx| *idx >= 4)
            .collect();
        regs.sort_unstable();
        regs.dedup();

        let expected: Vec<u16> = (4..out.num_vregs).collect();
        assert_eq!(regs, expected);
    }

    #[test]
    fn regalloc_enforces_physical_register_limit_256() {
        let mut insts = Vec::new();
        let mut args = Vec::new();
        let mut next = 4u16;
        for _ in 0..64 {
            let b0 = Val8::reg(next);
            let b1 = Val8::reg(next + 1);
            let b2 = Val8::reg(next + 2);
            let b3 = Val8::reg(next + 3);
            next += 4;

            insts.push(Inst8::with_dst(b0, Inst8Kind::Copy(Val8::imm(1))));
            insts.push(Inst8::with_dst(b1, Inst8Kind::Copy(Val8::imm(2))));
            insts.push(Inst8::with_dst(b2, Inst8Kind::Copy(Val8::imm(3))));
            insts.push(Inst8::with_dst(b3, Inst8Kind::Copy(Val8::imm(4))));
            args.push(Word::new(b0, b1, b2, b3));
        }

        let prog = Ir8Program {
            entry_func: 0,
            num_vregs: 260,
            func_blocks: vec![vec![
                BasicBlock8 {
                    id: Pc::new(0),
                    insts,
                    terminator: Terminator8::CallSetup {
                        callee_entry: CallTarget::Pc(Pc::new(0)),
                        cont: Pc::new(1),
                        args,
                        callee_arg_vregs: vec![],
                    },
                },
                BasicBlock8 {
                    id: Pc::new(1),
                    insts: vec![],
                    terminator: Terminator8::Exit { val: None },
                },
            ]],
            func_entries: vec![Pc::new(0)],
            func_num_locals: vec![0],
            cycles: vec![],
            memory_end: 0,
            init_bytes: vec![],
            global_init: vec![],
        };

        let err = match regalloc(prog, DEFAULT_MAX_PHYS_REGS) {
            Ok(_) => panic!("regalloc should fail when exceeding 256 registers"),
            Err(err) => err,
        };
        let msg = err.to_string();
        assert!(msg.contains("physical register limit exceeded"));
    }

    #[test]
    fn regalloc_ignores_dead_high_lanes_in_projected_byte_ops() {
        let prog = Ir8Program {
            entry_func: 0,
            num_vregs: 16,
            func_blocks: vec![vec![BasicBlock8 {
                id: Pc::new(0),
                insts: vec![
                    Inst8::with_dst(Val8::reg(4), Inst8Kind::Copy(Val8::imm(1))),
                    Inst8::with_dst(
                        Val8::reg(5),
                        Inst8Kind::Add32Byte {
                            lhs: Word::new(
                                Val8::reg(4),
                                Val8::reg(12),
                                Val8::reg(13),
                                Val8::reg(14),
                            ),
                            rhs: Word::from_u32_imm(2),
                            lane: 0,
                        },
                    ),
                ],
                terminator: Terminator8::Exit {
                    val: Some(Word::new(
                        Val8::reg(5),
                        Val8::imm(0),
                        Val8::imm(0),
                        Val8::imm(0),
                    )),
                },
            }]],
            func_entries: vec![Pc::new(0)],
            func_num_locals: vec![1],
            cycles: vec![],
            memory_end: 0,
            init_bytes: vec![],
            global_init: vec![],
        };

        regalloc(prog, DEFAULT_MAX_PHYS_REGS)
            .expect("regalloc should ignore dead projected-byte lanes");
    }
}
