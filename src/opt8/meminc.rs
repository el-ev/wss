//! Fuse `load32; add32/sub32 imm; store32` of the same four bytes into one
//! [`Inst8Kind::MemInc32`].
//!
//! Unfused, the update needs four load lookups, an add chain and four stores
//! spread over several cycles (the scheduler never chains a register through
//! one cycle). Fused, the memory merge computes the new bytes straight from
//! the cells' previous values in a single cycle.

use super::*;

/// The four byte positions of an i32 at `base`: (`base` field, lane).
fn byte_pos(base: u16, j: usize) -> (u16, u8) {
    (base + 2 * (j as u16 / 2), (j % 2) as u8)
}

/// Positions of a block's stores and register definitions.
struct BlockIndex {
    /// First store to each `(base, addr, lane)`.
    stores: HashMap<(u16, Addr, u8), usize>,
    /// First definition of each register.
    defs: HashMap<Val8, usize>,
}

impl BlockIndex {
    fn new(insts: &[Inst8]) -> Self {
        let mut stores = HashMap::new();
        let mut defs = HashMap::new();
        for (i, inst) in insts.iter().enumerate() {
            if let Inst8Kind::StoreMem {
                base, addr, lane, ..
            } = inst.kind
            {
                stores.entry((base, addr, lane)).or_insert(i);
            }
            if let Some(dst) = inst.dst {
                defs.entry(dst).or_insert(i);
            }
        }
        Self { stores, defs }
    }
}

struct Match {
    /// Instruction indices to delete, sorted.
    remove: Vec<usize>,
    /// Where the fused op goes (first of the stores).
    at: usize,
    inst: Inst8,
}

fn match_at(
    insts: &[Inst8],
    first_store: usize,
    index: &BlockIndex,
    uses: &HashMap<Val8, usize>,
    defs: &HashMap<Val8, usize>,
) -> Option<Match> {
    let Inst8Kind::StoreMem {
        base,
        addr,
        lane: 0,
        ..
    } = insts[first_store].kind
    else {
        return None;
    };
    // The only definition of `r`, when it is in this block.
    let find_def = |r: Val8| -> Option<usize> {
        (defs.get(&r).copied() == Some(1))
            .then(|| index.defs.get(&r).copied())
            .flatten()
    };

    let mut stores = [0usize; 4];
    let mut adds = [0usize; 4];
    let mut loads = [0usize; 4];
    let mut lane_delta = [0u32; 4];
    let mut loaded: [Option<Val8>; 4] = [None; 4];
    for j in 0..4 {
        let (b, l) = byte_pos(base, j);
        let s = *index.stores.get(&(b, addr, l))?;
        let Inst8Kind::StoreMem { val, .. } = insts[s].kind else {
            unreachable!()
        };
        if uses.get(&val).copied() != Some(1) {
            return None;
        }
        let d = find_def(val)?;
        let lane = j as u8;
        let (word, delta) = match insts[d].kind {
            Inst8Kind::Add32Byte { lhs, rhs, lane: k } if k == lane => {
                match (rhs.imm_prefix(k), lhs.imm_prefix(k)) {
                    (Some(imm), _) => (lhs, imm),
                    (None, Some(imm)) => (rhs, imm),
                    _ => return None,
                }
            }
            Inst8Kind::Sub32Byte { lhs, rhs, lane: k } if k == lane => {
                (lhs, rhs.imm_prefix(k)?.wrapping_neg())
            }
            _ => return None,
        };
        for (t, b) in word.bytes().into_iter().enumerate().take(j + 1) {
            match loaded[t] {
                None => loaded[t] = Some(b),
                Some(prev) if prev == b => {}
                Some(_) => return None,
            }
        }
        stores[j] = s;
        adds[j] = d;
        lane_delta[j] = delta;
    }
    for (t, r) in loaded.iter().enumerate() {
        let r = (*r)?;
        // Read by lanes t..=3 of the add chain only.
        if uses.get(&r).copied() != Some(4 - t) {
            return None;
        }
        let d = find_def(r)?;
        let (b, l) = byte_pos(base, t);
        if !matches!(insts[d].kind, Inst8Kind::LoadMem { base: lb, addr: la, lane: ll }
            if lb == b && la == addr && ll == l)
        {
            return None;
        }
        loads[t] = d;
    }

    // Lane `j` only sees the low `j + 1` bytes of the delta.
    let delta = lane_delta[3];
    // The delta lives in a registered property, and Blink prints registered
    // numbers with 6 significant digits when substituting them: keep it
    // within the range that round-trips exactly.
    if (delta as i32).unsigned_abs() > 999_999 {
        return None;
    }
    for (j, d) in lane_delta.iter().enumerate().take(3) {
        let mask = (1u32 << (8 * (j + 1))) - 1;
        if d & mask != delta & mask {
            return None;
        }
    }

    let mut remove: Vec<usize> = stores.iter().chain(&adds).chain(&loads).copied().collect();
    remove.sort_unstable();
    remove.dedup();
    if remove.len() != 12 {
        return None;
    }
    // Nothing in between may touch memory or move the address.
    for (i, inst) in insts
        .iter()
        .enumerate()
        .take(remove[11] + 1)
        .skip(remove[0])
    {
        if remove.contains(&i) {
            continue;
        }
        let touches_mem = matches!(
            inst.kind,
            Inst8Kind::LoadMem { .. } | Inst8Kind::StoreMem { .. } | Inst8Kind::MemInc32 { .. }
        );
        let moves_addr = inst.dst.is_some_and(|d| addr_uses_reg(addr, d));
        if touches_mem || moves_addr {
            return None;
        }
    }
    // Loads read the old value, so every load must precede every store.
    let first_st = *stores.iter().min()?;
    if *loads.iter().max()? > first_st {
        return None;
    }
    Some(Match {
        remove,
        at: first_st,
        inst: Inst8::no_dst(Inst8Kind::MemInc32 { base, addr, delta }),
    })
}

pub(super) fn fuse_mem_inc(prog: &mut Ir8Program) {
    for blocks in &mut prog.func_blocks {
        // A fused group's loaded and summed registers are used only inside
        // the group, so updating the counts in place leaves every other
        // candidate's checks exactly as a recount would.
        let mut uses = collect_use_counts(blocks);
        let mut defs = collect_def_counts(blocks);
        let uncount = |counts: &mut HashMap<Val8, usize>, r: Val8| {
            if let Some(n) = counts.get_mut(&r) {
                *n -= 1;
            }
        };
        for bb in blocks.iter_mut() {
            loop {
                let index = BlockIndex::new(&bb.insts);
                let Some(m) =
                    (0..bb.insts.len()).find_map(|i| match_at(&bb.insts, i, &index, &uses, &defs))
                else {
                    break;
                };
                let mut keep = vec![true; bb.insts.len()];
                for &i in &m.remove {
                    keep[i] = false;
                    let inst = &bb.insts[i];
                    inst.uses().into_iter().for_each(|r| uncount(&mut uses, r));
                    inst.dst.into_iter().for_each(|d| uncount(&mut defs, d));
                }
                for r in m.inst.uses() {
                    *uses.entry(r).or_insert(0) += 1;
                }
                keep[m.at] = true;
                bb.insts[m.at] = m.inst;
                filter_by_mask(&mut bb.insts, &keep);
            }
        }
    }
}
