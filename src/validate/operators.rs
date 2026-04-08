use super::*;

pub(super) fn validate_operator(
    op: &Operator,
    module: &ModuleInfo,
    location: &str,
) -> anyhow::Result<()> {
    use Operator::*;
    match op {
        Block { blockty } | Loop { blockty } | If { blockty } => {
            validate_block_type(*blockty, module, location)?;
        }
        F32Load { .. }
        | F32Store { .. }
        | F64Load { .. }
        | F64Store { .. }
        | F32Const { .. }
        | F64Const { .. }
        | F32Eq
        | F32Ne
        | F32Lt
        | F32Gt
        | F32Le
        | F32Ge
        | F64Eq
        | F64Ne
        | F64Lt
        | F64Gt
        | F64Le
        | F64Ge
        | F32Abs
        | F32Neg
        | F32Ceil
        | F32Floor
        | F32Trunc
        | F32Nearest
        | F32Sqrt
        | F32Add
        | F32Sub
        | F32Mul
        | F32Div
        | F32Min
        | F32Max
        | F32Copysign
        | F64Abs
        | F64Neg
        | F64Ceil
        | F64Floor
        | F64Trunc
        | F64Nearest
        | F64Sqrt
        | F64Add
        | F64Sub
        | F64Mul
        | F64Div
        | F64Min
        | F64Max
        | F64Copysign
        | I32TruncF32S
        | I32TruncF32U
        | I32TruncF64S
        | I32TruncF64U
        | I64TruncF32S
        | I64TruncF32U
        | I64TruncF64S
        | I64TruncF64U
        | F32ConvertI32S
        | F32ConvertI32U
        | F32ConvertI64S
        | F32ConvertI64U
        | F32DemoteF64
        | F64ConvertI32S
        | F64ConvertI32U
        | F64ConvertI64S
        | F64ConvertI64U
        | F64PromoteF32
        | I32ReinterpretF32
        | I64ReinterpretF64
        | F32ReinterpretI32
        | F64ReinterpretI64
        | I32TruncSatF32S
        | I32TruncSatF32U
        | I32TruncSatF64S
        | I32TruncSatF64U
        | I64TruncSatF32S
        | I64TruncSatF32U
        | I64TruncSatF64S
        | I64TruncSatF64U => bail!("floating point not supported at {}", location),
        // TODO(i64): operator validator currently rejects all i64 operators.
        I64Load { .. }
        | I64Load8S { .. }
        | I64Load8U { .. }
        | I64Load16S { .. }
        | I64Load16U { .. }
        | I64Load32S { .. }
        | I64Load32U { .. }
        | I64Store { .. }
        | I64Store8 { .. }
        | I64Store16 { .. }
        | I64Store32 { .. }
        | I64Const { .. }
        | I64Eqz
        | I64Eq
        | I64Ne
        | I64LtS
        | I64LtU
        | I64GtS
        | I64GtU
        | I64LeS
        | I64LeU
        | I64GeS
        | I64GeU
        | I64Clz
        | I64Ctz
        | I64Popcnt
        | I64Add
        | I64Sub
        | I64Mul
        | I64DivS
        | I64DivU
        | I64RemS
        | I64RemU
        | I64And
        | I64Or
        | I64Xor
        | I64Shl
        | I64ShrS
        | I64ShrU
        | I64Rotl
        | I64Rotr
        | I32WrapI64
        | I64ExtendI32S
        | I64ExtendI32U
        | I64Extend8S
        | I64Extend16S
        | I64Extend32S => bail!("i64 not supported at {}", location),
        MemoryGrow { .. } => bail!("memory.grow not supported at {}", location),
        MemoryInit { .. }
        | DataDrop { .. }
        | MemoryCopy { .. }
        | MemoryFill { .. }
        | TableInit { .. }
        | ElemDrop { .. }
        | TableCopy { .. }
        | TableFill { .. }
        | TableGrow { .. }
        | TableGet { .. }
        | TableSet { .. } => bail!("unsupported bulk/table op: {:?} at {}", op, location),
        StructNew { .. }
        | StructNewDefault { .. }
        | StructGet { .. }
        | StructGetS { .. }
        | StructSet { .. }
        | StructGetU { .. }
        | ArrayNew { .. }
        | ArrayNewDefault { .. }
        | ArrayNewFixed { .. }
        | ArrayNewData { .. }
        | ArrayNewElem { .. }
        | ArrayGet { .. }
        | ArrayGetS { .. }
        | ArrayGetU { .. }
        | ArraySet { .. }
        | ArrayLen
        | ArrayFill { .. }
        | ArrayCopy { .. }
        | ArrayInitData { .. }
        | ArrayInitElem { .. }
        | RefNull { .. }
        | RefIsNull
        | RefFunc { .. }
        | RefEq
        | RefTestNonNull { .. }
        | RefTestNullable { .. }
        | RefCastNonNull { .. }
        | RefCastNullable { .. }
        | BrOnCast { .. }
        | BrOnCastFail { .. }
        | RefI31
        | I31GetS
        | I31GetU
        | AnyConvertExtern
        | ExternConvertAny
        | StructNewDesc { .. }
        | StructNewDefaultDesc { .. }
        | RefGetDesc { .. }
        | RefCastDescEqNonNull { .. }
        | RefCastDescEqNullable { .. }
        | BrOnCastDescEq { .. }
        | BrOnCastDescEqFail { .. }
        | MemoryDiscard { .. } => bail!("reference/GC types not supported at {}", location),
        TypedSelect { ty } => validate_valtype(*ty, location)?,
        TypedSelectMulti { tys } => {
            for t in tys {
                validate_valtype(*t, location)?;
            }
        }
        CallIndirect {
            type_index,
            table_index,
        }
        | ReturnCallIndirect {
            type_index,
            table_index,
        } => validate_indirect_call(*type_index, *table_index, module, location)?,
        MemorySize { mem } => {
            if *mem != 0 {
                bail!("multiple memories not supported at {}", location);
            }
        }
        TableSize { table } => {
            module.table_at(*table).context({
                format!(
                    "table.size table index {} out of bounds at {}",
                    table, location
                )
            })?;
        }
        I32AtomicLoad { .. }
        | I64AtomicLoad { .. }
        | I32AtomicLoad8U { .. }
        | I32AtomicLoad16U { .. }
        | I64AtomicLoad8U { .. }
        | I64AtomicLoad16U { .. }
        | I64AtomicLoad32U { .. }
        | I32AtomicStore { .. }
        | I64AtomicStore { .. }
        | I32AtomicStore8 { .. }
        | I32AtomicStore16 { .. }
        | I64AtomicStore8 { .. }
        | I64AtomicStore16 { .. }
        | I64AtomicStore32 { .. }
        | I32AtomicRmwAdd { .. }
        | I64AtomicRmwAdd { .. }
        | I32AtomicRmw8AddU { .. }
        | I32AtomicRmw16AddU { .. }
        | I64AtomicRmw8AddU { .. }
        | I64AtomicRmw16AddU { .. }
        | I64AtomicRmw32AddU { .. }
        | I32AtomicRmwSub { .. }
        | I64AtomicRmwSub { .. }
        | I32AtomicRmw8SubU { .. }
        | I32AtomicRmw16SubU { .. }
        | I64AtomicRmw8SubU { .. }
        | I64AtomicRmw16SubU { .. }
        | I64AtomicRmw32SubU { .. }
        | I32AtomicRmwAnd { .. }
        | I64AtomicRmwAnd { .. }
        | I32AtomicRmw8AndU { .. }
        | I32AtomicRmw16AndU { .. }
        | I64AtomicRmw8AndU { .. }
        | I64AtomicRmw16AndU { .. }
        | I64AtomicRmw32AndU { .. }
        | I32AtomicRmwOr { .. }
        | I64AtomicRmwOr { .. }
        | I32AtomicRmw8OrU { .. }
        | I32AtomicRmw16OrU { .. }
        | I64AtomicRmw8OrU { .. }
        | I64AtomicRmw16OrU { .. }
        | I64AtomicRmw32OrU { .. }
        | I32AtomicRmwXor { .. }
        | I64AtomicRmwXor { .. }
        | I32AtomicRmw8XorU { .. }
        | I32AtomicRmw16XorU { .. }
        | I64AtomicRmw8XorU { .. }
        | I64AtomicRmw16XorU { .. }
        | I64AtomicRmw32XorU { .. }
        | I32AtomicRmwXchg { .. }
        | I64AtomicRmwXchg { .. }
        | I32AtomicRmw8XchgU { .. }
        | I32AtomicRmw16XchgU { .. }
        | I64AtomicRmw8XchgU { .. }
        | I64AtomicRmw16XchgU { .. }
        | I64AtomicRmw32XchgU { .. }
        | I32AtomicRmwCmpxchg { .. }
        | I64AtomicRmwCmpxchg { .. }
        | I32AtomicRmw8CmpxchgU { .. }
        | I32AtomicRmw16CmpxchgU { .. }
        | I64AtomicRmw8CmpxchgU { .. }
        | I64AtomicRmw16CmpxchgU { .. }
        | I64AtomicRmw32CmpxchgU { .. }
        | MemoryAtomicNotify { .. }
        | MemoryAtomicWait32 { .. }
        | MemoryAtomicWait64 { .. }
        | AtomicFence => bail!("atomics not supported at {}", location),
        TryTable { .. }
        | Throw { .. }
        | ThrowRef
        | Try { .. }
        | Catch { .. }
        | Rethrow { .. }
        | Delegate { .. }
        | CatchAll => bail!("exceptions not supported at {}", location),
        _ => {}
    }
    Ok(())
}

fn validate_indirect_call(
    type_index: u32,
    table_index: u32,
    module: &ModuleInfo,
    location: &str,
) -> anyhow::Result<()> {
    let ty = module.type_at(type_index).context({
        format!(
            "call_indirect type index {} out of bounds at {}",
            type_index, location
        )
    })?;
    for (kind, i, ty) in ty
        .params()
        .iter()
        .enumerate()
        .map(|(i, ty)| ("param", i, *ty))
        .chain(
            ty.results()
                .iter()
                .enumerate()
                .map(|(i, ty)| ("result", i, *ty)),
        )
    {
        validate_valtype(ty, &format!("type[{}].{}[{}]", type_index, kind, i))?;
    }

    let table = module.table_at(table_index).context({
        format!(
            "call_indirect table index {} out of bounds at {}",
            table_index, location
        )
    })?;
    if table.element_type() != RefType::FUNCREF {
        bail!(
            "call_indirect requires funcref table, found {:?} at {}",
            table.element_type(),
            location
        );
    }
    Ok(())
}
