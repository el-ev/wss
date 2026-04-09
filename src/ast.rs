#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, Default)]
pub struct AstRef(usize);

impl AstRef {
    pub const fn new(value: usize) -> Self {
        Self(value)
    }

    pub const fn index(self) -> usize {
        self.0
    }
}

impl std::fmt::Display for AstRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl From<usize> for AstRef {
    fn from(value: usize) -> Self {
        Self(value)
    }
}

impl From<AstRef> for usize {
    fn from(value: AstRef) -> Self {
        value.0
    }
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    DivS,
    DivU,
    RemS,
    RemU,
    And,
    Or,
    Xor,
    Shl,
    ShrS,
    ShrU,
    Rotl,
    Rotr,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum RelOp {
    Eq,
    Ne,
    LtS,
    LtU,
    GtS,
    GtU,
    LeS,
    LeU,
    GeS,
    GeU,
}

#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq)]
pub enum UnOp {
    Clz,
    Ctz,
    Popcnt,
    Eqz,
    Extend8S,
    Extend16S,
}

#[derive(Debug, Clone)]
pub enum Node {
    // TODO(i64): AST value nodes are currently i32-only; add i64/typed const and op variants.
    I32Const(i32),

    LocalGet(u32),
    LocalTee(u32, AstRef),
    GlobalGet(u32),
    MemorySize,
    TableSize(u32),

    Unary(UnOp, AstRef),
    Binary(BinOp, AstRef, AstRef),
    Compare(RelOp, AstRef, AstRef),

    Select {
        cond: AstRef,
        then_val: AstRef,
        else_val: AstRef,
    },

    Load {
        // TODO(i64): load size handling is wired for i32 consumers (8/16/32 extensions only).
        size: usize,
        signed: bool,
        offset: usize,
        address: AstRef,
    },

    Call(u32, Vec<AstRef>),
    CallIndirect {
        type_index: u32,
        table_index: u32,
        index: AstRef,
        args: Vec<AstRef>,
    },

    Drop(AstRef),
    LocalSet(u32, AstRef),
    GlobalSet(u32, AstRef),

    Store {
        // TODO(i64): store size handling is wired for i32 producers (8/16/32 only).
        size: usize,
        offset: usize,
        value: AstRef,
        address: AstRef,
    },

    Block(Vec<Node>),
    Loop(Vec<Node>),

    If {
        cond: AstRef,
        then_body: Vec<Node>,
        else_body: Vec<Node>,
    },

    Br(u32),
    BrIf(u32, AstRef),
    BrTable(Vec<u32>, u32, AstRef),

    Return(Option<AstRef>),

    Unreachable,
}
