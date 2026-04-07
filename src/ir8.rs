#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Val8 {
    VReg(u16),
    Imm(u8),
}

impl Val8 {
    pub const fn vreg(value: u16) -> Self {
        Self::VReg(value)
    }
    pub const fn imm(value: u8) -> Self {
        Self::Imm(value)
    }
    pub const fn is_imm(self) -> bool {
        matches!(self, Self::Imm(_))
    }
    pub const fn imm_value(self) -> Option<u8> {
        match self {
            Self::Imm(v) => Some(v),
            _ => None,
        }
    }
    pub const fn reg_index(self) -> Option<u16> {
        match self {
            Self::VReg(i) => Some(i),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pc(u16);

impl Pc {
    pub const fn new(value: u16) -> Self {
        Self(value)
    }
    pub const fn index(self) -> u16 {
        self.0
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct Word {
    pub b0: Val8,
    pub b1: Val8,
    pub b2: Val8,
    pub b3: Val8,
}

impl Word {
    pub fn new(b0: Val8, b1: Val8, b2: Val8, b3: Val8) -> Self {
        Self { b0, b1, b2, b3 }
    }
    pub const fn from_u32_imm(value: u32) -> Self {
        Self {
            b0: Val8::imm((value & 0xff) as u8),
            b1: Val8::imm(((value >> 8) & 0xff) as u8),
            b2: Val8::imm(((value >> 16) & 0xff) as u8),
            b3: Val8::imm(((value >> 24) & 0xff) as u8),
        }
    }
    pub fn bytes(self) -> [Val8; 4] {
        [self.b0, self.b1, self.b2, self.b3]
    }
    pub fn byte(self, lane: u8) -> Val8 {
        match lane {
            0 => self.b0,
            1 => self.b1,
            2 => self.b2,
            3 => self.b3,
            _ => unreachable!("word lane must be in 0..=3"),
        }
    }
    pub fn uses_through_lane(self, lane: u8) -> Vec<Val8> {
        self.bytes()
            .into_iter()
            .take(usize::from(lane) + 1)
            .filter(|r| !r.is_imm())
            .collect()
    }
    pub fn lo16(self) -> Addr {
        Addr {
            lo: self.b0,
            hi: self.b1,
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct Addr {
    pub lo: Val8,
    pub hi: Val8,
}

impl Addr {
    pub fn new(lo: Val8, hi: Val8) -> Self {
        Self { lo, hi }
    }

    pub fn uses(self) -> Vec<Val8> {
        let mut out = Vec::new();
        if !self.lo.is_imm() {
            out.push(self.lo);
        }
        if !self.hi.is_imm() {
            out.push(self.hi);
        }
        out
    }
}

pub const RET_B0: Val8 = Val8::vreg(0);
pub const RET_B1: Val8 = Val8::vreg(1);
pub const RET_B2: Val8 = Val8::vreg(2);
pub const RET_B3: Val8 = Val8::vreg(3);
pub const RET: Word = Word {
    b0: RET_B0,
    b1: RET_B1,
    b2: RET_B2,
    b3: RET_B3,
};

pub const VREG_START: u16 = 4;
pub const PC_STRIDE: u16 = 1_000;
pub const BUILTIN_PC_BASE: u16 = 0xFF00;

pub const BUILTIN_DIV_U32: Pc = Pc::new(BUILTIN_PC_BASE);
pub const BUILTIN_REM_U32: Pc = Pc::new(BUILTIN_PC_BASE + 1);
pub const BUILTIN_DIV_S32: Pc = Pc::new(BUILTIN_PC_BASE + 2);
pub const BUILTIN_REM_S32: Pc = Pc::new(BUILTIN_PC_BASE + 3);
pub const BUILTIN_SHL_32: Pc = Pc::new(BUILTIN_PC_BASE + 4);
pub const BUILTIN_SHR_U32: Pc = Pc::new(BUILTIN_PC_BASE + 5);
pub const BUILTIN_SHR_S32: Pc = Pc::new(BUILTIN_PC_BASE + 6);
pub const BUILTIN_ROTL_32: Pc = Pc::new(BUILTIN_PC_BASE + 7);
pub const BUILTIN_ROTR_32: Pc = Pc::new(BUILTIN_PC_BASE + 8);
pub const BUILTIN_CLZ_32: Pc = Pc::new(BUILTIN_PC_BASE + 9);
pub const BUILTIN_CTZ_32: Pc = Pc::new(BUILTIN_PC_BASE + 10);
pub const BUILTIN_POPCNT_32: Pc = Pc::new(BUILTIN_PC_BASE + 11);

pub const BOOL_NARY_MAX_INPUTS: usize = 16;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub struct BoolNary8 {
    pub len: u8,
    pub regs: [Val8; BOOL_NARY_MAX_INPUTS],
}

impl BoolNary8 {
    pub fn from_regs(regs: &[Val8]) -> Option<Self> {
        if regs.is_empty() || regs.len() > BOOL_NARY_MAX_INPUTS {
            return None;
        }

        let mut out = Self {
            len: regs.len() as u8,
            regs: [Val8::imm(0); BOOL_NARY_MAX_INPUTS],
        };
        out.regs[..regs.len()].copy_from_slice(regs);
        Some(out)
    }

    pub fn as_slice(&self) -> &[Val8] {
        &self.regs[..usize::from(self.len)]
    }

    pub fn regs(self) -> Vec<Val8> {
        self.as_slice()
            .iter()
            .copied()
            .filter(|r| !r.is_imm())
            .collect()
    }

    pub fn map_regs(self, mut f: impl FnMut(Val8) -> Val8) -> Self {
        let mut out = self;
        for reg in &mut out.regs[..usize::from(out.len)] {
            *reg = f(*reg);
        }
        out
    }

    pub fn try_map_regs<E>(self, mut f: impl FnMut(Val8) -> Result<Val8, E>) -> Result<Self, E> {
        let mut out = self;
        for reg in &mut out.regs[..usize::from(out.len)] {
            *reg = f(*reg)?;
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct Inst8 {
    pub dst: Option<Val8>,
    pub kind: Inst8Kind,
}

impl Inst8 {
    pub fn with_dst(dst: Val8, kind: Inst8Kind) -> Self {
        Self {
            dst: Some(dst),
            kind,
        }
    }
    pub fn no_dst(kind: Inst8Kind) -> Self {
        Self { dst: None, kind }
    }

    pub fn uses(&self) -> Vec<Val8> {
        let maybe_push = |out: &mut Vec<Val8>, r: Val8| {
            if !r.is_imm() {
                out.push(r);
            }
        };

        match &self.kind {
            Inst8Kind::Getchar | Inst8Kind::GlobalGetByte { .. } => vec![],
            Inst8Kind::Copy(s) | Inst8Kind::BoolNot(s) | Inst8Kind::Putchar(s) => {
                let mut out = Vec::new();
                maybe_push(&mut out, *s);
                out
            }
            Inst8Kind::Add32Byte { lhs, rhs, lane } | Inst8Kind::Sub32Byte { lhs, rhs, lane } => {
                let mut out = lhs.uses_through_lane(*lane);
                out.extend(rhs.uses_through_lane(*lane));
                out
            }
            Inst8Kind::Sub32Borrow { lhs, rhs } => {
                let mut out = lhs.uses_through_lane(3);
                out.extend(rhs.uses_through_lane(3));
                out
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
                let mut out = Vec::new();
                maybe_push(&mut out, *l);
                maybe_push(&mut out, *r);
                out
            }
            Inst8Kind::BoolAnd(op) | Inst8Kind::BoolOr(op) => op.regs(),
            Inst8Kind::Sel(c, l, r) => {
                let mut out = Vec::new();
                maybe_push(&mut out, *l);
                maybe_push(&mut out, *r);
                maybe_push(&mut out, *c);
                out
            }
            Inst8Kind::GlobalSetByte { val, .. } => {
                let mut out = Vec::new();
                maybe_push(&mut out, *val);
                out
            }
            Inst8Kind::LoadMem { addr, .. } => addr.uses(),
            Inst8Kind::StoreMem { addr, val, .. } => {
                let mut out = addr.uses();
                maybe_push(&mut out, *val);
                out
            }
            Inst8Kind::CsStore { val, .. } => {
                let mut out = Vec::new();
                maybe_push(&mut out, *val);
                out
            }
            Inst8Kind::CsLoad { .. }
            | Inst8Kind::CsStorePc { .. }
            | Inst8Kind::CsLoadPc { .. }
            | Inst8Kind::CsAlloc(_)
            | Inst8Kind::CsFree(_) => vec![],
        }
    }
    pub fn defs(&self) -> Vec<Val8> {
        self.dst.into_iter().collect()
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum Inst8Kind {
    Copy(Val8),

    Add32Byte {
        lhs: Word,
        rhs: Word,
        lane: u8,
    },
    Sub32Byte {
        lhs: Word,
        rhs: Word,
        lane: u8,
    },
    Sub32Borrow {
        lhs: Word,
        rhs: Word,
    },
    Add(Val8, Val8),   // (l + r) & 0xff
    Carry(Val8, Val8), // (l + r) >> 8
    Sub(Val8, Val8),   // (l - r) & 0xff

    MulLo(Val8, Val8), // (l * r) & 0xff
    MulHi(Val8, Val8), // (l * r) >> 8

    And8(Val8, Val8),
    Or8(Val8, Val8),
    Xor8(Val8, Val8),

    Eq(Val8, Val8),
    Ne(Val8, Val8),
    LtU(Val8, Val8),
    GeU(Val8, Val8),

    BoolAnd(BoolNary8),
    BoolOr(BoolNary8),
    BoolNot(Val8),

    Sel(Val8, Val8, Val8), // cond ? if_true : if_false

    GlobalGetByte {
        global_idx: u32,
        lane: u8,
    },
    GlobalSetByte {
        global_idx: u32,
        lane: u8,
        val: Val8,
    },

    LoadMem {
        base: u16,
        addr: Addr,
        lane: u8,
    },
    StoreMem {
        base: u16,
        addr: Addr,
        lane: u8,
        val: Val8,
    },

    Getchar,
    Putchar(Val8),

    // ─── Call stack (flat CSS memory with --cs_sp pointer) ──────────────
    //
    // Each call stack slot is a 16-bit CSS property (`--cs{N}`).
    // `cs_sp` counts slots, not bytes.
    //
    // Frame layout pushed by the caller before a call (N = num_locals):
    //   cs[cs_sp + 0]          = saved_local_0.b0 | (saved_local_0.b1 << 8)
    //   cs[cs_sp + 1]          = saved_local_0.b2 | (saved_local_0.b3 << 8)
    //   ...
    //   cs[cs_sp + 2*N - 1]    = saved_local_{N-1}.b2 | (saved_local_{N-1}.b3 << 8)
    //   cs[cs_sp + 2*N]        = RA  (continuation Pc, fits in one 16-bit slot)
    //
    // RA is at the top so the callee can pop it with CsFree(1) +
    // CsLoadPc(0) regardless of the caller's local count.
    //
    // CsAlloc advances cs_sp by 2*N + 1 (full frame in slots).
    // On return: callee CsFree(1) pops RA, caller CsFree(2*N) pops locals.
    /// Store an 8-bit VReg value to a byte lane in the call stack frame.
    /// `offset` is byte-addressed from the current `cs_sp` base:
    /// cell = cs_sp + floor(offset / 2), lane = offset % 2.
    CsStore {
        offset: u16,
        val: Val8,
    },
    /// Load an 8-bit value from a byte lane in the current call stack frame.
    /// `offset` uses the same byte-addressed mapping as CsStore.
    CsLoad {
        offset: u16,
    },
    /// Store a 16-bit Pc to call stack slot cs[cs_sp + offset] (one slot).
    /// `offset` is slot-indexed.
    CsStorePc {
        offset: u16,
        val: Pc,
    },
    /// Load a 16-bit Pc from call stack slot cs[cs_sp + offset] (one slot).
    /// `offset` is slot-indexed.
    /// Used by Return to load the RA. Result is used by the terminator, not
    /// stored in a VReg — the emitter reads this to know where to jump.
    CsLoadPc {
        offset: u16,
    },
    /// Advance cs_sp by `size` slots (after storing a frame).
    CsAlloc(u16),
    /// Retreat cs_sp by `size` slots (before loading a frame).
    CsFree(u16),
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum Terminator8 {
    Goto(Pc),
    Branch {
        cond: Val8,
        if_true: Pc,
        if_false: Pc,
    },
    Switch {
        index: Val8,
        targets: Vec<Pc>,
        default: Pc,
    },

    /// Call another function.
    ///
    /// The caller is responsible for saving/restoring its own locals using
    /// explicit CsStore/CsLoad/CsAlloc/CsFree instructions around this
    /// terminator.
    ///
    /// Emitter expands CallSetup to:
    ///   1. Copy `args` → `callee_arg_vregs` (parallel, equal-length).
    ///   2. Jump to `callee_entry`.
    ///
    /// The RA (`cont`) is saved by the caller as part of its call
    /// stack frame (via CsStore before CsAlloc), and loaded by the callee's
    /// Return terminator (via CsLoad after CsFree).
    CallSetup {
        callee_entry: Pc,
        cont: Pc,
        args: Vec<Word>,
        callee_arg_vregs: Vec<Word>,
    },

    /// Non-main return: copy `val` to RET, load RA from the call stack
    /// (via CsFree + CsLoadPc emitted before this terminator), jump to it.
    Return {
        val: Option<Word>,
    },
    /// Main exit: halt the CSS clock.
    Exit {
        val: Option<Word>,
    },

    Trap(TrapCode),
}

impl Terminator8 {
    pub fn uses(&self) -> Vec<Val8> {
        let maybe_push = |out: &mut Vec<Val8>, r: Val8| {
            if !r.is_imm() {
                out.push(r);
            }
        };

        match self {
            Terminator8::Goto(_) => vec![],
            Terminator8::Branch { cond, .. } => {
                let mut out = Vec::new();
                maybe_push(&mut out, *cond);
                out
            }
            Terminator8::Switch { index, .. } => {
                let mut out = Vec::new();
                maybe_push(&mut out, *index);
                out
            }
            Terminator8::CallSetup { args, .. } => {
                let mut out = Vec::new();
                for w in args {
                    for r in w.bytes() {
                        maybe_push(&mut out, r);
                    }
                }
                out
            }
            Terminator8::Return { val } | Terminator8::Exit { val } => {
                if let Some(w) = val {
                    let mut out = Vec::new();
                    for r in w.bytes() {
                        maybe_push(&mut out, r);
                    }
                    out
                } else {
                    vec![]
                }
            }
            Terminator8::Trap(_) => vec![],
        }
    }

    pub fn defs(&self) -> Vec<Val8> {
        match self {
            Terminator8::CallSetup {
                callee_arg_vregs, ..
            } => RET
                .bytes()
                .into_iter()
                .chain(callee_arg_vregs.iter().flat_map(|w| w.bytes()))
                .collect(),
            _ => vec![],
        }
    }

    pub fn successors(&self) -> Vec<Pc> {
        match self {
            Terminator8::Goto(pc) => vec![*pc],
            Terminator8::Branch {
                if_true, if_false, ..
            } => vec![*if_true, *if_false],
            Terminator8::Switch {
                targets, default, ..
            } => {
                let mut v: Vec<Pc> = targets.to_vec();
                v.push(*default);
                v
            }
            Terminator8::CallSetup { cont, .. } => vec![*cont],
            Terminator8::Return { .. } | Terminator8::Exit { .. } | Terminator8::Trap(_) => vec![],
        }
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum TrapCode {
    CallstackOverflow = -5,
    Exited = -1,
    Unreachable = -2,
    InvalidMemoryAccess = -3,
    DivisionByZero = -4,
}

// ─── Basic blocks ────────────────────────────────────────────────────────────

/// A basic block. `id` is its global Pc (`func_id * PC_STRIDE + local_index`),
/// assigned by lower8 — no separate scheduling pass needed.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub struct BasicBlock8 {
    pub id: Pc,
    pub insts: Vec<Inst8>,
    pub terminator: Terminator8,
}

/// Post-regalloc flat list of blocks in Pc order, ready for emission.
pub struct Cycle {
    pub pc: Pc,
    pub ops: Vec<Inst8>,
    pub terminator: Terminator8,
}

pub struct FrameInfo {
    pub entry: Pc,       // set by lower8
    pub num_locals: u32, // includes parameters
}

pub struct MemoryLayout {
    pub memory_end: u32,
    pub init_bytes: Vec<u8>,
}

pub struct Ir8Program {
    pub entry_func: u32,
    pub num_vregs: u16,
    pub func_blocks: Vec<Vec<BasicBlock8>>,
    pub cycles: Vec<Cycle>,
    pub frame_infos: Vec<FrameInfo>,
    pub memory_layout: MemoryLayout,
    pub global_init: Vec<u32>,
}
