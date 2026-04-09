use crate::ast::{BinOp, RelOp, UnOp};

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct IrNode(pub usize);

impl IrNode {
    pub const IMM_TAG: usize = 1usize << (usize::BITS as usize - 1);

    pub const fn index(self) -> usize {
        self.0
    }

    // TODO(i64): immediate encoding/decoding is hardcoded to 32-bit integers.
    pub const fn imm_i32(value: i32) -> Self {
        Self(Self::IMM_TAG | (value as u32 as usize))
    }

    // TODO(i64): immediate encoding/decoding is hardcoded to 32-bit integers.
    pub const fn imm_i32_value(self) -> Option<i32> {
        if self.is_imm() {
            Some((self.0 as u32) as i32)
        } else {
            None
        }
    }

    pub const fn is_imm(self) -> bool {
        (self.0 & Self::IMM_TAG) != 0
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
    // TODO(i64): IR instruction set is currently i32-only for value-producing ops.
    I32Const(i32),

    LocalGet(u32),
    LocalSet(u32, IrNode),
    LocalTee(u32, IrNode),

    GlobalGet(u32),
    GlobalSet(u32, IrNode),

    MemorySize,
    TableSize(u32),

    Unary(UnOp, IrNode),
    Binary(BinOp, IrNode, IrNode),
    Compare(RelOp, IrNode, IrNode),
    Select {
        cond: IrNode,
        if_true: IrNode,
        if_false: IrNode,
    },

    Load {
        // TODO(i64): memory op metadata assumes i32 load widths/sign-extension behavior.
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
        // TODO(i64): memory op metadata assumes i32 store widths.
        size: u8,
        offset: u32,
        addr: IrNode,
        val: IrNode,
    },
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
            | Terminator::Unreachable => vec![],
        }
    }

    pub fn ref_base(blocks: &[BasicBlock], block_idx: usize) -> IrNode {
        IrNode(blocks[..block_idx].iter().map(|b| b.insts.len()).sum())
    }
}
