use super::*;
use crate::ir8::RET;

pub(super) struct FuncAlloc {
    pub(super) local_vregs: Vec<Word>,
}

pub(super) fn prealloc_locals(module: &IrModule) -> (Vec<FuncAlloc>, u16) {
    let mut counter = VREG_START;
    let allocs = module
        .bodies()
        .iter()
        .map(|body| {
            let n = body.as_ref().map_or(0, |b| b.locals().len());
            let local_vregs = (0..n)
                .map(|_| {
                    // TODO(i64): each local is currently represented as one 4-byte word.
                    let w = Word::new(
                        Val8::reg(counter),
                        Val8::reg(counter + 1),
                        Val8::reg(counter + 2),
                        Val8::reg(counter + 3),
                    );
                    counter += 4;
                    w
                })
                .collect();
            FuncAlloc { local_vregs }
        })
        .collect();
    (allocs, counter)
}

pub(super) fn alloc_builtin_div_params(counter: &mut u16) -> FuncAlloc {
    let word0 = Word::new(
        Val8::reg(*counter),
        Val8::reg(*counter + 1),
        Val8::reg(*counter + 2),
        Val8::reg(*counter + 3),
    );
    *counter += 4;
    let word1 = Word::new(
        Val8::reg(*counter),
        Val8::reg(*counter + 1),
        Val8::reg(*counter + 2),
        Val8::reg(*counter + 3),
    );
    *counter += 4;
    FuncAlloc {
        local_vregs: vec![word0, word1],
    }
}

pub(super) struct FuncBuilder {
    pub(super) func_id: u32,
    pub(super) is_entry: bool,
    pub(super) vreg_counter: u16,
    pub(super) local_vregs: Vec<Word>,
    pub(super) inst_map: HashMap<IrNode, Word>,
    pub(super) block_pc_map: HashMap<BlockId, Pc>,
    pub(super) curr_blk: Pc,
    pub(super) curr_insts: Vec<Inst8>,
    pub(super) blocks: Vec<BasicBlock8>,
    pub(super) next_blk_idx: u16,
}

impl FuncBuilder {
    pub(super) fn new(
        func_id: u32,
        is_entry: bool,
        vreg_counter: u16,
        local_vregs: Vec<Word>,
    ) -> Self {
        Self {
            func_id,
            is_entry,
            vreg_counter,
            local_vregs,
            inst_map: HashMap::new(),
            block_pc_map: HashMap::new(),
            curr_blk: Pc::new(0),
            curr_insts: Vec::new(),
            blocks: Vec::new(),
            next_blk_idx: 0,
        }
    }

    pub(super) fn alloc_reg(&mut self) -> Val8 {
        let v = Val8::reg(self.vreg_counter);
        self.vreg_counter += 1;
        v
    }

    pub(super) fn alloc_word(&mut self) -> Word {
        Word::new(
            self.alloc_reg(),
            self.alloc_reg(),
            self.alloc_reg(),
            self.alloc_reg(),
        )
    }

    pub(super) fn alloc_block(&mut self) -> Pc {
        let pc_value = self.func_id * u32::from(PC_STRIDE) + u32::from(self.next_blk_idx);
        assert!(
            pc_value <= u32::from(u16::MAX),
            "function {} ran out of block PCs (block {} would exceed u16 range)",
            self.func_id,
            self.next_blk_idx,
        );
        let pc = Pc::new(pc_value as u16);
        self.next_blk_idx += 1;
        pc
    }

    pub(super) fn switch_to(&mut self, pc: Pc) {
        debug_assert!(self.curr_insts.is_empty(), "previous block not finished");
        self.curr_blk = pc;
    }

    pub(super) fn emit(&mut self, inst: Inst8) {
        self.curr_insts.push(inst);
    }

    pub(super) fn finish(&mut self, term: Terminator8) {
        self.blocks.push(BasicBlock8 {
            id: self.curr_blk,
            insts: std::mem::take(&mut self.curr_insts),
            terminator: term,
        });
    }

    pub(super) fn get_word(&self, iref: IrNode) -> Word {
        if let Some(v) = iref.imm_i32_value() {
            // TODO(i64): immediate materialization assumes i32 literals.
            Word::from_u32_imm(v as u32)
        } else {
            self.inst_map[&iref]
        }
    }

    pub(super) fn set_word(&mut self, iref: IrNode, word: Word) {
        self.inst_map.insert(iref, word);
    }

    pub(super) fn copy_word(&mut self, dst: Word, src: Word) {
        for (dst_lane, src_lane) in dst.bytes().into_iter().zip(src.bytes()) {
            self.emit(Inst8::with_dst(dst_lane, Inst8Kind::Copy(src_lane)));
        }
    }

    pub(super) fn set_word_from_byte(&mut self, dst: Word, src: Val8) {
        let [lo, hi1, hi2, hi3] = dst.bytes();
        self.emit(Inst8::with_dst(lo, Inst8Kind::Copy(src)));
        for lane in [hi1, hi2, hi3] {
            self.emit(Inst8::with_dst(lane, Inst8Kind::Copy(Val8::imm(0))));
        }
    }

    pub(super) fn copy_ret_to_word(&mut self, dst: Word) {
        self.copy_word(dst, RET);
    }

    pub(super) fn set_ret_from_byte(&mut self, src: Val8) {
        self.set_word_from_byte(RET, src);
    }

    pub(super) fn load_global_word(&mut self, global_idx: u32) -> Word {
        let dst = self.alloc_word();
        for (lane, dst_lane) in dst.bytes().into_iter().enumerate() {
            self.emit(Inst8::with_dst(
                dst_lane,
                Inst8Kind::GlobalGetByte {
                    global_idx,
                    lane: lane as u8,
                },
            ));
        }
        dst
    }

    pub(super) fn store_global_word(&mut self, global_idx: u32, val: Word) {
        for (lane, src_lane) in val.bytes().into_iter().enumerate() {
            self.emit(Inst8::no_dst(Inst8Kind::GlobalSetByte {
                global_idx,
                lane: lane as u8,
                val: src_lane,
            }));
        }
    }

    fn emit_cs_store_word(&mut self, base: u16, word: Word) {
        for (lane, src) in word.bytes().into_iter().enumerate() {
            self.emit(Inst8::no_dst(Inst8Kind::CsStore {
                offset: base + lane as u16,
                val: src,
            }));
        }
    }

    fn emit_cs_load_word(&mut self, base: u16, word: Word) {
        for (lane, dst) in word.bytes().into_iter().enumerate() {
            self.emit(Inst8::with_dst(
                dst,
                Inst8Kind::CsLoad {
                    offset: base + lane as u16,
                },
            ));
        }
    }

    pub(super) fn local_get(&mut self, local_index: u32) -> Word {
        let src = self.local_vregs[local_index as usize];
        let dst = self.alloc_word();
        self.copy_word(dst, src);
        dst
    }

    pub(super) fn local_set(&mut self, local_index: u32, val: Word) {
        let dst = self.local_vregs[local_index as usize];
        self.copy_word(dst, val);
    }

    pub(super) fn pc_of(&self, ir_block: BlockId) -> Pc {
        self.block_pc_map[&ir_block]
    }

    /// Number of call stack slots needed for saving all locals.
    // TODO(i64): call-stack sizing assumes each local is a 32-bit value.
    /// Each i32 local uses two 16-bit slots.
    pub(super) fn cs_locals_slots(&self) -> u16 {
        self.local_vregs.len() as u16 * 2
    }

    /// Emit CsStore instructions to save locals, live spill words, and RA,
    /// then CsAlloc.
    ///
    /// Frame layout (slot-indexed, each slot = one 16-bit CSS property):
    ///   cs[cs_sp + 0 .. 2*N-1]          = saved locals (packed: two bytes/slot)
    ///   cs[cs_sp + 2*N .. +2*N+2*S-1]   = saved spill words
    ///   cs[cs_sp + 2*N + 2*S]           = RA (Pc)
    /// CsAlloc advances cs_sp by 2*N + 2*S + 1 slots.
    ///
    /// RA is at the top so the callee can always pop it with CsFree(1) +
    /// CsLoadPc(0) regardless of the caller's local count.
    pub(super) fn emit_cs_save(&mut self, cont: Pc, spill_words: &[Word]) {
        for i in 0..self.local_vregs.len() {
            self.emit_cs_store_word((i as u16) * 4, self.local_vregs[i]);
        }

        let spill_base_bytes = self.cs_locals_slots() * 2;
        for (i, word) in spill_words.iter().copied().enumerate() {
            self.emit_cs_store_word(spill_base_bytes + (i as u16) * 4, word);
        }

        let spill_slots = (spill_words.len() as u16) * 2;
        let ra_slot = self.cs_locals_slots() + spill_slots;
        self.emit(Inst8::no_dst(Inst8Kind::CsStorePc {
            offset: ra_slot,
            val: cont,
        }));
        self.emit(Inst8::no_dst(Inst8Kind::CsAlloc(ra_slot + 1)));
    }

    /// Emit CsFree + CsLoad to restore locals from the call stack.
    /// Called in the caller's cont block after the callee has already
    /// popped the RA slot via its Return sequence.
    pub(super) fn emit_cs_restore(&mut self, spill_words: &[Word]) {
        let locals_slots = self.cs_locals_slots();
        let spill_slots = (spill_words.len() as u16) * 2;
        let frame_slots = locals_slots + spill_slots;
        self.emit(Inst8::no_dst(Inst8Kind::CsFree(frame_slots)));

        for i in 0..self.local_vregs.len() {
            self.emit_cs_load_word((i as u16) * 4, self.local_vregs[i]);
        }

        let spill_base_bytes = locals_slots * 2;
        for (i, word) in spill_words.iter().copied().enumerate() {
            self.emit_cs_load_word(spill_base_bytes + (i as u16) * 4, word);
        }
    }
}
