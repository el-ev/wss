use crate::ast::{BinOp, RelOp, UnOp};
use wasmparser::ValType;

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct IrNode(pub usize);

impl IrNode {
    pub const fn index(self) -> usize {
        self.0
    }

    pub const fn saturating_sub(self, rhs: usize) -> Self {
        Self(self.0.saturating_sub(rhs))
    }
}

impl std::fmt::Display for IrNode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<usize> for IrNode {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<IrNode> for usize {
    fn from(value: IrNode) -> Self {
        value.0
    }
}

impl std::ops::Add<usize> for IrNode {
    type Output = IrNode;

    fn add(self, rhs: usize) -> Self::Output {
        IrNode(self.0 + rhs)
    }
}

impl std::ops::AddAssign<usize> for IrNode {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs;
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct BlockId(pub usize);

impl BlockId {
    pub const fn index(self) -> usize {
        self.0
    }
}

impl From<usize> for BlockId {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<BlockId> for usize {
    fn from(value: BlockId) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub enum Inst {
    I32Const(i32),
    I64Const(i64),

    LocalGet(u32),
    LocalSet(u32, IrNode),
    LocalTee(u32, IrNode),

    GlobalGet(u32),
    GlobalSet(u32, IrNode),

    MemorySize,
    TableSize(u32),

    Unary {
        op: UnOp,
        ty: ValType,
        val: IrNode,
    },
    Binary {
        op: BinOp,
        ty: ValType,
        lhs: IrNode,
        rhs: IrNode,
    },
    Compare {
        op: RelOp,
        ty: ValType,
        lhs: IrNode,
        rhs: IrNode,
    },
    Select {
        ty: ValType,
        cond: IrNode,
        if_true: IrNode,
        if_false: IrNode,
    },

    Load {
        ty: ValType,
        size: u8,
        signed: bool,
        offset: u32,
        addr: IrNode,
    },

    Call {
        func: u32,
        args: Vec<IrNode>,
    },
    CallIndirect {
        type_index: u32,
        table_index: u32,
        index: IrNode,
        args: Vec<IrNode>,
    },
    Putchar(IrNode),
    Getchar,
    Drop,
    Store {
        ty: ValType,
        size: u8,
        offset: u32,
        addr: IrNode,
        val: IrNode,
    },

    // Exception state ops. Pseudo-instructions that lower8 (PR 5) will
    // materialize into real state reads/writes (via hidden globals or
    // callstack slots). `ExcSet`/`ExcClear` mutate state and do not produce
    // a value. `ExcFlagGet`/`ExcTagGet` read the current state.
    ExcSet {
        tag_index: u32,
    },
    ExcClear,
    ExcFlagGet,
    ExcTagGet,
    /// Store the current exception payload (single i32). Mutating,
    /// no value produced.
    ExcPayloadSet(IrNode),
    /// Read the current exception payload (single i32).
    ExcPayloadGet,
}

#[derive(Debug, Clone)]
pub enum Terminator {
    Goto(BlockId),

    Branch {
        cond: IrNode,
        if_true: BlockId,
        if_false: BlockId,
    },

    Switch {
        index: IrNode,
        targets: Vec<BlockId>,
        default: BlockId,
    },

    TailCall {
        func: u32,
        args: Vec<IrNode>,
    },
    TailCallIndirect {
        type_index: u32,
        table_index: u32,
        index: IrNode,
        args: Vec<IrNode>,
    },

    Return(Option<IrNode>),

    Unreachable,

    /// Uncaught-exception exit for a function: when no handler matched,
    /// control flows here. The entry function (`_start`) traps with
    /// `TrapCode::UncaughtException`; non-entry functions return to the
    /// caller with the current exception state still set so the caller's
    /// post-call check re-propagates.
    UncaughtExit,
}

#[derive(Debug, Clone)]
pub struct BasicBlock {
    pub id: BlockId,
    pub insts: Vec<Inst>,
    pub terminator: Terminator,
}

impl BasicBlock {
    pub fn successors(&self) -> Vec<BlockId> {
        match &self.terminator {
            Terminator::Goto(t) => vec![*t],
            Terminator::Branch {
                if_true, if_false, ..
            } => vec![*if_true, *if_false],
            Terminator::Switch {
                targets, default, ..
            } => targets
                .iter()
                .copied()
                .chain(std::iter::once(*default))
                .collect(),
            Terminator::TailCall { .. }
            | Terminator::TailCallIndirect { .. }
            | Terminator::Return(_)
            | Terminator::Unreachable
            | Terminator::UncaughtExit => vec![],
        }
    }

    pub fn ref_base(blocks: &[BasicBlock], block_idx: usize) -> IrNode {
        IrNode(blocks[..block_idx].iter().map(|b| b.insts.len()).sum())
    }
}
