use super::*;
use crate::ir8::Addr;

pub(super) fn mem_access_width_bytes(size: u8, op_name: &str) -> anyhow::Result<u16> {
    match size {
        8 => Ok(1),
        16 => Ok(2),
        32 => Ok(4),
        64 => Ok(8),
        _ => anyhow::bail!(
            "{} memory width {} is unsupported (expected 8/16/32/64 bits)",
            op_name,
            size
        ),
    }
}

pub(super) fn lower_mem_base(offset: u32, access_bytes: u16, op_name: &str) -> anyhow::Result<u16> {
    let max_base = (u16::MAX as u32) - ((access_bytes as u32) - 1);
    anyhow::ensure!(
        offset <= max_base,
        "{} offset {:#x} exceeds 16-bit address space for {}-byte access (max {:#x})",
        op_name,
        offset,
        access_bytes,
        max_base
    );
    Ok(offset as u16)
}

pub(super) fn lower_load_fill_byte(b: &mut FuncBuilder, sign_src: Val8, signed: bool) -> Val8 {
    if signed {
        let sign_bit = Val8::imm(0x80);
        let is_neg = b.alloc_reg();
        b.emit(Inst8::with_dst(is_neg, Inst8Kind::GeU(sign_src, sign_bit)));
        let zero = Val8::imm(0);
        let fill = b.alloc_reg();
        b.emit(Inst8::with_dst(fill, Inst8Kind::Sub(zero, is_neg)));
        fill
    } else {
        Val8::imm(0)
    }
}

pub(super) fn lower_inst(
    b: &mut FuncBuilder,
    ctx: &Lower8Context<'_>,
    inst: &Inst,
    iref: IrNode,
    live_after: &[IrNode],
) -> anyhow::Result<()> {
    if lower_locals_globals_inst(b, ctx, inst, iref)? {
        return Ok(());
    }
    if lower_numeric_inst(b, ctx, inst, iref, live_after)? {
        return Ok(());
    }
    if lower_memory_inst(b, inst, iref)? {
        return Ok(());
    }
    if lower_call_io_inst(b, ctx, inst, iref, live_after)? {
        return Ok(());
    }
    match inst {
        Inst::Drop => Ok(()),
        Inst::ExcSet { tag_index } => {
            let tag_word = Word::from_u32_imm(*tag_index);
            for (lane, src) in tag_word.bytes().into_iter().enumerate() {
                b.emit(Inst8::no_dst(Inst8Kind::ExcTagSet {
                    lane: lane as u8,
                    val: src,
                }));
            }
            b.emit(Inst8::no_dst(Inst8Kind::ExcFlagSet { val: Val8::imm(1) }));
            Ok(())
        }
        Inst::ExcClear => {
            b.emit(Inst8::no_dst(Inst8Kind::ExcFlagSet { val: Val8::imm(0) }));
            Ok(())
        }
        Inst::ExcFlagGet => {
            let flag = b.alloc_reg();
            b.emit(Inst8::with_dst(flag, Inst8Kind::ExcFlagGet));
            let dst = b.alloc_word();
            b.set_word_from_byte(dst, flag);
            b.set_word(iref, dst);
            Ok(())
        }
        Inst::ExcTagGet => {
            let dst = b.alloc_word();
            for (lane, dst_lane) in dst.bytes().into_iter().enumerate() {
                b.emit(Inst8::with_dst(
                    dst_lane,
                    Inst8Kind::ExcTagGet { lane: lane as u8 },
                ));
            }
            b.set_word(iref, dst);
            Ok(())
        }
        Inst::ExcPayloadSet(val_ref) => {
            let word = b.get_word(*val_ref);
            for (lane, src) in word.bytes().into_iter().enumerate() {
                b.emit(Inst8::no_dst(Inst8Kind::ExcPayloadSet {
                    lane: lane as u8,
                    val: src,
                }));
            }
            Ok(())
        }
        Inst::ExcPayloadGet => {
            let dst = b.alloc_word();
            for (lane, dst_lane) in dst.bytes().into_iter().enumerate() {
                b.emit(Inst8::with_dst(
                    dst_lane,
                    Inst8Kind::ExcPayloadGet { lane: lane as u8 },
                ));
            }
            b.set_word(iref, dst);
            Ok(())
        }
        _ => anyhow::bail!("ice: unhandled lowering op {inst:?}"),
    }
}

fn lower_locals_globals_inst(
    b: &mut FuncBuilder,
    ctx: &Lower8Context<'_>,
    inst: &Inst,
    iref: IrNode,
) -> anyhow::Result<bool> {
    match inst {
        Inst::I32Const(v) => {
            b.set_word(iref, Word::from_u32_imm(*v as u32));
        }
        Inst::I64Const(v) => {
            b.set_value(iref, ValueWords::from_i64_imm(*v));
        }
        Inst::LocalGet(local_index) => {
            let value = b.local_get(*local_index);
            b.set_value(iref, value);
        }
        Inst::LocalSet(local_index, val_ref) => {
            let val = b.get_value(*val_ref);
            b.local_set(*local_index, val);
        }
        Inst::LocalTee(local_index, val_ref) => {
            let val = b.get_value(*val_ref);
            b.local_set(*local_index, val);
            let snap = b.alloc_value(val.val_type());
            b.copy_value(snap, val);
            b.set_value(iref, snap);
        }
        Inst::GlobalGet(global_index) => {
            let slots = ctx
                .global_words
                .get(*global_index as usize)
                .ok_or_else(|| anyhow::anyhow!("global {} does not exist", global_index))?;
            let lo = b.load_global_word(slots[0]);
            let value = if slots.len() == 2 {
                ValueWords::two(lo, b.load_global_word(slots[1]))
            } else {
                ValueWords::one(lo)
            };
            b.set_value(iref, value);
        }
        Inst::GlobalSet(global_index, val_ref) => {
            let slots = ctx
                .global_words
                .get(*global_index as usize)
                .ok_or_else(|| anyhow::anyhow!("global {} does not exist", global_index))?;
            let value = b.get_value(*val_ref);
            b.store_global_word(slots[0], value.lo);
            if let Some(hi_slot) = slots.get(1) {
                let hi = value.hi.context("missing high word for i64 global.set")?;
                b.store_global_word(*hi_slot, hi);
            }
        }
        Inst::MemorySize => {
            let pages = ctx.module.num_pages() as u32;
            b.set_word(iref, Word::from_u32_imm(pages));
        }
        Inst::TableSize(table_index) => {
            let table = ctx.module.table_at(*table_index).ok_or_else(|| {
                anyhow::anyhow!(
                    "table.size references table {} which does not exist",
                    table_index
                )
            })?;
            // TODO(i64): table.size result is forced to i32 range.
            let entries = u32::try_from(table.entries().len()).map_err(|_| {
                anyhow::anyhow!(
                    "table.size table {} has {} entries which do not fit i32",
                    table_index,
                    table.entries().len()
                )
            })?;
            b.set_word(iref, Word::from_u32_imm(entries));
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn lower_numeric_inst(
    b: &mut FuncBuilder,
    ctx: &Lower8Context<'_>,
    inst: &Inst,
    iref: IrNode,
    live_after: &[IrNode],
) -> anyhow::Result<bool> {
    match inst {
        Inst::Unary { op, ty, val } => {
            if *ty == wasmparser::ValType::I32 {
                let operand = b.get_word(*val);
                match *op {
                    UnOp::ExtendI32S => {
                        let out = lower_extend_i32_to_i64(b, operand, true);
                        b.set_value(iref, out);
                    }
                    UnOp::ExtendI32U => {
                        let out = lower_extend_i32_to_i64(b, operand, false);
                        b.set_value(iref, out);
                    }
                    _ => {
                        let dst = lower_unary(b, *op, operand)?;
                        b.set_word(iref, dst);
                    }
                }
            } else {
                let value = b.get_value(*val);
                let out = match *op {
                    UnOp::Eqz => ValueWords::one(lower_eqz64(b, value)),
                    UnOp::WrapI64 => ValueWords::one(value.lo),
                    UnOp::Extend8S => lower_sign_extend64_from_byte(b, value, false, 0),
                    UnOp::Extend16S => lower_sign_extend64_from_byte(b, value, false, 1),
                    UnOp::Extend32S => {
                        let lo = value.lo;
                        lower_extend_i32_to_i64(b, lo, true)
                    }
                    UnOp::Clz => {
                        anyhow::bail!("i64.clz lower8 support is not implemented yet");
                    }
                    UnOp::Ctz => {
                        anyhow::bail!("i64.ctz lower8 support is not implemented yet");
                    }
                    UnOp::Popcnt => {
                        anyhow::bail!("i64.popcnt lower8 support is not implemented yet");
                    }
                    UnOp::ExtendI32S | UnOp::ExtendI32U => {
                        unreachable!("extend_i32 ops carry ty=I32 and are handled above")
                    }
                };
                b.set_value(iref, out);
            }
        }
        Inst::Binary { op, ty, lhs, rhs } => {
            if *ty == wasmparser::ValType::I32 {
                let lhs_word = b.get_word(*lhs);
                let rhs_word = b.get_word(*rhs);
                let dst = match *op {
                    BinOp::DivU | BinOp::RemU | BinOp::DivS | BinOp::RemS => {
                        if ctx.div_builtins.is_none() {
                            let builtin = if *op == BinOp::DivU {
                                BuiltinId::DivU32
                            } else if *op == BinOp::RemU {
                                BuiltinId::RemU32
                            } else if *op == BinOp::DivS {
                                BuiltinId::DivS32
                            } else {
                                BuiltinId::RemS32
                            };
                            lower_builtin_call(b, builtin, vec![lhs_word, rhs_word])
                        } else if let Some(denom_const) = word_const_u32(rhs_word) {
                            let lowered = match *op {
                                BinOp::DivU | BinOp::RemU => {
                                    lower_divrem_const_u32(b, *op, lhs_word, denom_const)
                                }
                                BinOp::DivS | BinOp::RemS => {
                                    lower_divrem_const_s32(b, *op, lhs_word, denom_const)
                                }
                                _ => None,
                            };
                            if let Some(v) = lowered {
                                v
                            } else {
                                lower_divrem_call_via_function(
                                    b,
                                    *op,
                                    lhs_word,
                                    rhs_word,
                                    live_after,
                                    ctx.allocs,
                                    ctx.div_builtins,
                                )?
                            }
                        } else {
                            lower_divrem_call_via_function(
                                b,
                                *op,
                                lhs_word,
                                rhs_word,
                                live_after,
                                ctx.allocs,
                                ctx.div_builtins,
                            )?
                        }
                    }
                    _ => lower_binary(b, *op, lhs_word, rhs_word)?,
                };
                b.set_word(iref, dst);
            } else {
                let lhs_value = b.get_value(*lhs);
                let rhs_value = b.get_value(*rhs);
                let out = match *op {
                    BinOp::Add => lower_add64(b, lhs_value, rhs_value),
                    BinOp::Sub => lower_sub64(b, lhs_value, rhs_value),
                    BinOp::And => lower_value_bytewise_op(b, lhs_value, rhs_value, Inst8Kind::And8),
                    BinOp::Or => lower_value_bytewise_op(b, lhs_value, rhs_value, Inst8Kind::Or8),
                    BinOp::Xor => lower_value_bytewise_op(b, lhs_value, rhs_value, Inst8Kind::Xor8),
                    _ => anyhow::bail!("i64 {:?} lower8 support is not implemented yet", op),
                };
                b.set_value(iref, out);
            }
        }
        Inst::Compare { op, ty, lhs, rhs } => {
            if *ty == wasmparser::ValType::I32 {
                let lhs_word = b.get_word(*lhs);
                let rhs_word = b.get_word(*rhs);
                let dst = lower_compare(b, *op, lhs_word, rhs_word)?;
                b.set_word(iref, dst);
            } else {
                let lhs_value = b.get_value(*lhs);
                let rhs_value = b.get_value(*rhs);
                let bit = match *op {
                    RelOp::Eq => lower_eq64_bit(b, lhs_value, rhs_value),
                    RelOp::Ne => {
                        let eq = lower_eq64_bit(b, lhs_value, rhs_value);
                        bool_not(b, eq)
                    }
                    RelOp::LtU => lower_ltu64_bit(b, lhs_value, rhs_value),
                    RelOp::LeU => {
                        let gt = lower_ltu64_bit(b, rhs_value, lhs_value);
                        bool_not(b, gt)
                    }
                    RelOp::GtU => lower_ltu64_bit(b, rhs_value, lhs_value),
                    RelOp::GeU => {
                        let lt = lower_ltu64_bit(b, lhs_value, rhs_value);
                        bool_not(b, lt)
                    }
                    _ => anyhow::bail!("signed i64 compare lower8 support is not implemented yet"),
                };
                let word = bool_to_word(b, bit);
                b.set_word(iref, word);
            }
        }
        Inst::Select {
            ty,
            cond,
            if_true,
            if_false,
        } => {
            let cv = b.get_word(*cond);
            let cond_bit = bool32(b, cv);
            if *ty == wasmparser::ValType::I32 {
                let tv = b.get_word(*if_true);
                let fv = b.get_word(*if_false);
                let dst = select_word(b, cond_bit, tv, fv);
                b.set_word(iref, dst);
            } else {
                let tv = b.get_value(*if_true);
                let fv = b.get_value(*if_false);
                let value = select_value(b, cond_bit, tv, fv);
                b.set_value(iref, value);
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn lower_memory_inst(b: &mut FuncBuilder, inst: &Inst, iref: IrNode) -> anyhow::Result<bool> {
    match inst {
        Inst::Load {
            ty,
            size,
            signed,
            offset,
            addr,
        } => {
            let width = mem_access_width_bytes(*size, "load")?;
            let base = lower_mem_base(*offset, width, "load")?;
            let addr_word = b.get_word(*addr);
            let a = addr_word.lo16();
            let load_word = |b: &mut FuncBuilder, base: u16, a: Addr| -> Word {
                let dst = b.alloc_word();
                b.emit(Inst8::with_dst(
                    dst.b0,
                    Inst8Kind::LoadMem {
                        base,
                        addr: a,
                        lane: 0,
                    },
                ));
                b.emit(Inst8::with_dst(
                    dst.b1,
                    Inst8Kind::LoadMem {
                        base,
                        addr: a,
                        lane: 1,
                    },
                ));
                let base_hi = base + 2;
                b.emit(Inst8::with_dst(
                    dst.b2,
                    Inst8Kind::LoadMem {
                        base: base_hi,
                        addr: a,
                        lane: 0,
                    },
                ));
                b.emit(Inst8::with_dst(
                    dst.b3,
                    Inst8Kind::LoadMem {
                        base: base_hi,
                        addr: a,
                        lane: 1,
                    },
                ));
                dst
            };
            if *ty == wasmparser::ValType::I32 {
                let dst = b.alloc_word();
                b.emit(Inst8::with_dst(
                    dst.b0,
                    Inst8Kind::LoadMem {
                        base,
                        addr: a,
                        lane: 0,
                    },
                ));
                if width == 1 {
                    let fill = lower_load_fill_byte(b, dst.b0, *signed);
                    b.emit(Inst8::with_dst(dst.b1, Inst8Kind::Copy(fill)));
                    b.emit(Inst8::with_dst(dst.b2, Inst8Kind::Copy(fill)));
                    b.emit(Inst8::with_dst(dst.b3, Inst8Kind::Copy(fill)));
                } else if width == 2 {
                    b.emit(Inst8::with_dst(
                        dst.b1,
                        Inst8Kind::LoadMem {
                            base,
                            addr: a,
                            lane: 1,
                        },
                    ));
                    let fill = lower_load_fill_byte(b, dst.b1, *signed);
                    b.emit(Inst8::with_dst(dst.b2, Inst8Kind::Copy(fill)));
                    b.emit(Inst8::with_dst(dst.b3, Inst8Kind::Copy(fill)));
                } else {
                    let base_hi = base + 2;
                    b.emit(Inst8::with_dst(
                        dst.b1,
                        Inst8Kind::LoadMem {
                            base,
                            addr: a,
                            lane: 1,
                        },
                    ));
                    b.emit(Inst8::with_dst(
                        dst.b2,
                        Inst8Kind::LoadMem {
                            base: base_hi,
                            addr: a,
                            lane: 0,
                        },
                    ));
                    b.emit(Inst8::with_dst(
                        dst.b3,
                        Inst8Kind::LoadMem {
                            base: base_hi,
                            addr: a,
                            lane: 1,
                        },
                    ));
                }
                b.set_word(iref, dst);
            } else {
                let value = match width {
                    8 => {
                        let lo = load_word(b, base, a);
                        let hi = load_word(b, base + 4, a);
                        ValueWords::two(lo, hi)
                    }
                    4 => {
                        let lo = load_word(b, base, a);
                        let fill = lower_load_fill_byte(b, lo.b3, *signed);
                        ValueWords::two(lo, Word::new(fill, fill, fill, fill))
                    }
                    2 => {
                        let lo = b.alloc_word();
                        b.emit(Inst8::with_dst(
                            lo.b0,
                            Inst8Kind::LoadMem {
                                base,
                                addr: a,
                                lane: 0,
                            },
                        ));
                        b.emit(Inst8::with_dst(
                            lo.b1,
                            Inst8Kind::LoadMem {
                                base,
                                addr: a,
                                lane: 1,
                            },
                        ));
                        let fill = lower_load_fill_byte(b, lo.b1, *signed);
                        let lo_ext = Word::new(lo.b0, lo.b1, fill, fill);
                        ValueWords::two(lo_ext, Word::new(fill, fill, fill, fill))
                    }
                    1 => {
                        let lo = b.alloc_word();
                        b.emit(Inst8::with_dst(
                            lo.b0,
                            Inst8Kind::LoadMem {
                                base,
                                addr: a,
                                lane: 0,
                            },
                        ));
                        let fill = lower_load_fill_byte(b, lo.b0, *signed);
                        let lo_ext = Word::new(lo.b0, fill, fill, fill);
                        ValueWords::two(lo_ext, Word::new(fill, fill, fill, fill))
                    }
                    _ => unreachable!("unsupported i64 load width {}", width),
                };
                b.set_value(iref, value);
            }
        }
        Inst::Store {
            ty,
            size,
            offset,
            addr,
            val,
        } => {
            let width = mem_access_width_bytes(*size, "store")?;
            let base = lower_mem_base(*offset, width, "store")?;
            let addr_word = b.get_word(*addr);
            let val_value = b.get_value(*val);
            let val_word = val_value.lo;
            let a = addr_word.lo16();
            b.emit(Inst8::no_dst(Inst8Kind::StoreMem {
                base,
                addr: a,
                lane: 0,
                val: val_word.b0,
            }));
            if width >= 2 {
                b.emit(Inst8::no_dst(Inst8Kind::StoreMem {
                    base,
                    addr: a,
                    lane: 1,
                    val: val_word.b1,
                }));
            }
            if width >= 4 {
                let base_hi = base + 2;
                b.emit(Inst8::no_dst(Inst8Kind::StoreMem {
                    base: base_hi,
                    addr: a,
                    lane: 0,
                    val: val_word.b2,
                }));
                b.emit(Inst8::no_dst(Inst8Kind::StoreMem {
                    base: base_hi,
                    addr: a,
                    lane: 1,
                    val: val_word.b3,
                }));
            }
            if *ty == wasmparser::ValType::I64 && width >= 8 {
                let hi = val_value.hi.context("missing high word for i64 store")?;
                let base_hi = base + 4;
                b.emit(Inst8::no_dst(Inst8Kind::StoreMem {
                    base: base_hi,
                    addr: a,
                    lane: 0,
                    val: hi.b0,
                }));
                b.emit(Inst8::no_dst(Inst8Kind::StoreMem {
                    base: base_hi,
                    addr: a,
                    lane: 1,
                    val: hi.b1,
                }));
                let base_hi2 = base_hi + 2;
                b.emit(Inst8::no_dst(Inst8Kind::StoreMem {
                    base: base_hi2,
                    addr: a,
                    lane: 0,
                    val: hi.b2,
                }));
                b.emit(Inst8::no_dst(Inst8Kind::StoreMem {
                    base: base_hi2,
                    addr: a,
                    lane: 1,
                    val: hi.b3,
                }));
            }
        }
        _ => return Ok(false),
    }
    Ok(true)
}

fn lower_call_io_inst(
    b: &mut FuncBuilder,
    ctx: &Lower8Context<'_>,
    inst: &Inst,
    iref: IrNode,
    live_after: &[IrNode],
) -> anyhow::Result<bool> {
    match inst {
        Inst::Putchar(val_ref) => {
            let val = b.get_word(*val_ref);
            b.emit(Inst8::no_dst(Inst8Kind::Putchar(val.b0)));
            let dst = b.alloc_word();
            b.set_word_from_byte(dst, val.b0);
            b.set_word(iref, dst);
        }
        Inst::Getchar => {
            let ch = b.alloc_reg();
            b.emit(Inst8::with_dst(ch, Inst8Kind::Getchar));
            let dst = b.alloc_word();
            b.set_word_from_byte(dst, ch);
            b.set_word(iref, dst);
        }
        Inst::Call {
            func: callee_id,
            args,
        } => {
            let mut arg_words = Vec::new();
            for r in args {
                let value = b.get_value(*r);
                arg_words.push(value.lo);
                if let Some(hi) = value.hi {
                    arg_words.push(hi);
                }
            }

            let callee_alloc = &ctx.allocs[*callee_id as usize];
            let n_params = arg_words.len();
            let callee_arg_vregs =
                calls::flatten_local_prefix(&callee_alloc.local_vregs, n_params)?;

            let callee_entry = Pc::new(*callee_id as u16 * PC_STRIDE);
            let cont = b.alloc_block();
            let spill_words =
                analysis::collect_spill_words(live_after, &b.inst_map, &b.local_vregs);
            b.emit_cs_save(cont, &spill_words);

            b.finish(Terminator8::CallSetup {
                callee_entry: CallTarget::Pc(callee_entry),
                cont,
                args: arg_words,
                callee_arg_vregs,
            });
            b.switch_to(cont);
            b.emit_cs_restore(&spill_words);

            if let Some(result_ty) = ctx
                .module
                .func_type_at(*callee_id)
                .and_then(|sig| sig.results().first())
                .copied()
            {
                let dst = b.alloc_value(result_ty);
                b.copy_ret_to_value(dst);
                b.set_value(iref, dst);
            }
        }
        Inst::CallIndirect {
            type_index,
            table_index,
            index,
            args,
        } => {
            calls::lower_call_indirect_inst(
                b,
                ctx,
                calls::CallIndirectInst {
                    type_index: *type_index,
                    table_index: *table_index,
                    index: *index,
                    args,
                    live_after,
                    result_ref: iref,
                },
            )?;
        }
        _ => return Ok(false),
    }
    Ok(true)
}
