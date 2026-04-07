use std::collections::HashMap;

use anyhow::{Context, bail};

use crate::constants::{
    SCHEDULE_MAX_COMPLEXITY_PER_CYCLE, SCHEDULE_MAX_OPS_PER_CYCLE, SCHEDULE_MAX_STORE_MEM_PER_CYCLE,
};
use crate::ir8::{
    Addr, BUILTIN_PC_BASE, Cycle, Inst8, Inst8Kind, Ir8Program, PC_STRIDE, Pc, Terminator8,
};

#[derive(Clone)]
struct ScheduledBlock {
    old_pc: Pc,
    op_groups: Vec<Vec<Inst8>>,
    terminator: Terminator8,
    new_pcs: Vec<Pc>,
}

#[derive(Default)]
struct PendingCycle {
    ops: Vec<Inst8>,
    has_putchar: bool,
    complexity: usize,
    mem_store_count: usize,
}

impl PendingCycle {
    fn can_append(&self, inst: &Inst8, profile: InstProfile) -> bool {
        !self.ops.is_empty()
            && !profile.is_getchar
            && (!profile.is_putchar || !self.has_putchar)
            && (self.has_mem_stores() == profile.is_store_mem)
            && (!profile.is_store_mem || self.mem_store_count < SCHEDULE_MAX_STORE_MEM_PER_CYCLE)
            && (self.ops.len() < SCHEDULE_MAX_OPS_PER_CYCLE)
            && (self.complexity + profile.complexity <= SCHEDULE_MAX_COMPLEXITY_PER_CYCLE)
            && self.ops.iter().all(|prev| independent(prev, inst))
    }

    fn push(&mut self, inst: Inst8, profile: InstProfile) {
        self.has_putchar |= profile.is_putchar;
        self.mem_store_count += usize::from(profile.is_store_mem);
        self.complexity += profile.complexity;
        self.ops.push(inst);
    }

    fn take(&mut self) -> Vec<Inst8> {
        let ops = std::mem::take(&mut self.ops);
        *self = Self::default();
        ops
    }

    fn has_mem_stores(&self) -> bool {
        self.mem_store_count != 0
    }
}

#[derive(Clone, Copy)]
struct InstProfile {
    is_getchar: bool,
    is_putchar: bool,
    is_store_mem: bool,
    complexity: usize,
}

impl InstProfile {
    fn from_inst(inst: &Inst8) -> Self {
        let kind = &inst.kind;
        Self {
            is_getchar: matches!(kind, Inst8Kind::Getchar),
            is_putchar: matches!(kind, Inst8Kind::Putchar(_)),
            is_store_mem: matches!(kind, Inst8Kind::StoreMem { .. }),
            complexity: inst_complexity(kind),
        }
    }
}

pub fn schedule(ir8: &mut Ir8Program) -> anyhow::Result<()> {
    let mut scheduled_funcs: Vec<Vec<ScheduledBlock>> = Vec::with_capacity(ir8.func_blocks.len());
    let mut first_pc_map: HashMap<Pc, Pc> = HashMap::new();

    for (func_id, blocks) in ir8.func_blocks.iter().enumerate() {
        if blocks.is_empty() {
            scheduled_funcs.push(Vec::new());
            continue;
        }

        let mut scheduled_blocks = Vec::with_capacity(blocks.len());
        for block in blocks {
            let mut groups = schedule_block_ops(&block.insts);
            if groups.is_empty() {
                groups.push(Vec::new());
            }
            scheduled_blocks.push(ScheduledBlock {
                old_pc: block.id,
                op_groups: groups,
                terminator: block.terminator.clone(),
                new_pcs: Vec::new(),
            });
        }

        let mut next_local_pc: u16 = 0;
        for block in &mut scheduled_blocks {
            block.new_pcs = Vec::with_capacity(block.op_groups.len());
            for _ in 0..block.op_groups.len() {
                if next_local_pc >= PC_STRIDE {
                    bail!("function {} exceeds PC_STRIDE while scheduling", func_id);
                }
                let pc = Pc::new(func_id as u16 * PC_STRIDE + next_local_pc);
                next_local_pc += 1;
                block.new_pcs.push(pc);
            }
            first_pc_map.insert(block.old_pc, block.new_pcs[0]);
        }

        scheduled_funcs.push(scheduled_blocks);
    }

    let mut cycles: Vec<Cycle> = scheduled_funcs
        .iter()
        .flat_map(|scheduled_blocks| scheduled_blocks.iter())
        .flat_map(|block| {
            block
                .op_groups
                .iter()
                .enumerate()
                .map(|(i, ops)| -> anyhow::Result<Cycle> {
                    let term = if i + 1 < block.op_groups.len() {
                        Terminator8::Goto(block.new_pcs[i + 1])
                    } else {
                        rewrite_term_pcs(block.terminator.clone(), &first_pc_map)?
                    };
                    let ops = ops
                        .iter()
                        .cloned()
                        .map(|inst| rewrite_inst_pcs(inst, &first_pc_map))
                        .collect::<anyhow::Result<Vec<_>>>()?;

                    Ok(Cycle {
                        pc: block.new_pcs[i],
                        ops,
                        terminator: term,
                    })
                })
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    cycles.sort_by_key(|c| c.pc.index());
    ir8.cycles = cycles;
    Ok(())
}

fn schedule_block_ops(insts: &[Inst8]) -> Vec<Vec<Inst8>> {
    if insts.is_empty() {
        return Vec::new();
    }

    let mut cycles: Vec<Vec<Inst8>> = Vec::new();
    let mut current = PendingCycle::default();

    for inst in insts {
        let profile = InstProfile::from_inst(inst);

        if current.can_append(inst, profile) {
            current.push(inst.clone(), profile);
            continue;
        }

        if !current.ops.is_empty() {
            cycles.push(current.take());
        }

        // Blocking input operation: dedicate a full cycle to getchar.
        if profile.is_getchar {
            cycles.push(vec![inst.clone()]);
            continue;
        }

        current.push(inst.clone(), profile);
    }

    if !current.ops.is_empty() {
        cycles.push(current.take());
    }

    cycles
}

fn inst_complexity(kind: &Inst8Kind) -> usize {
    match kind {
        Inst8Kind::MulLo(_, _) | Inst8Kind::MulHi(_, _) => 3,
        Inst8Kind::LoadMem { .. } | Inst8Kind::StoreMem { .. } => 3,
        Inst8Kind::GlobalGetByte { .. } | Inst8Kind::GlobalSetByte { .. } => 2,
        Inst8Kind::CsStore { .. }
        | Inst8Kind::CsLoad { .. }
        | Inst8Kind::CsStorePc { .. }
        | Inst8Kind::CsLoadPc { .. }
        | Inst8Kind::CsAlloc(_)
        | Inst8Kind::CsFree(_) => 2,
        Inst8Kind::Getchar | Inst8Kind::Putchar(_) => 2,
        _ => 1,
    }
}

fn independent(a: &Inst8, b: &Inst8) -> bool {
    no_register_hazard(a, b) && !effect_order_dependent(&a.kind, &b.kind)
}

fn no_register_hazard(a: &Inst8, b: &Inst8) -> bool {
    let uses_a = a.uses();
    let defs_a = a.defs();
    let uses_b = b.uses();
    let defs_b = b.defs();

    !defs_a
        .iter()
        .any(|r| defs_b.contains(r) || uses_b.contains(r))
        && !defs_b.iter().any(|r| uses_a.contains(r))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MemAccess {
    Read { addr: Addr, byte_offset: u32 },
    Write { addr: Addr, byte_offset: u32 },
}

impl MemAccess {
    fn is_write(self) -> bool {
        matches!(self, Self::Write { .. })
    }

    fn addr(self) -> Addr {
        match self {
            Self::Read { addr, .. } | Self::Write { addr, .. } => addr,
        }
    }

    fn byte_offset(self) -> u32 {
        match self {
            Self::Read { byte_offset, .. } | Self::Write { byte_offset, .. } => byte_offset,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GlobalAccess {
    Read { global_idx: u32, lane: u8 },
    Write { global_idx: u32, lane: u8 },
}

impl GlobalAccess {
    fn is_write(self) -> bool {
        matches!(self, Self::Write { .. })
    }

    fn key(self) -> (u32, u8) {
        match self {
            Self::Read { global_idx, lane } | Self::Write { global_idx, lane } => {
                (global_idx, lane)
            }
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CsAccess {
    Read { slot: u16, lane_mask: u8 },
    Write { slot: u16, lane_mask: u8 },
    StackPtrAdjust,
}

fn effect_order_dependent(a: &Inst8Kind, b: &Inst8Kind) -> bool {
    if let (Some(lhs), Some(rhs)) = (cs_access(a), cs_access(b)) {
        return match (lhs, rhs) {
            (CsAccess::StackPtrAdjust, _) | (_, CsAccess::StackPtrAdjust) => true,
            (CsAccess::Read { .. }, CsAccess::Read { .. }) => false,
            (
                CsAccess::Read {
                    slot: l_slot,
                    lane_mask: l_mask,
                },
                CsAccess::Write {
                    slot: r_slot,
                    lane_mask: r_mask,
                },
            )
            | (
                CsAccess::Write {
                    slot: l_slot,
                    lane_mask: l_mask,
                },
                CsAccess::Read {
                    slot: r_slot,
                    lane_mask: r_mask,
                },
            )
            | (
                CsAccess::Write {
                    slot: l_slot,
                    lane_mask: l_mask,
                },
                CsAccess::Write {
                    slot: r_slot,
                    lane_mask: r_mask,
                },
            ) => l_slot == r_slot && (l_mask & r_mask) != 0,
        };
    }

    // Keep I/O operations ordered with each other.
    if is_io_op(a) && is_io_op(b) {
        return true;
    }

    if let (Some(lhs), Some(rhs)) = (mem_access(a), mem_access(b)) {
        if !(lhs.is_write() || rhs.is_write()) {
            return false;
        }

        // If both instructions use the same runtime address expression, we can
        // prove disjointness byte-by-byte and keep only true byte conflicts ordered.
        if lhs.addr() == rhs.addr() {
            return lhs.byte_offset() == rhs.byte_offset();
        }

        // may alias
        return true;
    }

    if let (Some(lhs), Some(rhs)) = (global_access(a), global_access(b)) {
        if !(lhs.is_write() || rhs.is_write()) {
            return false;
        }
        return lhs.key() == rhs.key();
    }

    false
}

fn mem_access(kind: &Inst8Kind) -> Option<MemAccess> {
    match kind {
        Inst8Kind::LoadMem { base, addr, lane } => Some(MemAccess::Read {
            addr: *addr,
            byte_offset: (*base as u32) + (*lane as u32),
        }),
        Inst8Kind::StoreMem {
            base, addr, lane, ..
        } => Some(MemAccess::Write {
            addr: *addr,
            byte_offset: (*base as u32) + (*lane as u32),
        }),
        _ => None,
    }
}

fn global_access(kind: &Inst8Kind) -> Option<GlobalAccess> {
    match kind {
        Inst8Kind::GlobalGetByte { global_idx, lane } => Some(GlobalAccess::Read {
            global_idx: *global_idx,
            lane: *lane,
        }),
        Inst8Kind::GlobalSetByte {
            global_idx, lane, ..
        } => Some(GlobalAccess::Write {
            global_idx: *global_idx,
            lane: *lane,
        }),
        _ => None,
    }
}

fn cs_access(kind: &Inst8Kind) -> Option<CsAccess> {
    fn byte_access(offset: u16, is_write: bool) -> CsAccess {
        let slot = offset / 2;
        let lane_mask = if offset.is_multiple_of(2) { 0b01 } else { 0b10 };
        if is_write {
            CsAccess::Write { slot, lane_mask }
        } else {
            CsAccess::Read { slot, lane_mask }
        }
    }

    Some(match kind {
        Inst8Kind::CsStore { offset, .. } => byte_access(*offset, true),
        Inst8Kind::CsLoad { offset } => byte_access(*offset, false),
        Inst8Kind::CsStorePc { offset, .. } => CsAccess::Write {
            slot: *offset,
            lane_mask: 0b11,
        },
        Inst8Kind::CsLoadPc { offset } => CsAccess::Read {
            slot: *offset,
            lane_mask: 0b11,
        },
        Inst8Kind::CsAlloc(_) | Inst8Kind::CsFree(_) => CsAccess::StackPtrAdjust,
        _ => return None,
    })
}

fn is_io_op(kind: &Inst8Kind) -> bool {
    matches!(kind, Inst8Kind::Getchar | Inst8Kind::Putchar(_))
}

fn rewrite_term_pcs(
    term: Terminator8,
    first_pc_map: &HashMap<Pc, Pc>,
) -> anyhow::Result<Terminator8> {
    Ok(match term {
        Terminator8::Goto(pc) => Terminator8::Goto(map_target_pc(pc, first_pc_map)?),

        Terminator8::Branch {
            cond,
            if_true,
            if_false,
        } => Terminator8::Branch {
            cond,
            if_true: map_target_pc(if_true, first_pc_map)?,
            if_false: map_target_pc(if_false, first_pc_map)?,
        },

        Terminator8::Switch {
            index,
            targets,
            default,
        } => Terminator8::Switch {
            index,
            targets: targets
                .into_iter()
                .map(|pc| map_target_pc(pc, first_pc_map))
                .collect::<anyhow::Result<Vec<_>>>()?,
            default: map_target_pc(default, first_pc_map)?,
        },

        Terminator8::CallSetup {
            callee_entry,
            cont,
            args,
            callee_arg_vregs,
        } => Terminator8::CallSetup {
            callee_entry: map_target_pc(callee_entry, first_pc_map)?,
            cont: map_target_pc(cont, first_pc_map)?,
            args,
            callee_arg_vregs,
        },

        Terminator8::Return { val } => Terminator8::Return { val },
        Terminator8::Exit { val } => Terminator8::Exit { val },
        Terminator8::Trap(code) => Terminator8::Trap(code),
    })
}

fn rewrite_inst_pcs(inst: Inst8, first_pc_map: &HashMap<Pc, Pc>) -> anyhow::Result<Inst8> {
    Ok(match inst.kind {
        Inst8Kind::CsStorePc { offset, val } => Inst8::no_dst(Inst8Kind::CsStorePc {
            offset,
            val: map_target_pc(val, first_pc_map)?,
        }),
        _ => inst,
    })
}

fn map_target_pc(pc: Pc, first_pc_map: &HashMap<Pc, Pc>) -> anyhow::Result<Pc> {
    if pc.index() >= BUILTIN_PC_BASE {
        return Ok(pc);
    }

    first_pc_map
        .get(&pc)
        .copied()
        .with_context(|| format!("missing scheduled mapping for target pc {}", pc.index()))
}

#[cfg(test)]
mod tests {
    use super::{schedule, schedule_block_ops};
    use crate::ir8::{
        Addr, BasicBlock8, FrameInfo, Inst8, Inst8Kind, Ir8Program, MemoryLayout, Pc, Terminator8,
        Val8, Word,
    };

    fn r(i: u16) -> Val8 {
        Val8::vreg(i)
    }

    #[test]
    fn schedule_allows_cs_stores_to_different_slots() {
        let insts = vec![
            Inst8::no_dst(Inst8Kind::CsStore {
                offset: 0,
                val: r(10),
            }),
            Inst8::no_dst(Inst8Kind::CsStore {
                offset: 1,
                val: r(11),
            }),
            Inst8::no_dst(Inst8Kind::CsStorePc {
                offset: 2,
                val: Pc::new(99),
            }),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].len(), 3);
    }

    #[test]
    fn schedule_allows_cs_loads_to_different_slots() {
        let insts = vec![
            Inst8::with_dst(r(20), Inst8Kind::CsLoad { offset: 0 }),
            Inst8::with_dst(r(21), Inst8Kind::CsLoad { offset: 1 }),
            Inst8::with_dst(r(22), Inst8Kind::CsLoad { offset: 2 }),
            Inst8::with_dst(r(23), Inst8Kind::CsLoad { offset: 3 }),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].len(), 4);
    }

    #[test]
    fn schedule_orders_cs_write_then_read_same_slot() {
        let insts = vec![
            Inst8::no_dst(Inst8Kind::CsStore {
                offset: 5,
                val: r(10),
            }),
            Inst8::with_dst(r(11), Inst8Kind::CsLoad { offset: 5 }),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0].len(), 1);
        assert_eq!(cycles[1].len(), 1);
    }

    #[test]
    fn schedule_orders_cs_sp_adjustments_with_stack_accesses() {
        let insts = vec![
            Inst8::no_dst(Inst8Kind::CsFree(1)),
            Inst8::no_dst(Inst8Kind::CsLoadPc { offset: 0 }),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0].len(), 1);
        assert_eq!(cycles[1].len(), 1);
    }

    #[test]
    fn schedule_orders_cs_store_pc_then_cs_load_byte_same_slot() {
        let insts = vec![
            Inst8::no_dst(Inst8Kind::CsStorePc {
                offset: 1,
                val: Pc::new(100),
            }),
            // byte offset 2 maps to slot 1, low lane
            Inst8::with_dst(r(20), Inst8Kind::CsLoad { offset: 2 }),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0].len(), 1);
        assert_eq!(cycles[1].len(), 1);
    }

    #[test]
    fn schedule_orders_cs_store_byte_then_cs_load_pc_same_slot() {
        let insts = vec![
            // byte offset 3 maps to slot 1, high lane
            Inst8::no_dst(Inst8Kind::CsStore {
                offset: 3,
                val: r(10),
            }),
            Inst8::no_dst(Inst8Kind::CsLoadPc { offset: 1 }),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0].len(), 1);
        assert_eq!(cycles[1].len(), 1);
    }

    #[test]
    fn schedule_allows_cs_store_pc_then_cs_load_byte_different_slot() {
        let insts = vec![
            Inst8::no_dst(Inst8Kind::CsStorePc {
                offset: 1,
                val: Pc::new(100),
            }),
            // byte offset 4 maps to slot 2, low lane
            Inst8::with_dst(r(20), Inst8Kind::CsLoad { offset: 4 }),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].len(), 2);
    }

    #[test]
    fn schedule_allows_byte_disjoint_store_pattern() {
        let addr = Addr::new(r(10), r(11));
        let insts = vec![
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 0,
                val: r(20),
            }),
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 1,
                val: r(21),
            }),
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 2,
                addr,
                lane: 0,
                val: r(22),
            }),
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 2,
                addr,
                lane: 1,
                val: r(23),
            }),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].len(), 4);
    }

    #[test]
    fn schedule_splits_compute_from_mem_store_cycle() {
        let addr = Addr::new(r(10), r(11));
        let insts = vec![
            Inst8::with_dst(r(20), Inst8Kind::Add(r(1), r(2))),
            Inst8::with_dst(
                r(21),
                Inst8Kind::Add32Byte {
                    lhs: Word::new(r(3), r(4), r(5), r(6)),
                    rhs: Word::new(r(7), r(8), r(9), r(10)),
                    lane: 3,
                },
            ),
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 0,
                val: r(20),
            }),
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 1,
                val: r(21),
            }),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0].len(), 2);
        assert_eq!(cycles[1].len(), 2);
        assert!(matches!(cycles[1][0].kind, Inst8Kind::StoreMem { .. }));
        assert!(matches!(cycles[1][1].kind, Inst8Kind::StoreMem { .. }));
    }

    #[test]
    fn schedule_splits_mem_store_and_putchar_cycles() {
        let addr = Addr::new(r(10), r(11));
        let insts = vec![
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 0,
                val: r(20),
            }),
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 1,
                val: r(21),
            }),
            Inst8::no_dst(Inst8Kind::Putchar(r(20))),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0].len(), 2);
        assert_eq!(cycles[1].len(), 1);
        assert!(matches!(cycles[1][0].kind, Inst8Kind::Putchar(_)));
    }

    #[test]
    fn schedule_orders_store_store_same_byte() {
        let addr = Addr::new(r(10), r(11));
        let insts = vec![
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 0,
                val: r(20),
            }),
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 0,
                val: r(21),
            }),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0].len(), 1);
        assert_eq!(cycles[1].len(), 1);
    }

    #[test]
    fn schedule_orders_store_load_for_unknown_alias() {
        let insts = vec![
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr: Addr::new(r(10), r(11)),
                lane: 0,
                val: r(20),
            }),
            Inst8::with_dst(
                r(30),
                Inst8Kind::LoadMem {
                    base: 0,
                    addr: Addr::new(r(12), r(13)),
                    lane: 0,
                },
            ),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0].len(), 1);
        assert_eq!(cycles[1].len(), 1);
    }

    #[test]
    fn schedule_allows_disjoint_global_writes() {
        let insts = vec![
            Inst8::no_dst(Inst8Kind::GlobalSetByte {
                global_idx: 0,
                lane: 0,
                val: r(10),
            }),
            Inst8::no_dst(Inst8Kind::GlobalSetByte {
                global_idx: 0,
                lane: 1,
                val: r(11),
            }),
            Inst8::no_dst(Inst8Kind::GlobalSetByte {
                global_idx: 0,
                lane: 2,
                val: r(12),
            }),
            Inst8::no_dst(Inst8Kind::GlobalSetByte {
                global_idx: 0,
                lane: 3,
                val: r(13),
            }),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].len(), 4);
        assert!(matches!(
            cycles[0][0].kind,
            Inst8Kind::GlobalSetByte { lane: 0, .. }
        ));
        assert!(matches!(
            cycles[0][1].kind,
            Inst8Kind::GlobalSetByte { lane: 1, .. }
        ));
        assert!(matches!(
            cycles[0][2].kind,
            Inst8Kind::GlobalSetByte { lane: 2, .. }
        ));
        assert!(matches!(
            cycles[0][3].kind,
            Inst8Kind::GlobalSetByte { lane: 3, .. }
        ));
    }

    #[test]
    fn schedule_orders_global_writes_to_same_lane() {
        let insts = vec![
            Inst8::no_dst(Inst8Kind::GlobalSetByte {
                global_idx: 0,
                lane: 0,
                val: r(10),
            }),
            Inst8::no_dst(Inst8Kind::GlobalSetByte {
                global_idx: 0,
                lane: 0,
                val: r(11),
            }),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 2);
        assert_eq!(cycles[0].len(), 1);
        assert_eq!(cycles[1].len(), 1);
    }

    #[test]
    fn schedule_packs_add32_byte_projections_into_one_cycle() {
        let lhs = Word::new(r(0), r(1), r(2), r(3));
        let rhs = Word::new(r(4), r(5), r(6), r(7));
        let insts = vec![
            Inst8::with_dst(r(20), Inst8Kind::Add32Byte { lhs, rhs, lane: 0 }),
            Inst8::with_dst(r(21), Inst8Kind::Add32Byte { lhs, rhs, lane: 1 }),
            Inst8::with_dst(r(22), Inst8Kind::Add32Byte { lhs, rhs, lane: 2 }),
            Inst8::with_dst(r(23), Inst8Kind::Add32Byte { lhs, rhs, lane: 3 }),
        ];

        let cycles = schedule_block_ops(&insts);
        assert_eq!(cycles.len(), 1);
        assert_eq!(cycles[0].len(), 4);
    }

    #[test]
    fn schedule_rewrites_cs_store_pc_target_after_split() {
        let mut prog = Ir8Program {
            entry_func: 0,
            num_vregs: 1,
            func_blocks: vec![vec![
                BasicBlock8 {
                    id: Pc::new(0),
                    insts: vec![
                        Inst8::no_dst(Inst8Kind::CsStorePc {
                            offset: 0,
                            val: Pc::new(1),
                        }),
                        Inst8::no_dst(Inst8Kind::Putchar(r(0))),
                        Inst8::no_dst(Inst8Kind::Putchar(r(0))),
                    ],
                    terminator: Terminator8::Goto(Pc::new(1)),
                },
                BasicBlock8 {
                    id: Pc::new(1),
                    insts: vec![],
                    terminator: Terminator8::Exit { val: None },
                },
            ]],
            cycles: Vec::new(),
            frame_infos: vec![FrameInfo {
                entry: Pc::new(0),
                num_locals: 0,
            }],
            memory_layout: MemoryLayout {
                memory_end: 0,
                init_bytes: Vec::new(),
            },
            global_init: Vec::new(),
        };

        schedule(&mut prog).expect("schedule should succeed");

        // After splitting the first block, old pc=1 remaps to new pc=2.
        let cs_store_val = prog.cycles[0]
            .ops
            .iter()
            .find_map(|op| match op.kind {
                Inst8Kind::CsStorePc { val, .. } => Some(val),
                _ => None,
            })
            .expect("expected CsStorePc in first cycle");
        assert_eq!(cs_store_val.index(), 2);
    }
}
