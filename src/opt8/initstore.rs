//! Drop stores that write a byte's initial value while it provably still
//! holds it.
//!
//! The entry function's entry block runs exactly once, at start, when memory
//! is exactly the initial image. A forward pass over the blocks of the entry
//! function tracks which constant byte addresses may have been modified
//! since; a store of the initial value to an unmodified byte is a no-op.
//! Typical source: zero-initialising globals and stack frames in `main`'s
//! prologue, which otherwise also inflates the per-cycle store slot count
//! for the whole program.

use super::*;
use crate::ir8::CallTarget;

/// Bytes that may differ from the initial image.
#[derive(Clone, PartialEq, Eq)]
enum Modified {
    Some(HashSet<u32>),
    All,
}

impl Modified {
    fn join(&mut self, other: &Modified) -> bool {
        match (&mut *self, other) {
            (Modified::All, _) => false,
            (_, Modified::All) => {
                *self = Modified::All;
                true
            }
            (Modified::Some(a), Modified::Some(b)) => {
                let before = a.len();
                a.extend(b.iter().copied());
                a.len() != before
            }
        }
    }

    fn contains(&self, at: u32) -> bool {
        match self {
            Modified::All => true,
            Modified::Some(s) => s.contains(&at),
        }
    }

    fn insert(&mut self, at: u32) {
        if let Modified::Some(s) = self {
            s.insert(at);
        }
    }
}

pub(super) fn drop_initial_stores(prog: &mut Ir8Program) {
    let Some(&entry) = prog.func_entries.get(prog.entry_func as usize) else {
        return;
    };
    let fi = prog.entry_func as usize;
    // Stores at or beyond the emitted memory size trap: keep them.
    let mem_limit = prog.memory_end;

    // Blocks entered from outside the entry function's own control flow
    // (call targets, saved return addresses, other functions' jumps) start
    // with unknown memory.
    let mut external: HashSet<Pc> = HashSet::new();
    for (fj, blocks) in prog.func_blocks.iter().enumerate() {
        for bb in blocks {
            for inst in &bb.insts {
                if let Inst8Kind::CsStorePc { val, .. } = inst.kind {
                    external.insert(val);
                }
            }
            if let Terminator8::CallSetup {
                callee_entry: CallTarget::Pc(c),
                ..
            } = bb.terminator
            {
                external.insert(c);
            }
            if fj != fi {
                external.extend(bb.terminator.successors());
            }
        }
    }
    if external.contains(&entry) {
        return;
    }

    let init_byte = |at: u32| prog.init_bytes.get(at as usize).copied().unwrap_or(0);
    let idx: HashMap<Pc, usize> = prog.func_blocks[fi]
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect();

    // Fixpoint over block entry states.
    let mut state_in: HashMap<Pc, Modified> = idx
        .keys()
        .filter(|pc| external.contains(pc))
        .map(|&pc| (pc, Modified::All))
        .collect();
    state_in.insert(entry, Modified::Some(HashSet::new()));
    let transfer = |bb: &BasicBlock8, mut m: Modified, removable: &mut Vec<usize>| {
        for (k, inst) in bb.insts.iter().enumerate() {
            match inst.kind {
                Inst8Kind::StoreMem {
                    base,
                    addr,
                    lane,
                    val,
                } => match addr.imm_value() {
                    Some(a) => {
                        let at = a + mem_byte_offset(base, lane);
                        if at < mem_limit
                            && !m.contains(at)
                            && val.imm_value() == Some(init_byte(at))
                        {
                            removable.push(k);
                        } else {
                            m.insert(at);
                        }
                    }
                    None => m = Modified::All,
                },
                Inst8Kind::MemInc32 { base, addr, .. } => match addr.imm_value() {
                    Some(a) => (0..4).for_each(|j| m.insert(a + u32::from(base) + j)),
                    None => m = Modified::All,
                },
                _ => {}
            }
        }
        // A call may store anywhere before control comes back.
        if matches!(bb.terminator, Terminator8::CallSetup { .. }) {
            m = Modified::All;
        }
        m
    };
    // Every seeded block must run: an `All` seed only reaches the blocks
    // after it (e.g. a loop header re-entered after a call) through it.
    let mut work: Vec<Pc> = state_in.keys().copied().collect();
    while let Some(pc) = work.pop() {
        let Some(&i) = idx.get(&pc) else { continue };
        let bb = &prog.func_blocks[fi][i];
        let out = transfer(bb, state_in[&pc].clone(), &mut Vec::new());
        for succ in bb.terminator.successors() {
            if !idx.contains_key(&succ) {
                continue;
            }
            let changed = match state_in.get_mut(&succ) {
                Some(s) => s.join(&out),
                None => {
                    state_in.insert(succ, out.clone());
                    true
                }
            };
            if changed {
                work.push(succ);
            }
        }
    }

    for bb in prog.func_blocks[fi].iter_mut() {
        let Some(m) = state_in.get(&bb.id).cloned() else {
            continue;
        };
        let mut removable = Vec::new();
        transfer(bb, m, &mut removable);
        if removable.is_empty() {
            continue;
        }
        let mut keep = vec![true; bb.insts.len()];
        for k in removable {
            keep[k] = false;
        }
        filter_by_mask(&mut bb.insts, &keep);
    }
}
