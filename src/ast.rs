use wasmparser::ValType;

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
pub struct TypedAstRef {
    pub value: AstRef,
    pub ty: ValType,
}

impl TypedAstRef {
    pub const fn new(value: AstRef, ty: ValType) -> Self {
        Self { value, ty }
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
    Extend32S,
    WrapI64,
    ExtendI32S,
    ExtendI32U,
}

#[derive(Debug, Clone)]
pub enum Node {
    I32Const(i32),
    I64Const(i64),

    LocalGet(u32),
    LocalTee(u32, AstRef),
    GlobalGet(u32),
    MemorySize,
    TableSize(u32),

    Unary {
        op: UnOp,
        ty: ValType,
        val: AstRef,
    },
    Binary {
        op: BinOp,
        ty: ValType,
        lhs: AstRef,
        rhs: AstRef,
    },
    Compare {
        op: RelOp,
        ty: ValType,
        lhs: AstRef,
        rhs: AstRef,
    },

    Select {
        ty: ValType,
        cond: AstRef,
        then_val: AstRef,
        else_val: AstRef,
    },

    Load {
        ty: ValType,
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
        ty: ValType,
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

    Try {
        body: Vec<Node>,
        catches: Vec<Catch>,
        catch_all: Option<Vec<Node>>,
        delegate: Option<u32>,
    },
    /// Throw a tag. `arg` is `Some` when the tag carries a single i32 payload.
    Throw {
        tag: u32,
        arg: Option<AstRef>,
    },
    Rethrow(u32),
    /// Read the current exception payload (i32). Pushed synthetically onto
    /// the operand stack at catch entry when the tag has an i32 payload.
    ExcPayloadGet,
}

#[derive(Debug, Clone)]
pub struct Catch {
    pub tag_index: u32,
    pub body: Vec<Node>,
}
