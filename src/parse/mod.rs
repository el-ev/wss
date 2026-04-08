use anyhow::{Context, bail};
use wasmparser::{BlockType, FunctionBody, Operator, Parser, Payload::*};

use crate::ast::{AstRef, BinOp, Node, RelOp, UnOp};
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
            *module
                .body_mut_at(idx as u32)
                .context(format!("code section function index {} out of bounds", idx))? =
                Some(parsed);
            func_index += 1;
        }
    }

    Ok(module)
}

fn extend_locals_from_body(
    locals: &mut Vec<wasmparser::ValType>,
    body: &FunctionBody,
) -> anyhow::Result<()> {
    body.get_locals_reader()?.into_iter().try_for_each(|local| {
        let (count, val_type) = local?;
        locals.extend(std::iter::repeat_n(val_type, count as usize));
        Ok(())
    })
}

fn call_shape_from_func(
    module: &AstModule,
    function_index: u32,
    op_name: &str,
) -> anyhow::Result<(usize, bool)> {
    let sig = module.function_type_at(function_index).context({
        format!(
            "{}: function index {} out of bounds",
            op_name, function_index
        )
    })?;
    Ok((sig.params().len(), !sig.results().is_empty()))
}

fn call_shape_from_type(
    module: &AstModule,
    type_index: u32,
    op_name: &str,
) -> anyhow::Result<(usize, bool)> {
    let sig = module.type_at(type_index).context(format!(
        "{}: type index {} out of bounds",
        op_name, type_index
    ))?;
    Ok((sig.params().len(), !sig.results().is_empty()))
}

fn emit_call(frame: &mut BlockFrame, ref_stack: &mut Vec<AstRef>, call: Node, has_result: bool) {
    let r = frame.emit(call);
    if has_result {
        ref_stack.push(r);
    }
}

fn emit_return_call(frame: &mut BlockFrame, call: Node, has_result: bool) {
    let call_ref = frame.emit(call);
    frame.emit(Node::Return(has_result.then_some(call_ref)));
    frame.ensure_dummy();
    frame.unreachable = true;
}

fn parse_function(
    module: &AstModule,
    func_index: usize,
    body: FunctionBody,
) -> anyhow::Result<AstFuncBody> {
    let mut locals = Vec::new();
    let signature = module
        .function_type_at(func_index as u32)
        .context(format!("function index {} out of bounds", func_index))?;
    locals.extend(signature.params().iter().copied());
    extend_locals_from_body(&mut locals, &body)?;
    let mut ref_stack: Vec<AstRef> = Vec::new();
    let mut block_stack = vec![BlockFrame {
        kind: BlockKind::Function,
        return_types: signature.results().to_vec(),
        insts: Vec::new(),
        temp_locals: Vec::new(),
        unreachable: false,
        dummy_ref: None,
    }];
    let mut ops_reader = body.get_operators_reader()?;

    while !ops_reader.eof() {
        let op = ops_reader.read()?;
        match op {
            // TODO(i64): parser operator dispatch currently only lowers i32 numeric ops.
            Operator::I32Const { value } => {
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::I32Const(value));
                ref_stack.push(r);
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
            | Operator::I32Rotr => {
                let frame = current_frame_mut(&mut block_stack)?;
                let rhs = frame
                    .pop_ref(&mut ref_stack)
                    .context("stack underflow while popping rhs for i32 binop")?;
                let lhs = frame
                    .pop_ref(&mut ref_stack)
                    .context("stack underflow while popping lhs for i32 binop")?;
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
                    _ => bail!("ice: unexpected i32 binop variant {:?}", op),
                };
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::Binary(binop, lhs, rhs));
                ref_stack.push(r);
            }
            Operator::I32Clz | Operator::I32Ctz | Operator::I32Popcnt | Operator::I32Eqz => {
                let frame = current_frame_mut(&mut block_stack)?;
                let val = frame
                    .pop_ref(&mut ref_stack)
                    .context("stack underflow while popping operand for i32 unop")?;
                let unop = match op {
                    Operator::I32Clz => UnOp::Clz,
                    Operator::I32Ctz => UnOp::Ctz,
                    Operator::I32Popcnt => UnOp::Popcnt,
                    Operator::I32Eqz => UnOp::Eqz,
                    _ => bail!("ice: unexpected i32 unop variant {:?}", op),
                };
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::Unary(unop, val));
                ref_stack.push(r);
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
            | Operator::I32GeU => {
                let frame = current_frame_mut(&mut block_stack)?;
                let rhs = frame
                    .pop_ref(&mut ref_stack)
                    .context("stack underflow while popping rhs for i32 relop")?;
                let lhs = frame
                    .pop_ref(&mut ref_stack)
                    .context("stack underflow while popping lhs for i32 relop")?;
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
                    _ => bail!("ice: unexpected i32 relop variant {:?}", op),
                };
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::Compare(relop, lhs, rhs));
                ref_stack.push(r);
            }
            Operator::I32Extend8S | Operator::I32Extend16S => {
                let frame = current_frame_mut(&mut block_stack)?;
                let val = frame
                    .pop_ref(&mut ref_stack)
                    .context("stack underflow while popping operand for i32 extend")?;
                let unop = match op {
                    Operator::I32Extend8S => UnOp::Extend8S,
                    Operator::I32Extend16S => UnOp::Extend16S,
                    _ => bail!("ice: unexpected i32 extend variant {:?}", op),
                };
                let r = frame.emit(Node::Unary(unop, val));
                ref_stack.push(r);
            }
            Operator::LocalGet { local_index } => {
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::LocalGet(local_index));
                ref_stack.push(r);
            }
            Operator::LocalSet { local_index } => {
                let frame = current_frame_mut(&mut block_stack)?;
                let val = frame.pop_ref(&mut ref_stack).context("empty stack?")?;
                frame.emit(Node::LocalSet(local_index, val));
            }
            Operator::LocalTee { local_index } => {
                let frame = current_frame_mut(&mut block_stack)?;
                let val = frame.pop_ref(&mut ref_stack).context("empty stack?")?;
                let r = frame.emit(Node::LocalTee(local_index, val));
                ref_stack.push(r);
            }
            Operator::GlobalGet { global_index } => {
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::GlobalGet(global_index));
                ref_stack.push(r);
            }
            Operator::GlobalSet { global_index } => {
                let frame = current_frame_mut(&mut block_stack)?;
                let val = frame.pop_ref(&mut ref_stack).context("empty stack?")?;
                frame.emit(Node::GlobalSet(global_index, val));
            }
            Operator::MemorySize { .. } => {
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::MemorySize);
                ref_stack.push(r);
            }
            Operator::I32Load { memarg }
            | Operator::I32Load8S { memarg }
            | Operator::I32Load8U { memarg }
            | Operator::I32Load16S { memarg }
            | Operator::I32Load16U { memarg } => {
                // TODO(i64): memory parsing only recognizes i32 load forms and widths.
                let size: usize = match op {
                    Operator::I32Load8S { .. } | Operator::I32Load8U { .. } => 8,
                    Operator::I32Load16S { .. } | Operator::I32Load16U { .. } => 16,
                    Operator::I32Load { .. } => 32,
                    _ => bail!("ice: unexpected i32 load variant {:?}", op),
                };
                let signed = matches!(op, Operator::I32Load8S { .. } | Operator::I32Load16S { .. });
                let frame = current_frame_mut(&mut block_stack)?;
                let address = frame.pop_ref(&mut ref_stack).context("empty stack?")?;
                let r = frame.emit(Node::Load {
                    size,
                    signed,
                    offset: memarg.offset as usize,
                    address,
                });
                ref_stack.push(r);
            }
            Operator::I32Store { memarg }
            | Operator::I32Store8 { memarg }
            | Operator::I32Store16 { memarg } => {
                // TODO(i64): memory parsing only recognizes i32 store forms and widths.
                let size: usize = match op {
                    Operator::I32Store8 { .. } => 8,
                    Operator::I32Store16 { .. } => 16,
                    Operator::I32Store { .. } => 32,
                    _ => bail!("ice: unexpected i32 store variant {:?}", op),
                };
                let frame = current_frame_mut(&mut block_stack)?;
                let value = frame.pop_ref(&mut ref_stack).context("empty stack?")?;
                let address = frame.pop_ref(&mut ref_stack).context("empty stack?")?;
                frame.emit(Node::Store {
                    size,
                    offset: memarg.offset as usize,
                    value,
                    address,
                });
            }
            Operator::TableSize { table } => {
                let frame = current_frame_mut(&mut block_stack)?;
                let r = frame.emit(Node::TableSize(table));
                ref_stack.push(r);
            }
            Operator::Drop => {
                let frame = current_frame_mut(&mut block_stack)?;
                let val = frame.pop_ref(&mut ref_stack).context("empty stack?")?;
                frame.emit(Node::Drop(val));
            }
            Operator::Select | Operator::TypedSelect { .. } => {
                let frame = current_frame_mut(&mut block_stack)?;
                let cond = frame.pop_ref(&mut ref_stack).context("empty stack?")?;
                let else_val = frame.pop_ref(&mut ref_stack).context("empty stack?")?;
                let then_val = frame.pop_ref(&mut ref_stack).context("empty stack?")?;
                let r = frame.emit(Node::Select {
                    cond,
                    then_val,
                    else_val,
                });
                ref_stack.push(r);
            }
            Operator::Block { blockty } | Operator::Loop { blockty } | Operator::If { blockty } => {
                let block_kind = match op {
                    Operator::Block { .. } => BlockKind::Block,
                    Operator::Loop { .. } => BlockKind::Loop,
                    Operator::If { .. } => {
                        let frame = current_frame_mut(&mut block_stack)?;
                        let cond_ref = frame
                            .pop_ref(&mut ref_stack)
                            .context("If: missing condition on stack")?;
                        BlockKind::If { cond_ref }
                    }
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
                    dummy_ref: None,
                });
                for temp in temps.iter() {
                    let r = current_frame_mut(&mut block_stack)?.emit(Node::LocalGet(*temp));
                    ref_stack.push(r);
                }
            }
            Operator::Else => {
                let mut if_frame = block_stack.pop().context("Else without matching If")?;

                for temp_index in if_frame.temp_locals.clone().into_iter().rev() {
                    let last_ref = if_frame
                        .pop_ref(&mut ref_stack)
                        .context("if then-branch end: stack underflow")?;
                    if_frame.emit(Node::LocalSet(temp_index, last_ref));
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
                        dummy_ref: None,
                    });
                } else {
                    bail!("Else without If");
                }
            }
            Operator::End => {
                let mut frame = block_stack.pop().context("unexpected End")?;
                for temp_local in frame.temp_locals.clone().into_iter().rev() {
                    let last_ref = frame
                        .pop_ref(&mut ref_stack)
                        .context("block end: stack underflow")?;
                    frame.emit(Node::LocalSet(temp_local, last_ref));
                }
                match frame.kind {
                    BlockKind::Function => {
                        let ret_ref = if !signature.results().is_empty() {
                            Some(frame.pop_ref(&mut ref_stack).with_context(
                                || "Function End: popping return value (stack underflow)",
                            )?)
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
                        };
                        let parent = current_frame_mut(&mut block_stack)?;
                        parent.emit(result_inst);
                        for &temp_index in temp_locals.iter() {
                            let r = parent.emit(Node::LocalGet(temp_index));
                            ref_stack.push(r);
                        }
                    }
                }
            }
            Operator::Return => {
                let frame = current_frame_mut(&mut block_stack)?;
                let return_ref = if !signature.results().is_empty() {
                    Some(
                        frame
                            .pop_ref(&mut ref_stack)
                            .context("Return: popping return value (stack underflow)")?,
                    )
                } else {
                    None
                };
                frame.ensure_dummy();
                frame.unreachable = true;
                frame.emit(Node::Return(return_ref));
            }
            Operator::Br { relative_depth } => {
                let target_index = target_block_index(block_stack.len(), relative_depth, "Br")?;
                let temp_locals = block_stack[target_index].temp_locals.clone();
                let current = current_frame_mut(&mut block_stack)?;
                for temp_index in temp_locals.into_iter().rev() {
                    let last_ref = current.pop_ref_or_dummy(&mut ref_stack)?;
                    current.emit(Node::LocalSet(temp_index, last_ref));
                }
                current.ensure_dummy();
                current.unreachable = true;
                current.emit(Node::Br(relative_depth));
            }

            Operator::BrIf { relative_depth } => {
                let target_index = target_block_index(block_stack.len(), relative_depth, "BrIf")?;
                let target_temp_locals = block_stack[target_index].temp_locals.clone();
                let target_return_types = block_stack[target_index].return_types.clone();
                let current = current_frame_mut(&mut block_stack)?;
                let condition = current
                    .pop_ref(&mut ref_stack)
                    .context("BrIf: missing condition")?;

                if target_temp_locals.is_empty() {
                    current.emit(Node::BrIf(relative_depth, condition));
                } else {
                    let temp_locals = target_temp_locals;
                    let return_types = target_return_types;
                    let n = temp_locals.len();
                    let mut values: Vec<AstRef> = (0..n)
                        .map(|_| current.pop_ref_or_dummy(&mut ref_stack))
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
                        let r =
                            current_frame_mut(&mut block_stack)?.emit(Node::LocalGet(stash_idx));
                        ref_stack.push(r);
                    }
                }
            }
            Operator::BrTable { targets } => {
                let index_ref = {
                    let current = current_frame_mut(&mut block_stack)?;
                    current
                        .pop_ref(&mut ref_stack)
                        .context("BrTable: missing index on stack")?
                };
                let mut target_depths = Vec::new();
                for target in targets.targets() {
                    target_depths.push(target.context("BrTable: failed to read target")?);
                }
                let default_depth = targets.default();

                let default_target_index = block_stack
                    .len()
                    .checked_sub(1 + default_depth as usize)
                    .context({
                        format!(
                            "BrTable: default_depth {} exceeds block depth",
                            default_depth
                        )
                    })?;
                let n = block_stack[default_target_index].temp_locals.len();
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
                    .map(|_| current.pop_ref_or_dummy(&mut ref_stack))
                    .collect::<anyhow::Result<Vec<_>>>()?;
                values.reverse();

                for temp_locals in &target_temp_locals {
                    for (temp_index, &value) in temp_locals.iter().zip(values.iter()) {
                        current.emit(Node::LocalSet(*temp_index, value));
                    }
                }

                current.emit(Node::BrTable(target_depths, default_depth, index_ref));
                current.ensure_dummy();
                current.unreachable = true;
            }
            Operator::Unreachable => {
                let frame = current_frame_mut(&mut block_stack)?;
                frame.ensure_dummy();
                frame.unreachable = true;
                frame.emit(Node::Unreachable);
            }
            Operator::Nop => {}
            Operator::Call { function_index } => {
                let (arg_count, has_result) = call_shape_from_func(module, function_index, "Call")?;
                let args = pop_call_args(&mut ref_stack, arg_count, "Call")?;
                let frame = current_frame_mut(&mut block_stack)?;
                emit_call(
                    frame,
                    &mut ref_stack,
                    Node::Call(function_index, args),
                    has_result,
                );
            }
            Operator::CallIndirect {
                type_index,
                table_index,
            } => {
                let (arg_count, has_result) =
                    call_shape_from_type(module, type_index, "CallIndirect")?;
                let table_ref = ref_stack
                    .pop()
                    .context("CallIndirect: missing table index on stack")?;
                let args = pop_call_args(&mut ref_stack, arg_count, "CallIndirect")?;
                let frame = current_frame_mut(&mut block_stack)?;
                emit_call(
                    frame,
                    &mut ref_stack,
                    Node::CallIndirect {
                        type_index,
                        table_index,
                        index: table_ref,
                        args,
                    },
                    has_result,
                );
            }
            Operator::ReturnCall { function_index } => {
                let (arg_count, has_result) =
                    call_shape_from_func(module, function_index, "ReturnCall")?;
                let args = pop_call_args(&mut ref_stack, arg_count, "ReturnCall")?;
                let current_block = current_frame_mut(&mut block_stack)?;
                emit_return_call(current_block, Node::Call(function_index, args), has_result);
            }
            Operator::ReturnCallIndirect {
                type_index,
                table_index,
            } => {
                let (arg_count, has_result) =
                    call_shape_from_type(module, type_index, "ReturnCallIndirect")?;
                let table_ref = ref_stack
                    .pop()
                    .context("ReturnCallIndirect: missing table index on stack")?;
                let args = pop_call_args(&mut ref_stack, arg_count, "ReturnCallIndirect")?;
                let current_block = current_frame_mut(&mut block_stack)?;
                emit_return_call(
                    current_block,
                    Node::CallIndirect {
                        type_index,
                        table_index,
                        index: table_ref,
                        args,
                    },
                    has_result,
                );
            }
            // TODO(i64): parser fallback still rejects unhandled i64 operators.
            _ => bail!("Unsupported operator: {:?}", op),
        }
    }

    bail!("function body ended without End")
}
