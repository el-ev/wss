use anyhow::{Context, bail};
use wasmparser::{BlockType, FunctionBody, Operator, Parser, Payload::*, ValType};

use crate::ast::{AstRef, BinOp, Catch, Node, RelOp, TypedAstRef, UnOp};
use crate::module::{AstFuncBody, AstModule, ModuleInfo};

mod frame;

use frame::{
    BlockFrame, BlockKind, current_frame_mut, materialize_ref_stack, pop_call_args,
    target_block_index,
};

pub fn parse_module(info: ModuleInfo, wasm_bytes: &[u8]) -> anyhow::Result<AstModule> {
    let mut module = AstModule::new(info, vec![]);
    let body_len = module.info().functions().len();
    *module.bodies_mut() = vec![None; body_len];
    let parser = Parser::new(0);
    let mut func_index = 0usize;

    for payload in parser.parse_all(wasm_bytes) {
        let payload = payload.context("WASM parse")?;
        if let CodeSectionEntry(body) = payload {
            let idx = module.num_imported_funcs() + func_index;
            let parsed = parse_function(&module, idx, body)?;
            module.set_body(idx as u32, Some(parsed))?;
            func_index += 1;
        }
    }

    Ok(module)
}

fn extend_locals_from_body(locals: &mut Vec<ValType>, body: &FunctionBody) -> anyhow::Result<()> {
    body.get_locals_reader()?.into_iter().try_for_each(|local| {
        let (count, val_type) = local?;
        locals.extend(std::iter::repeat_n(val_type, count as usize));
        Ok(())
    })
}

fn call_shape_from_func<'a>(
    module: &'a AstModule,
    function_index: u32,
    op_name: &str,
) -> anyhow::Result<(&'a [ValType], Option<ValType>)> {
    let sig = module.func_type_at(function_index).with_context(|| {
        format!(
            "{}: function index {} out of bounds",
            op_name, function_index
        )
    })?;
    Ok((sig.params(), sig.results().first().copied()))
}

fn call_shape_from_type<'a>(
    module: &'a AstModule,
    type_index: u32,
    op_name: &str,
) -> anyhow::Result<(&'a [ValType], Option<ValType>)> {
    let sig = module
        .type_at(type_index)
        .with_context(|| format!("{}: type index {} out of bounds", op_name, type_index))?;
    Ok((sig.params(), sig.results().first().copied()))
}

fn tag_has_i32_payload(module: &AstModule, tag_index: u32) -> anyhow::Result<bool> {
    let tag = module
        .info()
        .tag_at(tag_index)
        .with_context(|| format!("throw/catch: tag index {} out of bounds", tag_index))?;
    let ty = module.type_at(tag.type_index()).with_context(|| {
        format!(
            "tag {} references missing type {}",
            tag_index,
            tag.type_index()
        )
    })?;
    if !ty.results().is_empty() {
        bail!(
            "tag {} must not produce results (exception tag results must be empty)",
            tag_index
        );
    }
    let params = ty.params();
    match params {
        [] => Ok(false),
        [wasmparser::ValType::I32] => Ok(true),
        [other] => bail!(
            "tag {} has non-i32 payload {:?}; only zero- or single-i32-payload exception tags are supported",
            tag_index,
            other
        ),
        _ => bail!(
            "tag {} has {} payload value(s); only zero- or single-i32-payload exception tags are supported",
            tag_index,
            params.len()
        ),
    }
}

fn emit_call(
    frame: &mut BlockFrame,
    ref_stack: &mut Vec<TypedAstRef>,
    call: Node,
    result_ty: Option<ValType>,
) {
    let value = frame.emit(call);
    if let Some(ty) = result_ty {
        ref_stack.push(TypedAstRef::new(value, ty));
    }
}

fn emit_return_call(frame: &mut BlockFrame, call: Node, has_result: bool) {
    let call_ref = frame.emit(call);
    frame.emit(Node::Return(has_result.then_some(call_ref)));
    frame.ensure_dummy(ValType::I32);
    frame.unreachable = true;
}

fn local_type(locals: &[ValType], local_index: u32, op_name: &str) -> anyhow::Result<ValType> {
    locals
        .get(local_index as usize)
        .copied()
        .with_context(|| format!("{}: local {} out of bounds", op_name, local_index))
}

fn global_type(module: &AstModule, global_index: u32, op_name: &str) -> anyhow::Result<ValType> {
    module
        .globals()
        .get(global_index as usize)
        .map(|global| global.content_type())
        .with_context(|| format!("{}: global {} out of bounds", op_name, global_index))
}

fn pop_typed_ref(
    frame: &mut BlockFrame,
    ref_stack: &mut Vec<TypedAstRef>,
    expected_ty: ValType,
    context: &str,
) -> anyhow::Result<AstRef> {
    Ok(frame
        .pop_ref(ref_stack, expected_ty)
        .with_context(|| context.to_string())?
        .value)
}

fn parse_function(
    module: &AstModule,
    func_index: usize,
    body: FunctionBody,
) -> anyhow::Result<AstFuncBody> {
    let mut locals = Vec::new();
    let signature = module
        .func_type_at(func_index as u32)
        .with_context(|| format!("function index {} out of bounds", func_index))?;
    locals.extend(signature.params().iter().copied());
    extend_locals_from_body(&mut locals, &body)?;
    let mut ref_stack: Vec<TypedAstRef> = Vec::new();
    let mut block_stack = vec![BlockFrame {
        kind: BlockKind::Function,
        return_types: signature.results().to_vec(),
        insts: Vec::new(),
        temp_locals: Vec::new(),
        unreachable: false,
        dummy_refs: Vec::new(),
    }];
    let mut ops_reader = body.get_operators_reader()?;

    while !ops_reader.eof() {
        let op = ops_reader.read()?;
        match op {
            Operator::I32Const { value } => {
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::I32Const(value));
                ref_stack.push(TypedAstRef::new(r, ValType::I32));
            }
            Operator::I64Const { value } => {
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::I64Const(value));
                ref_stack.push(TypedAstRef::new(r, ValType::I64));
            }
            Operator::I32Add
            | Operator::I32Sub
            | Operator::I32Mul
            | Operator::I32DivS
            | Operator::I32DivU
            | Operator::I32RemS
            | Operator::I32RemU
            | Operator::I32And
            | Operator::I32Or
            | Operator::I32Xor
            | Operator::I32Shl
            | Operator::I32ShrS
            | Operator::I32ShrU
            | Operator::I32Rotl
            | Operator::I32Rotr
            | Operator::I64Add
            | Operator::I64Sub
            | Operator::I64Mul
            | Operator::I64DivS
            | Operator::I64DivU
            | Operator::I64RemS
            | Operator::I64RemU
            | Operator::I64And
            | Operator::I64Or
            | Operator::I64Xor
            | Operator::I64Shl
            | Operator::I64ShrS
            | Operator::I64ShrU
            | Operator::I64Rotl
            | Operator::I64Rotr => {
                let ty = match op {
                    Operator::I32Add
                    | Operator::I32Sub
                    | Operator::I32Mul
                    | Operator::I32DivS
                    | Operator::I32DivU
                    | Operator::I32RemS
                    | Operator::I32RemU
                    | Operator::I32And
                    | Operator::I32Or
                    | Operator::I32Xor
                    | Operator::I32Shl
                    | Operator::I32ShrS
                    | Operator::I32ShrU
                    | Operator::I32Rotl
                    | Operator::I32Rotr => ValType::I32,
                    _ => ValType::I64,
                };
                let frame = current_frame_mut(&mut block_stack)?;
                let rhs = pop_typed_ref(frame, &mut ref_stack, ty, "empty stack?")?;
                let lhs = pop_typed_ref(frame, &mut ref_stack, ty, "empty stack?")?;
                let binop = match op {
                    Operator::I32Add => BinOp::Add,
                    Operator::I32Sub => BinOp::Sub,
                    Operator::I32Mul => BinOp::Mul,
                    Operator::I32DivS => BinOp::DivS,
                    Operator::I32DivU => BinOp::DivU,
                    Operator::I32RemS => BinOp::RemS,
                    Operator::I32RemU => BinOp::RemU,
                    Operator::I32And => BinOp::And,
                    Operator::I32Or => BinOp::Or,
                    Operator::I32Xor => BinOp::Xor,
                    Operator::I32Shl => BinOp::Shl,
                    Operator::I32ShrS => BinOp::ShrS,
                    Operator::I32ShrU => BinOp::ShrU,
                    Operator::I32Rotl => BinOp::Rotl,
                    Operator::I32Rotr => BinOp::Rotr,
                    Operator::I64Add => BinOp::Add,
                    Operator::I64Sub => BinOp::Sub,
                    Operator::I64Mul => BinOp::Mul,
                    Operator::I64DivS => BinOp::DivS,
                    Operator::I64DivU => BinOp::DivU,
                    Operator::I64RemS => BinOp::RemS,
                    Operator::I64RemU => BinOp::RemU,
                    Operator::I64And => BinOp::And,
                    Operator::I64Or => BinOp::Or,
                    Operator::I64Xor => BinOp::Xor,
                    Operator::I64Shl => BinOp::Shl,
                    Operator::I64ShrS => BinOp::ShrS,
                    Operator::I64ShrU => BinOp::ShrU,
                    Operator::I64Rotl => BinOp::Rotl,
                    Operator::I64Rotr => BinOp::Rotr,
                    _ => bail!("ice: unexpected integer binop variant {:?}", op),
                };
                let r = frame.emit(Node::Binary {
                    op: binop,
                    ty,
                    lhs,
                    rhs,
                });
                ref_stack.push(TypedAstRef::new(r, ty));
            }
            Operator::I32Clz
            | Operator::I32Ctz
            | Operator::I32Popcnt
            | Operator::I32Eqz
            | Operator::I32Extend8S
            | Operator::I32Extend16S
            | Operator::I64Clz
            | Operator::I64Ctz
            | Operator::I64Popcnt
            | Operator::I64Eqz
            | Operator::I64Extend8S
            | Operator::I64Extend16S
            | Operator::I64Extend32S
            | Operator::I32WrapI64
            | Operator::I64ExtendI32S
            | Operator::I64ExtendI32U => {
                let (ty, result_ty, unop) = match op {
                    Operator::I32Clz => (ValType::I32, ValType::I32, UnOp::Clz),
                    Operator::I32Ctz => (ValType::I32, ValType::I32, UnOp::Ctz),
                    Operator::I32Popcnt => (ValType::I32, ValType::I32, UnOp::Popcnt),
                    Operator::I32Eqz => (ValType::I32, ValType::I32, UnOp::Eqz),
                    Operator::I32Extend8S => (ValType::I32, ValType::I32, UnOp::Extend8S),
                    Operator::I32Extend16S => (ValType::I32, ValType::I32, UnOp::Extend16S),
                    Operator::I64Clz => (ValType::I64, ValType::I64, UnOp::Clz),
                    Operator::I64Ctz => (ValType::I64, ValType::I64, UnOp::Ctz),
                    Operator::I64Popcnt => (ValType::I64, ValType::I64, UnOp::Popcnt),
                    Operator::I64Eqz => (ValType::I64, ValType::I32, UnOp::Eqz),
                    Operator::I64Extend8S => (ValType::I64, ValType::I64, UnOp::Extend8S),
                    Operator::I64Extend16S => (ValType::I64, ValType::I64, UnOp::Extend16S),
                    Operator::I64Extend32S => (ValType::I64, ValType::I64, UnOp::Extend32S),
                    Operator::I32WrapI64 => (ValType::I64, ValType::I32, UnOp::WrapI64),
                    Operator::I64ExtendI32S => (ValType::I32, ValType::I64, UnOp::ExtendI32S),
                    Operator::I64ExtendI32U => (ValType::I32, ValType::I64, UnOp::ExtendI32U),
                    _ => bail!("ice: unexpected integer unary variant {:?}", op),
                };
                let frame = current_frame_mut(&mut block_stack)?;
                let val = pop_typed_ref(frame, &mut ref_stack, ty, "empty stack?")?;
                let r = frame.emit(Node::Unary { op: unop, ty, val });
                ref_stack.push(TypedAstRef::new(r, result_ty));
            }
            Operator::I32Eq
            | Operator::I32Ne
            | Operator::I32LtS
            | Operator::I32LtU
            | Operator::I32GtS
            | Operator::I32LeS
            | Operator::I32GeS
            | Operator::I32GtU
            | Operator::I32LeU
            | Operator::I32GeU
            | Operator::I64Eq
            | Operator::I64Ne
            | Operator::I64LtS
            | Operator::I64LtU
            | Operator::I64GtS
            | Operator::I64LeS
            | Operator::I64GeS
            | Operator::I64GtU
            | Operator::I64LeU
            | Operator::I64GeU => {
                let ty = match op {
                    Operator::I32Eq
                    | Operator::I32Ne
                    | Operator::I32LtS
                    | Operator::I32LtU
                    | Operator::I32GtS
                    | Operator::I32LeS
                    | Operator::I32GeS
                    | Operator::I32GtU
                    | Operator::I32LeU
                    | Operator::I32GeU => ValType::I32,
                    _ => ValType::I64,
                };
                let frame = current_frame_mut(&mut block_stack)?;
                let rhs = pop_typed_ref(frame, &mut ref_stack, ty, "empty stack?")?;
                let lhs = pop_typed_ref(frame, &mut ref_stack, ty, "empty stack?")?;
                let relop = match op {
                    Operator::I32Eq => RelOp::Eq,
                    Operator::I32Ne => RelOp::Ne,
                    Operator::I32LtS => RelOp::LtS,
                    Operator::I32LtU => RelOp::LtU,
                    Operator::I32GtS => RelOp::GtS,
                    Operator::I32LeS => RelOp::LeS,
                    Operator::I32GeS => RelOp::GeS,
                    Operator::I32GtU => RelOp::GtU,
                    Operator::I32LeU => RelOp::LeU,
                    Operator::I32GeU => RelOp::GeU,
                    Operator::I64Eq => RelOp::Eq,
                    Operator::I64Ne => RelOp::Ne,
                    Operator::I64LtS => RelOp::LtS,
                    Operator::I64LtU => RelOp::LtU,
                    Operator::I64GtS => RelOp::GtS,
                    Operator::I64LeS => RelOp::LeS,
                    Operator::I64GeS => RelOp::GeS,
                    Operator::I64GtU => RelOp::GtU,
                    Operator::I64LeU => RelOp::LeU,
                    Operator::I64GeU => RelOp::GeU,
                    _ => bail!("ice: unexpected integer relop variant {:?}", op),
                };
                let r = frame.emit(Node::Compare {
                    op: relop,
                    ty,
                    lhs,
                    rhs,
                });
                ref_stack.push(TypedAstRef::new(r, ValType::I32));
            }
            Operator::LocalGet { local_index } => {
                let ty = local_type(&locals, local_index, "LocalGet")?;
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::LocalGet(local_index));
                ref_stack.push(TypedAstRef::new(r, ty));
            }
            Operator::LocalSet { local_index } => {
                let ty = local_type(&locals, local_index, "LocalSet")?;
                let frame = current_frame_mut(&mut block_stack)?;
                let val = pop_typed_ref(frame, &mut ref_stack, ty, "empty stack?")?;
                frame.emit(Node::LocalSet(local_index, val));
            }
            Operator::LocalTee { local_index } => {
                let ty = local_type(&locals, local_index, "LocalTee")?;
                let frame = current_frame_mut(&mut block_stack)?;
                let val = pop_typed_ref(frame, &mut ref_stack, ty, "empty stack?")?;
                let r = frame.emit(Node::LocalTee(local_index, val));
                ref_stack.push(TypedAstRef::new(r, ty));
            }
            Operator::GlobalGet { global_index } => {
                let ty = global_type(module, global_index, "GlobalGet")?;
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::GlobalGet(global_index));
                ref_stack.push(TypedAstRef::new(r, ty));
            }
            Operator::GlobalSet { global_index } => {
                let ty = global_type(module, global_index, "GlobalSet")?;
                let frame = current_frame_mut(&mut block_stack)?;
                let val = pop_typed_ref(frame, &mut ref_stack, ty, "empty stack?")?;
                frame.emit(Node::GlobalSet(global_index, val));
            }
            Operator::MemorySize { .. } => {
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::MemorySize);
                ref_stack.push(TypedAstRef::new(r, ValType::I32));
            }
            Operator::I32Load { memarg }
            | Operator::I32Load8S { memarg }
            | Operator::I32Load8U { memarg }
            | Operator::I32Load16S { memarg }
            | Operator::I32Load16U { memarg }
            | Operator::I64Load { memarg }
            | Operator::I64Load8S { memarg }
            | Operator::I64Load8U { memarg }
            | Operator::I64Load16S { memarg }
            | Operator::I64Load16U { memarg }
            | Operator::I64Load32S { memarg }
            | Operator::I64Load32U { memarg } => {
                let ty = match op {
                    Operator::I32Load { .. }
                    | Operator::I32Load8S { .. }
                    | Operator::I32Load8U { .. }
                    | Operator::I32Load16S { .. }
                    | Operator::I32Load16U { .. } => ValType::I32,
                    _ => ValType::I64,
                };
                let size: usize = match op {
                    Operator::I32Load8S { .. } | Operator::I32Load8U { .. } => 8,
                    Operator::I32Load16S { .. } | Operator::I32Load16U { .. } => 16,
                    Operator::I32Load { .. } => 32,
                    Operator::I64Load8S { .. } | Operator::I64Load8U { .. } => 8,
                    Operator::I64Load16S { .. } | Operator::I64Load16U { .. } => 16,
                    Operator::I64Load32S { .. } | Operator::I64Load32U { .. } => 32,
                    Operator::I64Load { .. } => 64,
                    _ => bail!("ice: unexpected integer load variant {:?}", op),
                };
                let signed = matches!(
                    op,
                    Operator::I32Load8S { .. }
                        | Operator::I32Load16S { .. }
                        | Operator::I64Load8S { .. }
                        | Operator::I64Load16S { .. }
                        | Operator::I64Load32S { .. }
                );
                let frame = current_frame_mut(&mut block_stack)?;
                let address = pop_typed_ref(frame, &mut ref_stack, ValType::I32, "empty stack?")?;
                let r = frame.emit(Node::Load {
                    ty,
                    size,
                    signed,
                    offset: memarg.offset as usize,
                    address,
                });
                ref_stack.push(TypedAstRef::new(r, ty));
            }
            Operator::I32Store { memarg }
            | Operator::I32Store8 { memarg }
            | Operator::I32Store16 { memarg }
            | Operator::I64Store { memarg }
            | Operator::I64Store8 { memarg }
            | Operator::I64Store16 { memarg }
            | Operator::I64Store32 { memarg } => {
                let ty = match op {
                    Operator::I32Store { .. }
                    | Operator::I32Store8 { .. }
                    | Operator::I32Store16 { .. } => ValType::I32,
                    _ => ValType::I64,
                };
                let size: usize = match op {
                    Operator::I32Store8 { .. } => 8,
                    Operator::I32Store16 { .. } => 16,
                    Operator::I32Store { .. } => 32,
                    Operator::I64Store8 { .. } => 8,
                    Operator::I64Store16 { .. } => 16,
                    Operator::I64Store32 { .. } => 32,
                    Operator::I64Store { .. } => 64,
                    _ => bail!("ice: unexpected integer store variant {:?}", op),
                };
                let frame = current_frame_mut(&mut block_stack)?;
                let value = pop_typed_ref(frame, &mut ref_stack, ty, "empty stack?")?;
                let address = pop_typed_ref(frame, &mut ref_stack, ValType::I32, "empty stack?")?;
                frame.emit(Node::Store {
                    ty,
                    size,
                    offset: memarg.offset as usize,
                    value,
                    address,
                });
            }
            Operator::TableSize { table } => {
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::TableSize(table));
                ref_stack.push(TypedAstRef::new(r, ValType::I32));
            }
            Operator::Drop => {
                let frame = current_frame_mut(&mut block_stack)?;
                let val = ref_stack.pop().context("empty stack?")?;
                frame.emit(Node::Drop(val.value));
            }
            Operator::Select | Operator::TypedSelect { .. } => {
                let select_ty = match op {
                    Operator::TypedSelect { ty } => ty,
                    // Untyped select: stack is [then, else, cond]; peek past cond at the else value.
                    _ => ref_stack
                        .iter()
                        .rev()
                        .nth(1)
                        .map(|typed| typed.ty)
                        .unwrap_or(ValType::I32),
                };
                let frame = current_frame_mut(&mut block_stack)?;
                let cond = pop_typed_ref(frame, &mut ref_stack, ValType::I32, "empty stack?")?;
                let else_val = pop_typed_ref(frame, &mut ref_stack, select_ty, "empty stack?")?;
                let then_val = pop_typed_ref(frame, &mut ref_stack, select_ty, "empty stack?")?;
                let r = frame.emit(Node::Select {
                    ty: select_ty,
                    cond,
                    then_val,
                    else_val,
                });
                ref_stack.push(TypedAstRef::new(r, select_ty));
            }
            Operator::Block { blockty }
            | Operator::Loop { blockty }
            | Operator::If { blockty }
            | Operator::Try { blockty } => {
                let block_kind = match op {
                    Operator::Block { .. } => BlockKind::Block,
                    Operator::Loop { .. } => BlockKind::Loop,
                    Operator::If { .. } => {
                        let frame = current_frame_mut(&mut block_stack)?;
                        let cond_ref = pop_typed_ref(
                            frame,
                            &mut ref_stack,
                            ValType::I32,
                            "If: missing condition on stack",
                        )?;
                        BlockKind::If { cond_ref }
                    }
                    Operator::Try { .. } => BlockKind::Try,
                    _ => bail!("ice: unexpected block operator {:?}", op),
                };
                let return_type = match blockty {
                    BlockType::Empty => None,
                    BlockType::Type(v) => Some(v),
                    BlockType::FuncType(_) => bail!("Multi-Value block unsupported"),
                };
                let mut temp_locals = Vec::new();
                if let Some(return_type) = return_type {
                    locals.push(return_type);
                    temp_locals.push((locals.len() - 1) as u32);
                }
                let temps = materialize_ref_stack(&mut ref_stack, &mut block_stack, &mut locals)?;
                block_stack.push(BlockFrame {
                    kind: block_kind,
                    return_types: return_type.into_iter().collect(),
                    temp_locals,
                    insts: Vec::new(),
                    unreachable: false,
                    dummy_refs: Vec::new(),
                });
                for temp in temps.iter() {
                    let ty = local_type(&locals, *temp, "block temp local")?;
                    let r = current_frame_mut(&mut block_stack)?.emit(Node::LocalGet(*temp));
                    ref_stack.push(TypedAstRef::new(r, ty));
                }
            }
            Operator::Else => {
                let mut if_frame = block_stack.pop().context("Else without matching If")?;

                for temp_index in if_frame.temp_locals.clone().into_iter().rev() {
                    let ty = local_type(&locals, temp_index, "if temp local")?;
                    let last_ref = if_frame
                        .pop_ref(&mut ref_stack, ty)
                        .context("if then-branch end: stack underflow")?;
                    if_frame.emit(Node::LocalSet(temp_index, last_ref.value));
                }

                if let BlockKind::If { cond_ref } = if_frame.kind {
                    block_stack.push(BlockFrame {
                        kind: BlockKind::Else {
                            cond_ref,
                            then_insts: if_frame.insts,
                        },
                        return_types: if_frame.return_types,
                        temp_locals: if_frame.temp_locals,
                        insts: Vec::new(),
                        unreachable: false,
                        dummy_refs: Vec::new(),
                    });
                } else {
                    bail!("Else without If");
                }
            }
            Operator::End => {
                let mut frame = block_stack.pop().context("unexpected End")?;
                for temp_local in frame.temp_locals.clone().into_iter().rev() {
                    let ty = local_type(&locals, temp_local, "block temp local")?;
                    let last_ref = frame
                        .pop_ref(&mut ref_stack, ty)
                        .context("block end: stack underflow")?;
                    frame.emit(Node::LocalSet(temp_local, last_ref.value));
                }
                match frame.kind {
                    BlockKind::Function => {
                        let ret_ref = if !signature.results().is_empty() {
                            let result_ty = signature.results()[0];
                            Some(
                                frame
                                    .pop_ref(&mut ref_stack, result_ty)
                                    .with_context(
                                        || "Function End: popping return value (stack underflow)",
                                    )?
                                    .value,
                            )
                        } else {
                            None
                        };
                        frame.emit(Node::Return(ret_ref));
                        return Ok(AstFuncBody::new(locals, frame.insts));
                    }
                    _ => {
                        let temp_locals = frame.temp_locals.clone();
                        let result_inst = match frame.kind {
                            BlockKind::Function => {
                                bail!("ice: unexpected nested function frame")
                            }
                            BlockKind::Block => Node::Block(frame.insts),
                            BlockKind::Loop => Node::Loop(frame.insts),
                            BlockKind::If { cond_ref } => Node::If {
                                cond: cond_ref,
                                then_body: frame.insts,
                                else_body: Vec::new(),
                            },
                            BlockKind::Else {
                                cond_ref,
                                then_insts,
                            } => Node::If {
                                cond: cond_ref,
                                then_body: then_insts,
                                else_body: frame.insts,
                            },
                            BlockKind::Try => Node::Try {
                                body: frame.insts,
                                catches: Vec::new(),
                                catch_all: None,
                                delegate: None,
                            },
                            BlockKind::TryCatch {
                                try_insts,
                                mut prior_catches,
                                catch_all_seen,
                                current_tag,
                            } => {
                                let mut catch_all_insts = None;
                                match current_tag {
                                    Some(tag_index) => {
                                        prior_catches.push(Catch {
                                            tag_index,
                                            body: frame.insts,
                                        });
                                    }
                                    None => {
                                        catch_all_insts = Some(frame.insts);
                                    }
                                }
                                // `catch_all_seen` is true exactly when we're ending on a catch_all body.
                                debug_assert_eq!(catch_all_seen, catch_all_insts.is_some());
                                let _ = catch_all_seen;
                                Node::Try {
                                    body: try_insts,
                                    catches: prior_catches,
                                    catch_all: catch_all_insts,
                                    delegate: None,
                                }
                            }
                        };
                        let parent = current_frame_mut(&mut block_stack)?;
                        parent.emit(result_inst);
                        for &temp_index in temp_locals.iter() {
                            let ty = local_type(&locals, temp_index, "block temp local")?;
                            let r = parent.emit(Node::LocalGet(temp_index));
                            ref_stack.push(TypedAstRef::new(r, ty));
                        }
                    }
                }
            }
            Operator::Return => {
                let frame = current_frame_mut(&mut block_stack)?;
                let return_ref = if !signature.results().is_empty() {
                    let result_ty = signature.results()[0];
                    Some(
                        frame
                            .pop_ref(&mut ref_stack, result_ty)
                            .context("Return: popping return value (stack underflow)")?
                            .value,
                    )
                } else {
                    None
                };
                frame.ensure_dummy(ValType::I32);
                frame.unreachable = true;
                frame.emit(Node::Return(return_ref));
            }
            Operator::Br { relative_depth } => {
                let target_index = target_block_index(block_stack.len(), relative_depth, "Br")?;
                let temp_locals = block_stack[target_index].temp_locals.clone();
                let current = current_frame_mut(&mut block_stack)?;
                for temp_index in temp_locals.into_iter().rev() {
                    let ty = local_type(&locals, temp_index, "Br target temp local")?;
                    let last_ref = current.pop_ref_or_dummy(&mut ref_stack, ty)?;
                    current.emit(Node::LocalSet(temp_index, last_ref.value));
                }
                current.ensure_dummy(ValType::I32);
                current.unreachable = true;
                current.emit(Node::Br(relative_depth));
            }

            Operator::BrIf { relative_depth } => {
                let target_index = target_block_index(block_stack.len(), relative_depth, "BrIf")?;
                let target_temp_locals = block_stack[target_index].temp_locals.clone();
                let target_return_types = block_stack[target_index].return_types.clone();
                let current = current_frame_mut(&mut block_stack)?;
                let condition = current
                    .pop_ref(&mut ref_stack, ValType::I32)
                    .context("BrIf: missing condition")?
                    .value;

                if target_temp_locals.is_empty() {
                    current.emit(Node::BrIf(relative_depth, condition));
                } else {
                    let temp_locals = target_temp_locals;
                    let return_types = target_return_types;
                    let n = temp_locals.len();
                    let mut values: Vec<AstRef> = (0..n)
                        .zip(return_types.iter().copied())
                        .map(|(_, ty)| {
                            current
                                .pop_ref_or_dummy(&mut ref_stack, ty)
                                .map(|typed| typed.value)
                        })
                        .collect::<anyhow::Result<Vec<_>>>()?;
                    values.reverse();

                    let mut stash_indices = Vec::with_capacity(n);
                    for &ret_ty in &return_types {
                        stash_indices.push(locals.len() as u32);
                        locals.push(ret_ty);
                    }

                    let current_block = current_frame_mut(&mut block_stack)?;
                    for (&stash_idx, &value) in stash_indices.iter().zip(&values) {
                        current_block.emit(Node::LocalSet(stash_idx, value));
                    }

                    let mut then_body = Vec::new();
                    for (&stash_idx, &temp_index) in stash_indices.iter().zip(&temp_locals) {
                        let get_ref = AstRef::new(then_body.len());
                        then_body.push(Node::LocalGet(stash_idx));
                        then_body.push(Node::LocalSet(temp_index, get_ref));
                    }
                    then_body.push(Node::Br(relative_depth + 1));

                    current_block.emit(Node::If {
                        cond: condition,
                        then_body,
                        else_body: Vec::new(),
                    });

                    for &stash_idx in stash_indices.iter() {
                        let ty = local_type(&locals, stash_idx, "BrIf stash local")?;
                        let r =
                            current_frame_mut(&mut block_stack)?.emit(Node::LocalGet(stash_idx));
                        ref_stack.push(TypedAstRef::new(r, ty));
                    }
                }
            }
            Operator::BrTable { targets } => {
                let index_ref = {
                    let current = current_frame_mut(&mut block_stack)?;
                    current
                        .pop_ref(&mut ref_stack, ValType::I32)
                        .context("BrTable: missing index on stack")?
                        .value
                };
                let mut target_depths = Vec::new();
                for target in targets.targets() {
                    target_depths.push(target.context("BrTable: failed to read target")?);
                }
                let default_depth = targets.default();

                let default_target_index = block_stack
                    .len()
                    .checked_sub(1 + default_depth as usize)
                    .with_context(|| {
                        format!(
                            "BrTable: default_depth {} exceeds block depth",
                            default_depth
                        )
                    })?;
                let n = block_stack[default_target_index].temp_locals.len();
                let default_return_types = block_stack[default_target_index].return_types.clone();
                let target_temp_locals: Vec<Vec<u32>> = target_depths
                    .iter()
                    .chain(std::iter::once(&default_depth))
                    .map(|&depth| {
                        let idx = target_block_index(block_stack.len(), depth, "BrTable")?;
                        Ok(block_stack[idx].temp_locals.clone())
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;

                let current = current_frame_mut(&mut block_stack)?;
                let mut values: Vec<AstRef> = (0..n)
                    .zip(default_return_types.iter().copied())
                    .map(|(_, ty)| {
                        current
                            .pop_ref_or_dummy(&mut ref_stack, ty)
                            .map(|typed| typed.value)
                    })
                    .collect::<anyhow::Result<Vec<_>>>()?;
                values.reverse();

                for temp_locals in &target_temp_locals {
                    for (temp_index, &value) in temp_locals.iter().zip(values.iter()) {
                        current.emit(Node::LocalSet(*temp_index, value));
                    }
                }

                current.emit(Node::BrTable(target_depths, default_depth, index_ref));
                current.ensure_dummy(ValType::I32);
                current.unreachable = true;
            }
            Operator::Unreachable => {
                let frame = current_frame_mut(&mut block_stack)?;
                frame.ensure_dummy(ValType::I32);
                frame.unreachable = true;
                frame.emit(Node::Unreachable);
            }
            Operator::Nop => {}
            Operator::Call { function_index } => {
                let (param_types, result_ty) =
                    call_shape_from_func(module, function_index, "Call")?;
                let args = pop_call_args(&mut ref_stack, param_types, "Call")?;
                let frame = current_frame_mut(&mut block_stack)?;
                emit_call(
                    frame,
                    &mut ref_stack,
                    Node::Call(function_index, args),
                    result_ty,
                );
            }
            Operator::CallIndirect {
                type_index,
                table_index,
            } => {
                let (param_types, result_ty) =
                    call_shape_from_type(module, type_index, "CallIndirect")?;
                let table_ref = ref_stack
                    .pop()
                    .context("CallIndirect: missing table index on stack")?;
                if table_ref.ty != ValType::I32 {
                    bail!(
                        "CallIndirect: table index must be i32, got {:?}",
                        table_ref.ty
                    );
                }
                let args = pop_call_args(&mut ref_stack, param_types, "CallIndirect")?;
                let frame = current_frame_mut(&mut block_stack)?;
                emit_call(
                    frame,
                    &mut ref_stack,
                    Node::CallIndirect {
                        type_index,
                        table_index,
                        index: table_ref.value,
                        args,
                    },
                    result_ty,
                );
            }
            Operator::ReturnCall { function_index } => {
                let (param_types, result_ty) =
                    call_shape_from_func(module, function_index, "ReturnCall")?;
                let args = pop_call_args(&mut ref_stack, param_types, "ReturnCall")?;
                let current_block = current_frame_mut(&mut block_stack)?;
                emit_return_call(
                    current_block,
                    Node::Call(function_index, args),
                    result_ty.is_some(),
                );
            }
            Operator::ReturnCallIndirect {
                type_index,
                table_index,
            } => {
                let (param_types, result_ty) =
                    call_shape_from_type(module, type_index, "ReturnCallIndirect")?;
                let table_ref = ref_stack
                    .pop()
                    .context("ReturnCallIndirect: missing table index on stack")?;
                if table_ref.ty != ValType::I32 {
                    bail!(
                        "ReturnCallIndirect: table index must be i32, got {:?}",
                        table_ref.ty
                    );
                }
                let args = pop_call_args(&mut ref_stack, param_types, "ReturnCallIndirect")?;
                let current_block = current_frame_mut(&mut block_stack)?;
                emit_return_call(
                    current_block,
                    Node::CallIndirect {
                        type_index,
                        table_index,
                        index: table_ref.value,
                        args,
                    },
                    result_ty.is_some(),
                );
            }
            Operator::Catch { tag_index } => {
                let mut frame = block_stack.pop().context("Catch without matching Try")?;
                for temp_index in frame.temp_locals.clone().into_iter().rev() {
                    let ty = local_type(&locals, temp_index, "Catch temp local")?;
                    let last_ref = frame
                        .pop_ref(&mut ref_stack, ty)
                        .context("Catch: stack underflow at segment end")?;
                    frame.emit(Node::LocalSet(temp_index, last_ref.value));
                }
                ref_stack.clear();
                let return_types = frame.return_types;
                let temp_locals = frame.temp_locals;
                let segment_insts = frame.insts;
                let (try_insts, prior_catches) = match frame.kind {
                    BlockKind::Try => (segment_insts, Vec::new()),
                    BlockKind::TryCatch {
                        try_insts,
                        mut prior_catches,
                        catch_all_seen,
                        current_tag,
                    } => {
                        if catch_all_seen {
                            bail!("Catch after catch_all is not allowed");
                        }
                        let prev_tag = current_tag
                            .context("ice: TryCatch expected current_tag before another Catch")?;
                        prior_catches.push(Catch {
                            tag_index: prev_tag,
                            body: segment_insts,
                        });
                        (try_insts, prior_catches)
                    }
                    _ => bail!("Catch without matching Try"),
                };
                block_stack.push(BlockFrame {
                    kind: BlockKind::TryCatch {
                        try_insts,
                        prior_catches,
                        catch_all_seen: false,
                        current_tag: Some(tag_index),
                    },
                    return_types,
                    temp_locals,
                    insts: Vec::new(),
                    unreachable: false,
                    dummy_refs: Vec::new(),
                });
                if tag_has_i32_payload(module, tag_index)? {
                    let frame = current_frame_mut(&mut block_stack)?;
                    let r = frame.emit(Node::ExcPayloadGet);
                    ref_stack.push(TypedAstRef::new(r, ValType::I32));
                }
            }
            Operator::CatchAll => {
                let mut frame = block_stack.pop().context("CatchAll without matching Try")?;
                for temp_index in frame.temp_locals.clone().into_iter().rev() {
                    let ty = local_type(&locals, temp_index, "CatchAll temp local")?;
                    let last_ref = frame
                        .pop_ref(&mut ref_stack, ty)
                        .context("CatchAll: stack underflow at segment end")?;
                    frame.emit(Node::LocalSet(temp_index, last_ref.value));
                }
                ref_stack.clear();
                let return_types = frame.return_types;
                let temp_locals = frame.temp_locals;
                let segment_insts = frame.insts;
                let (try_insts, prior_catches) = match frame.kind {
                    BlockKind::Try => (segment_insts, Vec::new()),
                    BlockKind::TryCatch {
                        try_insts,
                        mut prior_catches,
                        catch_all_seen,
                        current_tag,
                    } => {
                        if catch_all_seen {
                            bail!("Multiple catch_all clauses in the same try");
                        }
                        let prev_tag = current_tag
                            .context("ice: TryCatch expected current_tag before CatchAll")?;
                        prior_catches.push(Catch {
                            tag_index: prev_tag,
                            body: segment_insts,
                        });
                        (try_insts, prior_catches)
                    }
                    _ => bail!("CatchAll without matching Try"),
                };
                block_stack.push(BlockFrame {
                    kind: BlockKind::TryCatch {
                        try_insts,
                        prior_catches,
                        catch_all_seen: true,
                        current_tag: None,
                    },
                    return_types,
                    temp_locals,
                    insts: Vec::new(),
                    unreachable: false,
                    dummy_refs: Vec::new(),
                });
            }
            Operator::Delegate { relative_depth } => {
                let mut frame = block_stack.pop().context("Delegate without matching Try")?;
                for temp_index in frame.temp_locals.clone().into_iter().rev() {
                    let ty = local_type(&locals, temp_index, "Delegate temp local")?;
                    let last_ref = frame
                        .pop_ref(&mut ref_stack, ty)
                        .context("Delegate: stack underflow at try body end")?;
                    frame.emit(Node::LocalSet(temp_index, last_ref.value));
                }
                match frame.kind {
                    BlockKind::Try => {
                        let temp_locals = frame.temp_locals.clone();
                        let result_inst = Node::Try {
                            body: frame.insts,
                            catches: Vec::new(),
                            catch_all: None,
                            delegate: Some(relative_depth),
                        };
                        let parent = current_frame_mut(&mut block_stack)?;
                        parent.emit(result_inst);
                        for &temp_index in temp_locals.iter() {
                            let ty = local_type(&locals, temp_index, "Delegate temp local")?;
                            let r = parent.emit(Node::LocalGet(temp_index));
                            ref_stack.push(TypedAstRef::new(r, ty));
                        }
                    }
                    _ => bail!("Delegate is only valid inside a try block (not after catch)"),
                }
            }
            Operator::Throw { tag_index } => {
                let has_payload = tag_has_i32_payload(module, tag_index)?;
                let arg = if has_payload {
                    let frame = current_frame_mut(&mut block_stack)?;
                    Some(frame.pop_ref(&mut ref_stack, ValType::I32)?.value)
                } else {
                    None
                };
                let frame = current_frame_mut(&mut block_stack)?;
                frame.ensure_dummy(ValType::I32);
                frame.unreachable = true;
                frame.emit(Node::Throw {
                    tag: tag_index,
                    arg,
                });
            }
            Operator::Rethrow { relative_depth } => {
                let frame = current_frame_mut(&mut block_stack)?;
                frame.ensure_dummy(ValType::I32);
                frame.unreachable = true;
                frame.emit(Node::Rethrow(relative_depth));
            }
            _ => bail!("Unsupported operator: {:?}", op),
        }
    }

    bail!("function body ended without End")
}

#[cfg(test)]
mod tests {
    use super::tag_has_i32_payload;
    use crate::module::{AstModule, FuncType, ModuleInfo, TagInfo};
    use wasmparser::ValType;

    fn module_with_tag(params: &[ValType], results: &[ValType]) -> AstModule {
        let mut info = ModuleInfo::default();
        info.types_mut().push(FuncType::new(
            params.to_vec().into_boxed_slice(),
            results.to_vec().into_boxed_slice(),
        ));
        info.tags_mut().push(TagInfo::new(0));
        AstModule::new(info, vec![])
    }

    #[test]
    fn tag_payload_helper_accepts_zero_or_single_i32_payload() {
        let no_payload = module_with_tag(&[], &[]);
        assert!(!tag_has_i32_payload(&no_payload, 0).expect("zero-payload tag should work"));

        let i32_payload = module_with_tag(&[ValType::I32], &[]);
        assert!(tag_has_i32_payload(&i32_payload, 0).expect("i32-payload tag should work"));
    }

    #[test]
    fn tag_payload_helper_rejects_unsupported_payload_shapes() {
        let non_i32 = module_with_tag(&[ValType::F32], &[]);
        let err = tag_has_i32_payload(&non_i32, 0).expect_err("non-i32 payload should fail");
        assert!(format!("{err:#}").contains("non-i32 payload"));

        let multi = module_with_tag(&[ValType::I32, ValType::I32], &[]);
        let err = tag_has_i32_payload(&multi, 0).expect_err("multi-payload tag should fail");
        assert!(format!("{err:#}").contains("payload value(s)"));
    }
}
