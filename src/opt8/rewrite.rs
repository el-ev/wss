use super::*;

pub(super) fn rewrite_blocks_with_subst(blocks: &mut [BasicBlock8], subst: &HashMap<Val8, Val8>) {
    for bb in blocks.iter_mut() {
        for inst in &mut bb.insts {
            *inst = rewrite_inst(inst.clone(), subst);
        }
        rewrite_term(&mut bb.terminator, subst);
    }
}

pub(super) fn collect_def_counts(blocks: &[BasicBlock8]) -> HashMap<Val8, usize> {
    let mut counts: HashMap<Val8, usize> = HashMap::new();
    for bb in blocks {
        for dst in bb
            .insts
            .iter()
            .filter_map(|inst| inst.dst)
            .chain(bb.terminator.defs())
        {
            *counts.entry(dst).or_insert(0) += 1;
        }
    }
    counts
}

pub(super) fn collect_use_counts(blocks: &[BasicBlock8]) -> HashMap<Val8, usize> {
    let mut counts: HashMap<Val8, usize> = HashMap::new();
    for bb in blocks {
        for r in bb
            .insts
            .iter()
            .flat_map(|inst| inst.uses().into_iter())
            .chain(bb.terminator.uses())
        {
            *counts.entry(r).or_insert(0) += 1;
        }
    }
    counts
}

pub(super) fn collect_regs_used_before_def(blocks: &[BasicBlock8]) -> HashSet<Val8> {
    let mut seen_defs = HashSet::new();
    let mut used_before_def = HashSet::new();

    let note_use = |reg: Val8, seen_defs: &HashSet<Val8>, used_before_def: &mut HashSet<Val8>| {
        if !reg.is_imm() && !seen_defs.contains(&reg) {
            used_before_def.insert(reg);
        }
    };
    let note_def = |reg: Val8, seen_defs: &mut HashSet<Val8>| {
        if !reg.is_imm() {
            seen_defs.insert(reg);
        }
    };

    for bb in blocks {
        for inst in &bb.insts {
            for reg in inst.uses() {
                note_use(reg, &seen_defs, &mut used_before_def);
            }
            if let Some(dst) = inst.dst {
                note_def(dst, &mut seen_defs);
            }
        }

        for reg in bb.terminator.uses() {
            note_use(reg, &seen_defs, &mut used_before_def);
        }
        for reg in bb.terminator.defs() {
            note_def(reg, &mut seen_defs);
        }
    }

    used_before_def
}

pub(super) fn is_stable_source(reg: Val8, def_counts: &HashMap<Val8, usize>) -> bool {
    def_counts.get(&reg).copied().unwrap_or(0) <= 1
}

pub(super) fn resolve(subst: &HashMap<Val8, Val8>, mut r: Val8) -> Val8 {
    let mut hops = 0usize;
    while let Some(&next) = subst.get(&r) {
        if next == r || hops > subst.len() {
            break;
        }
        r = next;
        hops += 1;
    }
    r
}

#[inline]
pub(super) fn rw(subst: &HashMap<Val8, Val8>, r: Val8) -> Val8 {
    resolve(subst, r)
}

pub(super) fn rw_addr(subst: &HashMap<Val8, Val8>, a: Addr) -> Addr {
    Addr::new(rw(subst, a.lo), rw(subst, a.hi))
}

pub(super) fn rw_word(subst: &HashMap<Val8, Val8>, w: Word) -> Word {
    // TODO(i64): rewrite logic is currently specialized to 4-lane (32-bit) words.
    Word::new(
        rw(subst, w.b0),
        rw(subst, w.b1),
        rw(subst, w.b2),
        rw(subst, w.b3),
    )
}

pub(super) fn rewrite_inst(mut inst: Inst8, subst: &HashMap<Val8, Val8>) -> Inst8 {
    inst.kind = match inst.kind {
        Inst8Kind::GlobalGetByte { .. } | Inst8Kind::Getchar => inst.kind,
        Inst8Kind::Copy(s) => Inst8Kind::Copy(rw(subst, s)),
        Inst8Kind::Add32Byte { lhs, rhs, lane } => Inst8Kind::Add32Byte {
            lhs: rw_word(subst, lhs),
            rhs: rw_word(subst, rhs),
            lane,
        },
        Inst8Kind::Sub32Byte { lhs, rhs, lane } => Inst8Kind::Sub32Byte {
            lhs: rw_word(subst, lhs),
            rhs: rw_word(subst, rhs),
            lane,
        },
        Inst8Kind::Sub32Borrow { lhs, rhs } => Inst8Kind::Sub32Borrow {
            lhs: rw_word(subst, lhs),
            rhs: rw_word(subst, rhs),
        },
        Inst8Kind::Add(l, r) => Inst8Kind::Add(rw(subst, l), rw(subst, r)),
        Inst8Kind::Carry(l, r) => Inst8Kind::Carry(rw(subst, l), rw(subst, r)),
        Inst8Kind::Sub(l, r) => Inst8Kind::Sub(rw(subst, l), rw(subst, r)),
        Inst8Kind::MulLo(l, r) => Inst8Kind::MulLo(rw(subst, l), rw(subst, r)),
        Inst8Kind::MulHi(l, r) => Inst8Kind::MulHi(rw(subst, l), rw(subst, r)),
        Inst8Kind::And8(l, r) => Inst8Kind::And8(rw(subst, l), rw(subst, r)),
        Inst8Kind::Or8(l, r) => Inst8Kind::Or8(rw(subst, l), rw(subst, r)),
        Inst8Kind::Xor8(l, r) => Inst8Kind::Xor8(rw(subst, l), rw(subst, r)),
        Inst8Kind::Eq(l, r) => Inst8Kind::Eq(rw(subst, l), rw(subst, r)),
        Inst8Kind::Ne(l, r) => Inst8Kind::Ne(rw(subst, l), rw(subst, r)),
        Inst8Kind::LtU(l, r) => Inst8Kind::LtU(rw(subst, l), rw(subst, r)),
        Inst8Kind::GeU(l, r) => Inst8Kind::GeU(rw(subst, l), rw(subst, r)),
        Inst8Kind::BoolAnd(op) => Inst8Kind::BoolAnd(op.map_vals(|val| rw(subst, val))),
        Inst8Kind::BoolOr(op) => Inst8Kind::BoolOr(op.map_vals(|val| rw(subst, val))),
        Inst8Kind::BoolNot(v) => Inst8Kind::BoolNot(rw(subst, v)),
        Inst8Kind::Sel(c, t, f) => Inst8Kind::Sel(rw(subst, c), rw(subst, t), rw(subst, f)),

        Inst8Kind::GlobalSetByte {
            global_idx,
            lane,
            val,
        } => Inst8Kind::GlobalSetByte {
            global_idx,
            lane,
            val: rw(subst, val),
        },
        Inst8Kind::LoadMem { base, addr, lane } => Inst8Kind::LoadMem {
            base,
            addr: rw_addr(subst, addr),
            lane,
        },
        Inst8Kind::StoreMem {
            base,
            addr,
            lane,
            val,
        } => Inst8Kind::StoreMem {
            base,
            addr: rw_addr(subst, addr),
            lane,
            val: rw(subst, val),
        },
        Inst8Kind::Putchar(v) => Inst8Kind::Putchar(rw(subst, v)),

        Inst8Kind::CsStore { offset, val } => Inst8Kind::CsStore {
            offset,
            val: rw(subst, val),
        },
        Inst8Kind::CsLoad { .. }
        | Inst8Kind::CsStorePc { .. }
        | Inst8Kind::CsLoadPc { .. }
        | Inst8Kind::CsAlloc(_)
        | Inst8Kind::CsFree(_) => inst.kind,

        Inst8Kind::ExcFlagSet { val } => Inst8Kind::ExcFlagSet { val: rw(subst, val) },
        Inst8Kind::ExcTagSet { lane, val } => Inst8Kind::ExcTagSet {
            lane,
            val: rw(subst, val),
        },
        Inst8Kind::ExcPayloadSet { lane, val } => Inst8Kind::ExcPayloadSet {
            lane,
            val: rw(subst, val),
        },
        Inst8Kind::ExcFlagGet
        | Inst8Kind::ExcTagGet { .. }
        | Inst8Kind::ExcPayloadGet { .. } => inst.kind,
    };
    inst
}

pub(super) fn rewrite_term(term: &mut Terminator8, subst: &HashMap<Val8, Val8>) {
    match term {
        Terminator8::Goto(_) | Terminator8::Trap(_) => {}
        Terminator8::Branch { cond, .. } => *cond = rw(subst, *cond),
        Terminator8::Switch { index, .. } => *index = rw(subst, *index),
        Terminator8::Return { val } => {
            if let Some(w) = val {
                *w = rw_word(subst, *w);
            }
        }
        Terminator8::Exit { val } => {
            if let Some(w) = val {
                *w = rw_word(subst, *w);
            }
        }
        Terminator8::CallSetup { args, .. } => {
            for w in args.iter_mut() {
                *w = rw_word(subst, *w);
            }
        }
    }
}

pub(super) fn has_side_effect(kind: &Inst8Kind) -> bool {
    matches!(
        kind,
        Inst8Kind::Putchar(_)
            | Inst8Kind::Getchar
            | Inst8Kind::StoreMem { .. }
            | Inst8Kind::GlobalSetByte { .. }
            | Inst8Kind::CsStore { .. }
            | Inst8Kind::CsStorePc { .. }
            | Inst8Kind::CsLoadPc { .. }
            | Inst8Kind::CsAlloc(_)
            | Inst8Kind::CsFree(_)
            | Inst8Kind::ExcFlagSet { .. }
            | Inst8Kind::ExcTagSet { .. }
            | Inst8Kind::ExcPayloadSet { .. }
    )
}

pub(super) fn inst_uses(kind: &Inst8Kind, live: &mut HashSet<Val8>) {
    let add_use = |live: &mut HashSet<Val8>, val: Val8| {
        if !val.is_imm() {
            live.insert(val);
        }
    };

    match kind {
        Inst8Kind::Getchar | Inst8Kind::GlobalGetByte { .. } => {}

        Inst8Kind::Copy(s) | Inst8Kind::BoolNot(s) | Inst8Kind::Putchar(s) => {
            add_use(live, *s);
        }
        Inst8Kind::Add32Byte { lhs, rhs, lane } | Inst8Kind::Sub32Byte { lhs, rhs, lane } => {
            for reg in lhs.uses_through_lane(*lane) {
                add_use(live, reg);
            }
            for reg in rhs.uses_through_lane(*lane) {
                add_use(live, reg);
            }
        }
        Inst8Kind::Sub32Borrow { lhs, rhs } => {
            for reg in lhs.uses_through_lane(3) {
                add_use(live, reg);
            }
            for reg in rhs.uses_through_lane(3) {
                add_use(live, reg);
            }
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
            add_use(live, *l);
            add_use(live, *r);
        }
        Inst8Kind::BoolAnd(op) | Inst8Kind::BoolOr(op) => {
            for &val in op.as_slice() {
                add_use(live, val);
            }
        }
        Inst8Kind::Sel(c, l, r) => {
            add_use(live, *l);
            add_use(live, *r);
            add_use(live, *c);
        }
        Inst8Kind::GlobalSetByte { val, .. } => {
            add_use(live, *val);
        }

        Inst8Kind::LoadMem { addr, .. } => {
            add_use(live, addr.lo);
            add_use(live, addr.hi);
        }

        Inst8Kind::StoreMem { addr, val, .. } => {
            add_use(live, addr.lo);
            add_use(live, addr.hi);
            add_use(live, *val);
        }

        Inst8Kind::CsStore { val, .. } => {
            add_use(live, *val);
        }
        Inst8Kind::CsLoad { .. }
        | Inst8Kind::CsStorePc { .. }
        | Inst8Kind::CsLoadPc { .. }
        | Inst8Kind::CsAlloc(_)
        | Inst8Kind::CsFree(_) => {}

        Inst8Kind::ExcFlagSet { val }
        | Inst8Kind::ExcTagSet { val, .. }
        | Inst8Kind::ExcPayloadSet { val, .. } => {
            add_use(live, *val);
        }
        Inst8Kind::ExcFlagGet
        | Inst8Kind::ExcTagGet { .. }
        | Inst8Kind::ExcPayloadGet { .. } => {}
    }
}

pub(super) fn term_uses(term: &Terminator8, live: &mut HashSet<Val8>) {
    match term {
        Terminator8::Goto(_) | Terminator8::Trap(_) => {}
        Terminator8::Branch { cond, .. } => {
            live.insert(*cond);
        }
        Terminator8::Switch { index, .. } => {
            live.insert(*index);
        }
        Terminator8::Return { val } | Terminator8::Exit { val } => {
            if let Some(w) = val {
                live.extend(w.bytes().into_iter().filter(|r| !r.is_imm()));
            }
        }
        Terminator8::CallSetup { args, .. } => {
            live.extend(
                args.iter()
                    .flat_map(|w| w.bytes().into_iter())
                    .filter(|r| !r.is_imm()),
            );
        }
    }
}
