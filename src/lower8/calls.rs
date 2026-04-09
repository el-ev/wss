use super::*;

#[derive(Clone, Copy)]
enum IndirectTargetKind {
    Direct(u32),
    Putchar,
    Getchar,
}

pub(super) struct CallIndirectInst<'a> {
    pub(super) type_index: u32,
    pub(super) table_index: u32,
    pub(super) index: IrNode,
    pub(super) args: &'a [IrNode],
    pub(super) live_after: &'a [IrNode],
    pub(super) result_ref: IrNode,
}

struct IndirectDispatchRequest<'a> {
    type_index: u32,
    table_index: u32,
    arg_count: usize,
    index_word: Word,
    trap_pc: Pc,
    dispatch_pc: Pc,
    op_name: &'a str,
}

#[derive(Clone, Copy)]
enum IndirectLoweringMode {
    Call,
    TailCall,
}

struct IndirectEmitContext<'a> {
    allocs: &'a [FuncAlloc],
    op_name: &'a str,
    mode: IndirectLoweringMode,
    spill_words: &'a [Word],
    join_pc: Option<Pc>,
}

fn build_callee_setup(
    allocs: &[FuncAlloc],
    callee_id: u32,
    arg_count: usize,
    op_name: &str,
) -> anyhow::Result<(Pc, Vec<Word>)> {
    let callee_alloc = allocs.get(callee_id as usize).context(format!(
        "{} references missing function {}",
        op_name, callee_id
    ))?;
    let callee_arg_vregs = callee_alloc.local_vregs[..arg_count].to_vec();
    let callee_entry = Pc::new(callee_id as u16 * PC_STRIDE);
    Ok((callee_entry, callee_arg_vregs))
}

fn finish_indirect_case(
    b: &mut FuncBuilder,
    emit_ctx: &IndirectEmitContext<'_>,
) -> anyhow::Result<()> {
    match emit_ctx.mode {
        IndirectLoweringMode::Call => {
            let join_pc = emit_ctx.join_pc.context({
                format!(
                    "{} internal error: missing join block for indirect call",
                    emit_ctx.op_name
                )
            })?;
            b.finish(Terminator8::Goto(join_pc));
        }
        IndirectLoweringMode::TailCall => emit_non_main_return_sequence(b, None),
    }
    Ok(())
}

fn emit_indirect_target_case(
    b: &mut FuncBuilder,
    kind: IndirectTargetKind,
    arg_words: &[Word],
    emit_ctx: &IndirectEmitContext<'_>,
) -> anyhow::Result<()> {
    match kind {
        IndirectTargetKind::Direct(callee_id) => {
            let (callee_entry, callee_arg_vregs) = build_callee_setup(
                emit_ctx.allocs,
                callee_id,
                arg_words.len(),
                emit_ctx.op_name,
            )?;
            match emit_ctx.mode {
                IndirectLoweringMode::Call => {
                    let join_pc = emit_ctx.join_pc.context({
                        format!(
                            "{} internal error: missing join block for indirect call",
                            emit_ctx.op_name
                        )
                    })?;
                    let cont = b.alloc_block();
                    b.emit_cs_save(cont, emit_ctx.spill_words);
                    b.finish(Terminator8::CallSetup {
                        callee_entry: CallTarget::Pc(callee_entry),
                        cont,
                        args: arg_words.to_vec(),
                        callee_arg_vregs,
                    });
                    b.switch_to(cont);
                    b.emit_cs_restore(emit_ctx.spill_words);
                    b.finish(Terminator8::Goto(join_pc));
                }
                IndirectLoweringMode::TailCall => {
                    b.finish(Terminator8::CallSetup {
                        callee_entry: CallTarget::Pc(callee_entry),
                        cont: callee_entry,
                        args: arg_words.to_vec(),
                        callee_arg_vregs,
                    });
                }
            }
        }
        IndirectTargetKind::Putchar => {
            let arg = arg_words.first().context(format!(
                "{} putchar call missing argument",
                emit_ctx.op_name
            ))?;
            // TODO(i64): putchar/getchar bridge currently reads/writes only the low byte lane.
            b.emit(Inst8::no_dst(Inst8Kind::Putchar(arg.b0)));
            b.set_ret_from_byte(arg.b0);
            finish_indirect_case(b, emit_ctx)?;
        }
        IndirectTargetKind::Getchar => {
            let ch = b.alloc_reg();
            b.emit(Inst8::with_dst(ch, Inst8Kind::Getchar));
            b.set_ret_from_byte(ch);
            finish_indirect_case(b, emit_ctx)?;
        }
    }
    Ok(())
}

pub(super) fn emit_non_main_return_sequence(b: &mut FuncBuilder, val: Option<Word>) {
    // Pop RA from the call stack before returning.
    // The caller pushed RA as the last slot of its frame.
    // CsFree(1) backs up cs_sp by one slot, then CsLoadPc(0)
    // reads the RA for the jump.
    b.emit(Inst8::no_dst(Inst8Kind::CsFree(1)));
    b.emit(Inst8::no_dst(Inst8Kind::CsLoadPc { offset: 0 }));
    b.finish(Terminator8::Return { val });
}

pub(super) fn lower_tail_call(
    b: &mut FuncBuilder,
    func: u32,
    args: &[IrNode],
    allocs: &[FuncAlloc],
) -> anyhow::Result<Terminator8> {
    let arg_words: Vec<Word> = args.iter().map(|r| b.get_word(*r)).collect();
    let (callee_entry, callee_arg_vregs) =
        build_callee_setup(allocs, func, arg_words.len(), "tail_call")?;
    Ok(Terminator8::CallSetup {
        callee_entry: CallTarget::Pc(callee_entry),
        cont: callee_entry,
        args: arg_words,
        callee_arg_vregs,
    })
}

pub(super) fn lower_call_indirect_inst(
    b: &mut FuncBuilder,
    ctx: &Lower8Context<'_>,
    inst: CallIndirectInst<'_>,
) -> anyhow::Result<()> {
    let arg_words: Vec<Word> = inst.args.iter().map(|r| b.get_word(*r)).collect();
    let index_word = b.get_word(inst.index);
    let spill_words = analysis::collect_spill_words(inst.live_after, &b.inst_map, &b.local_vregs);

    let trap_pc = b.alloc_block();
    let dispatch_pc = b.alloc_block();
    let join_pc = b.alloc_block();
    let emit_ctx = IndirectEmitContext {
        allocs: ctx.allocs,
        op_name: "call_indirect",
        mode: IndirectLoweringMode::Call,
        spill_words: &spill_words,
        join_pc: Some(join_pc),
    };
    let case_blocks = build_indirect_dispatch(
        b,
        ctx.module,
        IndirectDispatchRequest {
            type_index: inst.type_index,
            table_index: inst.table_index,
            arg_count: arg_words.len(),
            index_word,
            trap_pc,
            dispatch_pc,
            op_name: "call_indirect",
        },
    )?;

    for (case_pc, kind) in case_blocks {
        b.switch_to(case_pc);
        emit_indirect_target_case(b, kind, &arg_words, &emit_ctx)?;
    }

    b.switch_to(join_pc);
    let dst = b.alloc_word();
    b.copy_ret_to_word(dst);
    b.set_word(inst.result_ref, dst);

    Ok(())
}

pub(super) fn lower_tail_call_indirect(
    b: &mut FuncBuilder,
    module: &IrModule,
    type_index: u32,
    table_index: u32,
    index: IrNode,
    args: &[IrNode],
    allocs: &[FuncAlloc],
) -> anyhow::Result<()> {
    let arg_words: Vec<Word> = args.iter().map(|r| b.get_word(*r)).collect();
    let index_word = b.get_word(index);
    let emit_ctx = IndirectEmitContext {
        allocs,
        op_name: "tail_call_indirect",
        mode: IndirectLoweringMode::TailCall,
        spill_words: &[],
        join_pc: None,
    };

    let trap_pc = b.alloc_block();
    let dispatch_pc = b.alloc_block();
    let case_blocks = build_indirect_dispatch(
        b,
        module,
        IndirectDispatchRequest {
            type_index,
            table_index,
            arg_count: arg_words.len(),
            index_word,
            trap_pc,
            dispatch_pc,
            op_name: "tail_call_indirect",
        },
    )?;

    for (case_pc, kind) in case_blocks {
        b.switch_to(case_pc);
        emit_indirect_target_case(b, kind, &arg_words, &emit_ctx)?;
    }

    Ok(())
}

fn build_indirect_dispatch(
    b: &mut FuncBuilder,
    module: &IrModule,
    req: IndirectDispatchRequest<'_>,
) -> anyhow::Result<Vec<(Pc, IndirectTargetKind)>> {
    let table = module.table_at(req.table_index).context({
        format!(
            "{} references table {} which does not exist",
            req.op_name, req.table_index
        )
    })?;
    anyhow::ensure!(
        table.entries().len() <= 256,
        "{} table {} has {} entries; max supported is 256",
        req.op_name,
        req.table_index,
        table.entries().len()
    );
    let expected_sig = module.type_at(req.type_index).context({
        format!(
            "{} references type {} which does not exist",
            req.op_name, req.type_index
        )
    })?;
    anyhow::ensure!(
        expected_sig.params().len() == req.arg_count,
        "{} type {} expects {} arg(s), got {}",
        req.op_name,
        req.type_index,
        expected_sig.params().len(),
        req.arg_count
    );

    // TODO(i64): indirect dispatch currently consumes only the low byte of the index word.
    // Switch indexes only use one byte; trap if high bytes are non-zero.
    let zero = Val8::imm(0);
    let b1_nonzero = b.alloc_reg();
    b.emit(Inst8::with_dst(
        b1_nonzero,
        Inst8Kind::Ne(req.index_word.b1, zero),
    ));
    let b2_nonzero = b.alloc_reg();
    b.emit(Inst8::with_dst(
        b2_nonzero,
        Inst8Kind::Ne(req.index_word.b2, zero),
    ));
    let b3_nonzero = b.alloc_reg();
    b.emit(Inst8::with_dst(
        b3_nonzero,
        Inst8Kind::Ne(req.index_word.b3, zero),
    ));
    let any_hi_nonzero =
        emit_bool_chain(b, &[b1_nonzero, b2_nonzero, b3_nonzero], Inst8Kind::BoolOr);
    b.finish(Terminator8::Branch {
        cond: any_hi_nonzero,
        if_true: req.trap_pc,
        if_false: req.dispatch_pc,
    });

    b.switch_to(req.trap_pc);
    b.finish(Terminator8::Trap(TrapCode::Unreachable));

    b.switch_to(req.dispatch_pc);
    let mut switch_targets = Vec::with_capacity(table.entries().len());
    let mut case_blocks = Vec::new();
    for entry in table.entries() {
        let Some(func_index) = *entry else {
            switch_targets.push(req.trap_pc);
            continue;
        };
        let Some(kind) = resolve_indirect_target(module, func_index, req.type_index) else {
            switch_targets.push(req.trap_pc);
            continue;
        };
        let case_pc = b.alloc_block();
        switch_targets.push(case_pc);
        case_blocks.push((case_pc, kind));
    }
    b.finish(Terminator8::Switch {
        index: req.index_word.b0,
        targets: switch_targets,
        default: req.trap_pc,
    });

    Ok(case_blocks)
}

fn resolve_indirect_target(
    module: &IrModule,
    func_index: u32,
    type_index: u32,
) -> Option<IndirectTargetKind> {
    let expected = module.type_at(type_index)?;
    let actual = module.func_type_at(func_index)?;
    if actual.params() != expected.params() || actual.results() != expected.results() {
        return None;
    }

    if Some(func_index) == module.putchar_import() {
        return Some(IndirectTargetKind::Putchar);
    }
    if Some(func_index) == module.getchar_import() {
        return Some(IndirectTargetKind::Getchar);
    }
    if module.body_at(func_index).is_some() {
        return Some(IndirectTargetKind::Direct(func_index));
    }
    None
}
