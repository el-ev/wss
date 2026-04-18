use std::collections::{HashMap, HashSet};

use anyhow::Context;

use crate::ast::{Node, RelOp};
use crate::ir::{BasicBlock, BlockId, Inst, IrNode, Terminator};
use crate::module::{AstModule, IrFuncBody, IrModule};

pub fn lower_module(module: AstModule) -> anyhow::Result<IrModule> {
    let functions_ir = module
        .bodies()
        .iter()
        .enumerate()
        .map(|(func_index, body)| lower_function(&module, func_index as u32, body.as_ref()))
        .collect::<anyhow::Result<Vec<_>>>()?;
    Ok(IrModule::new(module.into_info(), functions_ir))
}

fn lower_function(
    module: &AstModule,
    func_index: u32,
    body: Option<&crate::module::AstFuncBody>,
) -> anyhow::Result<Option<IrFuncBody>> {
    let ir_body = if let Some(body) = body {
        let is_entry = module.entry_export() == Some(func_index);
        let uses_eh = module_uses_exceptions(module);
        let mut ctx = LowerCtx::new(module, is_entry, uses_eh);
        let uncaught_exit = if uses_eh {
            let uncaught = ctx.builder.alloc_block();
            ctx.builder.eh_stack.push(uncaught);
            Some(uncaught)
        } else {
            None
        };
        lower_block_nodes(body.insts(), &mut ctx)?;
        // Always close the current block. Ending instructions like Return,
        // Br, Throw, or the tail of a Try leave `curr_blk` switched to a
        // fresh block that may or may not be targeted. If it's dead the
        // pass below prunes it; if a Try's merge target is still current
        // and reachable, this gives it a valid terminator.
        ctx.builder.finish_block(Terminator::Return(None));
        if let Some(uncaught) = uncaught_exit {
            // Close the function-level uncaught-exit block. Control reaches
            // here when no handler matched. The lower8 layer translates
            // `UncaughtExit` based on `is_entry`: the entry function traps
            // with `TrapCode::UncaughtException`; all other functions return
            // to the caller with the current exception state still set so
            // the caller's post-call check re-propagates.
            let popped = ctx.builder.eh_stack.pop();
            debug_assert_eq!(popped, Some(uncaught));
            ctx.builder.switch_to_block(uncaught);
            ctx.builder.finish_block(Terminator::UncaughtExit);
        }
        let mut blocks = ctx.builder.blocks;
        let mut entry = BlockId(0);

        remove_empty_blocks(&mut blocks, &mut entry);
        renumber_blocks(&mut blocks, &mut entry);
        remove_dead_blocks(&mut blocks, &mut entry)?;
        renumber_blocks(&mut blocks, &mut entry);

        Some(IrFuncBody::new(body.locals().to_vec(), entry, blocks))
    } else {
        None
    };
    Ok(ir_body)
}

fn remap_block_target(target: &mut BlockId, remap: &HashMap<BlockId, BlockId>) {
    if let Some(&new_target) = remap.get(target) {
        *target = new_target;
    }
}

fn remap_terminator_block_targets(term: &mut Terminator, remap: &HashMap<BlockId, BlockId>) {
    match term {
        Terminator::Goto(target) => remap_block_target(target, remap),
        Terminator::Branch {
            if_true, if_false, ..
        } => {
            remap_block_target(if_true, remap);
            remap_block_target(if_false, remap);
        }
        Terminator::Switch {
            targets, default, ..
        } => {
            targets
                .iter_mut()
                .for_each(|target| remap_block_target(target, remap));
            remap_block_target(default, remap);
        }
        Terminator::TailCall { .. }
        | Terminator::TailCallIndirect { .. }
        | Terminator::Return(_)
        | Terminator::Unreachable
        | Terminator::UncaughtExit => {}
    }
}

fn remove_empty_blocks(blocks: &mut Vec<BasicBlock>, entry: &mut BlockId) {
    let mut redirects = HashMap::new();
    for block in blocks.iter() {
        if block.insts.is_empty()
            && let Terminator::Goto(target) = block.terminator
            && block.id != target
        {
            redirects.insert(block.id, target);
        }
    }

    if redirects.is_empty() {
        return;
    }

    let mut resolved = HashMap::new();
    for &id in redirects.keys() {
        let mut curr = id;
        let mut visited = HashSet::new();
        visited.insert(curr);
        while let Some(&next) = redirects.get(&curr) {
            if visited.contains(&next) {
                break;
            }
            visited.insert(next);
            curr = next;
        }
        resolved.insert(id, curr);
    }

    blocks
        .iter_mut()
        .for_each(|block| remap_terminator_block_targets(&mut block.terminator, &resolved));
    remap_block_target(entry, &resolved);

    blocks.retain(|b| {
        !(b.insts.is_empty() && matches!(b.terminator, Terminator::Goto(t) if t != b.id))
    });
}

fn renumber_blocks(blocks: &mut [BasicBlock], entry: &mut BlockId) {
    let old_to_new: HashMap<BlockId, BlockId> = blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, BlockId(i)))
        .collect();
    let new_entry = *old_to_new.get(entry).unwrap_or(entry);
    for block in blocks.iter_mut() {
        block.id = *old_to_new.get(&block.id).unwrap_or(&block.id);
        remap_terminator_block_targets(&mut block.terminator, &old_to_new);
    }
    *entry = new_entry;
}

fn remove_dead_blocks(blocks: &mut Vec<BasicBlock>, entry: &mut BlockId) -> anyhow::Result<()> {
    let mut visited = HashSet::new();
    let mut stack = Vec::new();
    stack.push(*entry);
    while let Some(id) = stack.pop() {
        if visited.contains(&id) {
            continue;
        }
        visited.insert(id);
        let block = blocks.get(id.index()).with_context(|| {
            format!(
                "invalid block id {} during dead block elimination",
                id.index()
            )
        })?;
        stack.extend(block.successors());
    }

    let mut ref_remap = HashMap::new();
    let mut old_ref_base = IrNode(0);
    let mut new_ref_base = IrNode(0);
    for block in blocks.iter() {
        let inst_count = block.insts.len();
        if visited.contains(&block.id) {
            for i in 0..inst_count {
                ref_remap.insert(old_ref_base + i, new_ref_base + i);
            }
            new_ref_base += inst_count;
        }
        old_ref_base += inst_count;
    }

    blocks.retain(|b| visited.contains(&b.id));
    for block in blocks.iter_mut() {
        for inst in block.insts.iter_mut() {
            remap_inst_refs(inst, &ref_remap)?;
        }
        remap_terminator_refs(&mut block.terminator, &ref_remap)?;
    }
    Ok(())
}

fn remap_ref(r: &mut IrNode, remap: &HashMap<IrNode, IrNode>) -> anyhow::Result<()> {
    if r.is_imm() {
        return Ok(());
    }
    let old = *r;
    *r = *remap
        .get(&old)
        .with_context(|| format!("missing IrRef remap for {}", old))?;
    Ok(())
}

fn remap_refs<'a>(
    refs: impl IntoIterator<Item = &'a mut IrNode>,
    remap: &HashMap<IrNode, IrNode>,
) -> anyhow::Result<()> {
    refs.into_iter().try_for_each(|r| remap_ref(r, remap))
}

fn remap_inst_refs(inst: &mut Inst, remap: &HashMap<IrNode, IrNode>) -> anyhow::Result<()> {
    match inst {
        Inst::I32Const(_)
        | Inst::LocalGet(_)
        | Inst::GlobalGet(_)
        | Inst::MemorySize
        | Inst::TableSize(_)
        | Inst::Getchar
        | Inst::Drop
        | Inst::ExcSet { .. }
        | Inst::ExcClear
        | Inst::ExcFlagGet
        | Inst::ExcTagGet
        | Inst::ExcPayloadGet => {}
        Inst::LocalSet(_, v)
        | Inst::LocalTee(_, v)
        | Inst::GlobalSet(_, v)
        | Inst::Unary(_, v)
        | Inst::Putchar(v)
        | Inst::Load { addr: v, .. }
        | Inst::ExcPayloadSet(v) => remap_ref(v, remap)?,
        Inst::Binary(_, l, r)
        | Inst::Compare(_, l, r)
        | Inst::Store {
            addr: l, val: r, ..
        } => {
            remap_ref(l, remap)?;
            remap_ref(r, remap)?;
        }
        Inst::Select {
            cond,
            if_true,
            if_false,
        } => {
            remap_refs([cond, if_true, if_false], remap)?;
        }
        Inst::Call { args, .. } => remap_refs(args.iter_mut(), remap)?,
        Inst::CallIndirect { index, args, .. } => {
            remap_ref(index, remap)?;
            remap_refs(args.iter_mut(), remap)?;
        }
    }
    Ok(())
}

fn remap_terminator_refs(
    term: &mut Terminator,
    remap: &HashMap<IrNode, IrNode>,
) -> anyhow::Result<()> {
    match term {
        Terminator::Goto(_) | Terminator::Unreachable | Terminator::UncaughtExit => {}
        Terminator::Branch { cond, .. } | Terminator::Switch { index: cond, .. } => {
            remap_ref(cond, remap)?;
        }
        Terminator::TailCall { args, .. } => remap_refs(args.iter_mut(), remap)?,
        Terminator::TailCallIndirect { index, args, .. } => {
            remap_ref(index, remap)?;
            remap_refs(args.iter_mut(), remap)?;
        }
        Terminator::Return(Some(v)) => remap_ref(v, remap)?,
        Terminator::Return(None) => {}
    }
    Ok(())
}

struct LowerCtx<'a> {
    module: &'a AstModule,
    is_entry: bool,
    builder: IrBuilder,
    /// When `Some`, overrides the default `ref_map` entry for the current
    /// AST node (used when an arm emits extra instructions after its
    /// value-producing one, e.g. post-call exception checks).
    last_ref_override: Option<IrNode>,
    /// Whether this function is lowered in exception-handling mode. True
    /// iff the module declares any exception tags.
    uses_eh: bool,
}

impl<'a> LowerCtx<'a> {
    fn new(module: &'a AstModule, is_entry: bool, uses_eh: bool) -> Self {
        Self {
            module,
            is_entry,
            builder: IrBuilder::new(),
            last_ref_override: None,
            uses_eh,
        }
    }

    fn resolve_depth(&self, depth: u32) -> anyhow::Result<BlockId> {
        let idx = self
            .builder
            .label_stack
            .len()
            .checked_sub(1 + depth as usize)
            .ok_or_else(|| anyhow::anyhow!("br depth {} exceeds label stack depth", depth))?;
        Ok(self.builder.label_stack[idx])
    }
}

#[derive(Default)]
pub struct IrBuilder {
    pub blocks: Vec<BasicBlock>,
    pub curr_blk_id: BlockId,
    pub curr_blk_insts: Vec<Inst>,
    pub next_blk_id: BlockId,
    pub label_stack: Vec<BlockId>,
    pub next_ref: IrNode,
    /// Stack of exception handler dispatch blocks. The top is the target
    /// for `throw`, `rethrow`, and post-call exception checks active in the
    /// current scope. The bottom (if EH is active) is the function-level
    /// uncaught-exit block, which returns control to the caller with the
    /// current exception state still set so propagation continues.
    pub eh_stack: Vec<BlockId>,
    /// Tracks the tag captured by each enclosing catch body, innermost
    /// last. `Some(idx)` for typed catches; `None` for `catch_all`.
    pub catch_tag_stack: Vec<Option<u32>>,
}

impl IrBuilder {
    pub fn new() -> Self {
        Self {
            next_blk_id: BlockId(1),
            ..Default::default()
        }
    }

    pub fn alloc_block(&mut self) -> BlockId {
        let id = self.next_blk_id;
        self.next_blk_id = BlockId(self.next_blk_id.index() + 1);
        id
    }

    pub fn push(&mut self, inst: Inst) -> IrNode {
        let r = self.next_ref;
        self.curr_blk_insts.push(inst);
        self.next_ref += 1;
        r
    }

    pub fn finish_block(&mut self, terminator: Terminator) {
        self.blocks.push(BasicBlock {
            id: self.curr_blk_id,
            insts: std::mem::take(&mut self.curr_blk_insts),
            terminator,
        });
    }

    pub fn switch_to_block(&mut self, id: BlockId) {
        self.curr_blk_id = id;
    }

    pub fn finish_and_switch(&mut self, terminator: Terminator, next_id: BlockId) {
        self.finish_block(terminator);
        self.switch_to_block(next_id);
    }
}

fn lower_block_nodes(block: &[Node], ctx: &mut LowerCtx) -> anyhow::Result<()> {
    let mut ref_map: Vec<IrNode> = Vec::with_capacity(block.len());
    for node in block.iter() {
        ctx.last_ref_override = None;
        lower_node(node, block, ctx, &ref_map)?;
        let result_ref = ctx
            .last_ref_override
            .take()
            .unwrap_or_else(|| ctx.builder.next_ref.saturating_sub(1));
        ref_map.push(result_ref);
    }
    Ok(())
}

fn module_uses_exceptions(module: &AstModule) -> bool {
    module.info().tag_at(0).is_some()
}

fn ast_operand_ref(ast_ref: crate::ast::AstRef, block: &[Node], ref_map: &[IrNode]) -> IrNode {
    match block[ast_ref.index()] {
        // TODO(i64): fast-path immediates are currently encoded only for i32 consts.
        Node::I32Const(v) => IrNode::imm_i32(v),
        _ => ref_map[ast_ref.index()],
    }
}

fn map_ast_args(args: &[crate::ast::AstRef], ref_map: &[IrNode]) -> Vec<IrNode> {
    args.iter().map(|&a| ref_map[a.index()]).collect()
}

fn lower_node(
    node: &Node,
    block: &[Node],
    ctx: &mut LowerCtx,
    ref_map: &[IrNode],
) -> anyhow::Result<()> {
    let builder = &mut ctx.builder;
    match node {
        // TODO(i64): AST-to-IR lowering currently emits i32 const instructions only.
        Node::I32Const(v) => {
            builder.push(Inst::I32Const(*v));
        }
        Node::LocalGet(l) => {
            builder.push(Inst::LocalGet(*l));
        }
        Node::LocalTee(l, r) => {
            builder.push(Inst::LocalTee(*l, ref_map[r.index()]));
        }
        Node::GlobalGet(g) => {
            builder.push(Inst::GlobalGet(*g));
        }
        Node::MemorySize => {
            builder.push(Inst::MemorySize);
        }
        Node::TableSize(t) => {
            builder.push(Inst::TableSize(*t));
        }
        Node::Unary(op, r) => {
            builder.push(Inst::Unary(*op, ref_map[r.index()]));
        }
        Node::Binary(op, l, r) => {
            builder.push(Inst::Binary(
                *op,
                ast_operand_ref(*l, block, ref_map),
                ast_operand_ref(*r, block, ref_map),
            ));
        }
        Node::Compare(op, l, r) => {
            builder.push(Inst::Compare(
                *op,
                ast_operand_ref(*l, block, ref_map),
                ast_operand_ref(*r, block, ref_map),
            ));
        }
        Node::Select {
            cond,
            then_val,
            else_val,
        } => {
            ctx.builder.push(Inst::Select {
                cond: ref_map[cond.index()],
                if_true: ref_map[then_val.index()],
                if_false: ref_map[else_val.index()],
            });
        }
        Node::Load {
            size,
            signed,
            offset,
            address,
        } => {
            ctx.builder.push(Inst::Load {
                size: *size as u8,
                signed: *signed,
                offset: *offset as u32,
                addr: ref_map[address.index()],
            });
        }
        Node::Call(func, args) => {
            if Some(*func) == ctx.module.putchar_import() {
                ctx.builder.push(Inst::Putchar(ref_map[args[0].index()]));
            } else if Some(*func) == ctx.module.getchar_import() {
                ctx.builder.push(Inst::Getchar);
            } else {
                let call_ref = ctx.builder.push(Inst::Call {
                    func: *func,
                    args: map_ast_args(args, ref_map),
                });
                emit_post_call_check(ctx, call_ref);
            }
        }
        Node::CallIndirect {
            type_index,
            table_index,
            index,
            args,
        } => {
            let call_ref = ctx.builder.push(Inst::CallIndirect {
                type_index: *type_index,
                table_index: *table_index,
                index: ref_map[index.index()],
                args: map_ast_args(args, ref_map),
            });
            emit_post_call_check(ctx, call_ref);
        }
        Node::Drop(_) => {
            ctx.builder.push(Inst::Drop);
        }
        Node::LocalSet(l, r) => {
            ctx.builder.push(Inst::LocalSet(*l, ref_map[r.index()]));
        }
        Node::GlobalSet(g, r) => {
            ctx.builder.push(Inst::GlobalSet(*g, ref_map[r.index()]));
        }
        Node::Store {
            size,
            offset,
            value,
            address,
        } => {
            ctx.builder.push(Inst::Store {
                size: *size as u8,
                offset: *offset as u32,
                val: ref_map[value.index()],
                addr: ref_map[address.index()],
            });
        }
        Node::Return(val) => {
            let after = ctx.builder.alloc_block();
            let return_ref = val.map(|r| ref_map[r.index()]);
            if let Some(tail) = maybe_fuse_tail_call(ctx, return_ref)? {
                ctx.builder.finish_and_switch(tail, after);
                return Ok(());
            }
            ctx.builder
                .finish_and_switch(Terminator::Return(return_ref), after);
        }
        Node::Unreachable => {
            let after = ctx.builder.alloc_block();
            ctx.builder
                .finish_and_switch(Terminator::Unreachable, after);
        }

        Node::Block(body) => {
            let builder = &mut ctx.builder;
            let body_id = builder.alloc_block();
            let end_id = builder.alloc_block();
            builder.finish_and_switch(Terminator::Goto(body_id), body_id);
            builder.label_stack.push(end_id);
            lower_block_nodes(body, ctx)?;
            ctx.builder.label_stack.pop();
            ctx.builder
                .finish_and_switch(Terminator::Goto(end_id), end_id);
        }
        Node::Loop(body) => {
            let builder = &mut ctx.builder;
            let loop_id = builder.alloc_block();
            let end_id = builder.alloc_block();
            builder.finish_and_switch(Terminator::Goto(loop_id), loop_id);
            builder.label_stack.push(loop_id);
            lower_block_nodes(body, ctx)?;
            ctx.builder.label_stack.pop();
            let term = if ctx.builder.curr_blk_insts.is_empty() {
                Terminator::Goto(end_id)
            } else {
                Terminator::Goto(loop_id)
            };
            ctx.builder.finish_and_switch(term, end_id);
        }
        Node::If {
            cond,
            then_body,
            else_body,
        } => {
            let builder = &mut ctx.builder;
            let then_id = builder.alloc_block();
            let else_id = builder.alloc_block();
            let end_id = builder.alloc_block();

            builder.finish_and_switch(
                Terminator::Branch {
                    cond: ref_map[cond.index()],
                    if_true: then_id,
                    if_false: else_id,
                },
                then_id,
            );

            builder.label_stack.push(end_id);
            lower_block_nodes(then_body, ctx)?;
            let builder = &mut ctx.builder;
            builder.label_stack.pop();
            builder.finish_and_switch(Terminator::Goto(end_id), else_id);

            builder.label_stack.push(end_id);
            lower_block_nodes(else_body, ctx)?;
            ctx.builder.label_stack.pop();
            ctx.builder
                .finish_and_switch(Terminator::Goto(end_id), end_id);
        }

        Node::Br(depth) => {
            let target = ctx.resolve_depth(*depth)?;
            let after = ctx.builder.alloc_block();
            ctx.builder
                .finish_and_switch(Terminator::Goto(target), after);
        }
        Node::BrIf(depth, cond) => {
            let target = ctx.resolve_depth(*depth)?;
            let fallthrough = ctx.builder.alloc_block();
            ctx.builder.finish_and_switch(
                Terminator::Branch {
                    cond: ref_map[cond.index()],
                    if_true: target,
                    if_false: fallthrough,
                },
                fallthrough,
            );
        }
        Node::BrTable(targets, default, index) => {
            let target_ids: Vec<BlockId> = targets
                .iter()
                .map(|d| ctx.resolve_depth(*d))
                .collect::<anyhow::Result<Vec<_>>>()?;
            let default_id = ctx.resolve_depth(*default)?;
            let after = ctx.builder.alloc_block();
            ctx.builder.finish_and_switch(
                Terminator::Switch {
                    index: ref_map[index.index()],
                    targets: target_ids,
                    default: default_id,
                },
                after,
            );
        }
        Node::Try {
            body,
            catches,
            catch_all,
            delegate,
        } => {
            lower_try(ctx, body, catches, catch_all.as_deref(), *delegate)?;
        }
        Node::Throw { tag, arg } => {
            if let Some(arg_ref) = arg {
                let val = ref_map[arg_ref.index()];
                ctx.builder.push(Inst::ExcPayloadSet(val));
            }
            ctx.builder.push(Inst::ExcSet { tag_index: *tag });
            let handler = ctx
                .builder
                .eh_stack
                .last()
                .copied()
                .context("throw outside exception handler scope")?;
            let after = ctx.builder.alloc_block();
            ctx.builder
                .finish_and_switch(Terminator::Goto(handler), after);
        }
        Node::ExcPayloadGet => {
            ctx.builder.push(Inst::ExcPayloadGet);
        }
        Node::Rethrow(depth) => {
            if *depth != 0 {
                anyhow::bail!(
                    "rethrow with depth > 0 not supported (got depth {})",
                    depth
                );
            }
            let tag_idx = ctx
                .builder
                .catch_tag_stack
                .last()
                .copied()
                .context("rethrow outside exception handler scope")?;
            let tag_index = tag_idx.context(
                "rethrow from catch_all is not supported (no static tag to re-raise)",
            )?;
            ctx.builder.push(Inst::ExcSet { tag_index });
            let handler = ctx
                .builder
                .eh_stack
                .last()
                .copied()
                .context("rethrow outside exception handler scope")?;
            let after = ctx.builder.alloc_block();
            ctx.builder
                .finish_and_switch(Terminator::Goto(handler), after);
        }
    }
    Ok(())
}

fn emit_post_call_check(ctx: &mut LowerCtx, call_ref: IrNode) {
    if !ctx.uses_eh {
        return;
    }
    let handler = match ctx.builder.eh_stack.last().copied() {
        Some(h) => h,
        None => return,
    };
    let flag = ctx.builder.push(Inst::ExcFlagGet);
    let continue_block = ctx.builder.alloc_block();
    ctx.builder.finish_and_switch(
        Terminator::Branch {
            cond: flag,
            if_true: handler,
            if_false: continue_block,
        },
        continue_block,
    );
    // Preserve the call's ref as this AST node's produced value; the
    // flag-get and branch we just emitted are plumbing, not a value.
    ctx.last_ref_override = Some(call_ref);
}

fn lower_try(
    ctx: &mut LowerCtx,
    body: &[Node],
    catches: &[crate::ast::Catch],
    catch_all: Option<&[Node]>,
    delegate: Option<u32>,
) -> anyhow::Result<()> {
    // Delegate: body's exceptions skip local catches and go straight to
    // the handler at the given label depth. The current lowering only
    // supports depth 0 (the immediately enclosing scope's handler).
    if let Some(depth) = delegate {
        if depth != 0 {
            anyhow::bail!(
                "try ... delegate with depth > 0 not supported (got depth {})",
                depth
            );
        }
        let body_block = ctx.builder.alloc_block();
        let merge_block = ctx.builder.alloc_block();
        ctx.builder
            .finish_and_switch(Terminator::Goto(body_block), body_block);
        ctx.builder.label_stack.push(merge_block);
        lower_block_nodes(body, ctx)?;
        ctx.builder.label_stack.pop();
        ctx.builder
            .finish_and_switch(Terminator::Goto(merge_block), merge_block);
        return Ok(());
    }

    let body_block = ctx.builder.alloc_block();
    let dispatch_block = ctx.builder.alloc_block();
    let merge_block = ctx.builder.alloc_block();
    let uncaught_forward_block = ctx.builder.alloc_block();
    let catch_blocks: Vec<BlockId> = (0..catches.len())
        .map(|_| ctx.builder.alloc_block())
        .collect();
    let catch_all_block = catch_all.as_ref().map(|_| ctx.builder.alloc_block());

    ctx.builder
        .finish_and_switch(Terminator::Goto(body_block), body_block);
    ctx.builder.label_stack.push(merge_block);
    ctx.builder.eh_stack.push(dispatch_block);
    lower_block_nodes(body, ctx)?;
    let popped = ctx.builder.eh_stack.pop();
    debug_assert_eq!(popped, Some(dispatch_block));
    ctx.builder.label_stack.pop();
    ctx.builder
        .finish_and_switch(Terminator::Goto(merge_block), dispatch_block);

    // Dispatch: test current exc_tag against each catch's tag_index.
    for (i, catch) in catches.iter().enumerate() {
        let tag = ctx.builder.push(Inst::ExcTagGet);
        let expected = IrNode::imm_i32(catch.tag_index as i32);
        let cmp = ctx.builder.push(Inst::Compare(RelOp::Eq, tag, expected));
        let next_check = ctx.builder.alloc_block();
        ctx.builder.finish_and_switch(
            Terminator::Branch {
                cond: cmp,
                if_true: catch_blocks[i],
                if_false: next_check,
            },
            next_check,
        );
    }
    let dispatch_tail = catch_all_block.unwrap_or(uncaught_forward_block);
    ctx.builder.finish_and_switch(
        Terminator::Goto(dispatch_tail),
        uncaught_forward_block,
    );

    // Uncaught forward: no catch matched; propagate to outer handler.
    let outer_handler = ctx
        .builder
        .eh_stack
        .last()
        .copied()
        .context("try uncaught forward without outer handler")?;
    ctx.builder
        .finish_and_switch(Terminator::Goto(outer_handler), merge_block);

    // Catch bodies.
    for (i, catch) in catches.iter().enumerate() {
        ctx.builder.switch_to_block(catch_blocks[i]);
        ctx.builder.push(Inst::ExcClear);
        ctx.builder.label_stack.push(merge_block);
        ctx.builder.catch_tag_stack.push(Some(catch.tag_index));
        lower_block_nodes(&catch.body, ctx)?;
        ctx.builder.catch_tag_stack.pop();
        ctx.builder.label_stack.pop();
        ctx.builder
            .finish_and_switch(Terminator::Goto(merge_block), merge_block);
    }

    // Catch-all body.
    if let (Some(catch_all_body), Some(catch_all_id)) = (catch_all, catch_all_block) {
        ctx.builder.switch_to_block(catch_all_id);
        ctx.builder.push(Inst::ExcClear);
        ctx.builder.label_stack.push(merge_block);
        ctx.builder.catch_tag_stack.push(None);
        lower_block_nodes(catch_all_body, ctx)?;
        ctx.builder.catch_tag_stack.pop();
        ctx.builder.label_stack.pop();
        ctx.builder
            .finish_and_switch(Terminator::Goto(merge_block), merge_block);
    }

    // Subsequent nodes continue in merge_block (switched into above).
    Ok(())
}

fn call_returns_void(module: &AstModule, func: u32) -> anyhow::Result<bool> {
    let sig = module
        .func_type_at(func)
        .with_context(|| format!("tail-call fusion: function index {} out of bounds", func))?;
    Ok(sig.results().is_empty())
}

fn call_indirect_returns_void(module: &AstModule, type_index: u32) -> anyhow::Result<bool> {
    let sig = module
        .type_at(type_index)
        .with_context(|| format!("tail-call fusion: type index {} out of bounds", type_index))?;
    Ok(sig.results().is_empty())
}

fn maybe_fuse_tail_call(
    ctx: &mut LowerCtx<'_>,
    return_ref: Option<IrNode>,
) -> anyhow::Result<Option<Terminator>> {
    if ctx.is_entry || ctx.builder.curr_blk_insts.is_empty() {
        return Ok(None);
    }
    let last_inst_ref = ctx.builder.next_ref.saturating_sub(1);
    let tail = match ctx.builder.curr_blk_insts.last() {
        Some(Inst::Call { func, args }) => {
            let should_fuse = match return_ref {
                Some(ret) => ret == last_inst_ref,
                None => call_returns_void(ctx.module, *func)?,
            };
            if should_fuse {
                Some(Terminator::TailCall {
                    func: *func,
                    args: args.clone(),
                })
            } else {
                None
            }
        }
        Some(Inst::CallIndirect {
            type_index,
            table_index,
            index,
            args,
        }) => {
            let should_fuse = match return_ref {
                Some(ret) => ret == last_inst_ref,
                None => call_indirect_returns_void(ctx.module, *type_index)?,
            };
            if should_fuse {
                Some(Terminator::TailCallIndirect {
                    type_index: *type_index,
                    table_index: *table_index,
                    index: *index,
                    args: args.clone(),
                })
            } else {
                None
            }
        }
        _ => None,
    };
    if tail.is_some() {
        let _ = ctx.builder.curr_blk_insts.pop();
    }
    Ok(tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ast::{AstRef, BinOp, Catch, Node, RelOp};
    use crate::module::{AstFuncBody, AstModule, FuncType, ModuleInfo, TagInfo};
    use wasmparser::ValType;

    fn mk_module_with_tag(
        types: Vec<FuncType>,
        functions_ast: Vec<(FuncType, Option<AstFuncBody>)>,
        tag_type_index: u32,
    ) -> AstModule {
        let function_types = functions_ast
            .iter()
            .map(|(sig, _)| sig.clone())
            .collect::<Vec<_>>();
        let bodies = functions_ast
            .into_iter()
            .map(|(_, body)| body)
            .collect::<Vec<_>>();
        let mut info = ModuleInfo::default();
        *info.types_mut() = types;
        *info.functions_mut() = function_types;
        info.tags_mut().push(TagInfo::new(tag_type_index));
        AstModule::new(info, bodies)
    }

    fn mk_sig(params: &[ValType], results: &[ValType]) -> FuncType {
        FuncType::new(
            params.to_vec().into_boxed_slice(),
            results.to_vec().into_boxed_slice(),
        )
    }

    fn mk_module(
        entry_export: Option<u32>,
        types: Vec<FuncType>,
        functions_ast: Vec<(FuncType, Option<AstFuncBody>)>,
    ) -> AstModule {
        let function_types = functions_ast
            .iter()
            .map(|(sig, _)| sig.clone())
            .collect::<Vec<_>>();
        let bodies = functions_ast
            .into_iter()
            .map(|(_, body)| body)
            .collect::<Vec<_>>();
        let mut info = ModuleInfo::default();
        info.set_entry_export(entry_export);
        *info.types_mut() = types;
        *info.functions_mut() = function_types;
        AstModule::new(info, bodies)
    }

    #[test]
    fn lower_remove_dead_blocks_remaps_ir_refs() {
        let mut blocks = vec![
            BasicBlock {
                id: BlockId(0),
                insts: vec![Inst::I32Const(1)],
                terminator: Terminator::Goto(BlockId(2)),
            },
            BasicBlock {
                id: BlockId(1),
                insts: vec![Inst::I32Const(99)],
                terminator: Terminator::Goto(BlockId(2)),
            },
            BasicBlock {
                id: BlockId(2),
                insts: vec![
                    Inst::LocalGet(0),
                    Inst::Binary(BinOp::Add, IrNode(0), IrNode(2)),
                ],
                terminator: Terminator::Return(Some(IrNode(3))),
            },
        ];
        let mut entry = BlockId(0);

        remove_dead_blocks(&mut blocks, &mut entry).expect("dead block elimination should succeed");

        assert_eq!(blocks.len(), 2);
        assert_eq!(blocks[0].id, BlockId(0));
        assert_eq!(blocks[1].id, BlockId(2));
        assert!(matches!(blocks[1].insts[0], Inst::LocalGet(0)));
        assert!(matches!(
            blocks[1].insts[1],
            Inst::Binary(BinOp::Add, IrNode(0), IrNode(1))
        ));
        assert!(matches!(
            blocks[1].terminator,
            Terminator::Return(Some(IrNode(2)))
        ));
    }

    #[test]
    fn lower_non_main_call_return_fuses_to_tail_call() {
        let module = mk_module(
            Some(0),
            vec![],
            vec![
                (mk_sig(&[ValType::I32], &[ValType::I32]), None),
                (
                    mk_sig(&[], &[ValType::I32]),
                    Some(AstFuncBody::new(
                        vec![],
                        vec![
                            Node::I32Const(7),
                            Node::Call(0, vec![AstRef::new(0)]),
                            Node::Return(Some(AstRef::new(1))),
                        ],
                    )),
                ),
            ],
        );

        let ir = lower_function(&module, 1, module.body_at(1)).expect("lower_function");
        let body = ir.expect("expected function body");
        assert_eq!(body.blocks().len(), 1);
        let block = &body.blocks()[0];
        assert_eq!(
            block.insts.len(),
            1,
            "tail-call fusion should remove trailing call"
        );
        assert!(matches!(block.insts[0], Inst::I32Const(7)));
        assert!(matches!(
            &block.terminator,
            Terminator::TailCall { func: 0, args } if args.as_slice() == [IrNode(0)]
        ));
    }

    #[test]
    fn lower_non_main_call_indirect_return_fuses_to_tail_call_indirect() {
        let module = mk_module(
            Some(0),
            vec![mk_sig(&[ValType::I32], &[ValType::I32])],
            vec![
                (mk_sig(&[ValType::I32], &[ValType::I32]), None),
                (
                    mk_sig(&[], &[ValType::I32]),
                    Some(AstFuncBody::new(
                        vec![],
                        vec![
                            Node::I32Const(7),
                            Node::I32Const(0),
                            Node::CallIndirect {
                                type_index: 0,
                                table_index: 0,
                                index: AstRef::new(1),
                                args: vec![AstRef::new(0)],
                            },
                            Node::Return(Some(AstRef::new(2))),
                        ],
                    )),
                ),
            ],
        );

        let ir = lower_function(&module, 1, module.body_at(1)).expect("lower_function");
        let body = ir.expect("expected function body");
        assert_eq!(body.blocks().len(), 1);
        let block = &body.blocks()[0];
        assert_eq!(
            block.insts.len(),
            2,
            "tail-call fusion should remove trailing call_indirect"
        );
        assert!(matches!(
            &block.terminator,
            Terminator::TailCallIndirect {
                type_index: 0,
                table_index: 0,
                index,
                args
            } if *index == IrNode(1) && args.as_slice() == [IrNode(0)]
        ));
    }

    #[test]
    fn lower_non_main_void_call_return_none_fuses_to_tail_call() {
        let module = mk_module(
            Some(0),
            vec![],
            vec![
                (mk_sig(&[], &[]), None),
                (
                    mk_sig(&[], &[]),
                    Some(AstFuncBody::new(
                        vec![],
                        vec![Node::Call(0, vec![]), Node::Return(None)],
                    )),
                ),
            ],
        );

        let ir = lower_function(&module, 1, module.body_at(1)).expect("lower_function");
        let body = ir.expect("expected function body");
        assert_eq!(body.blocks().len(), 1);
        let block = &body.blocks()[0];
        assert!(
            block.insts.is_empty(),
            "tail-call fusion should remove the void trailing call"
        );
        assert!(matches!(
            &block.terminator,
            Terminator::TailCall { func: 0, args } if args.is_empty()
        ));
    }

    #[test]
    fn lower_main_call_return_does_not_fuse_to_tail_call() {
        let module = mk_module(
            Some(1),
            vec![],
            vec![
                (mk_sig(&[ValType::I32], &[ValType::I32]), None),
                (
                    mk_sig(&[], &[ValType::I32]),
                    Some(AstFuncBody::new(
                        vec![],
                        vec![
                            Node::I32Const(7),
                            Node::Call(0, vec![AstRef::new(0)]),
                            Node::Return(Some(AstRef::new(1))),
                        ],
                    )),
                ),
            ],
        );

        let ir = lower_function(&module, 1, module.body_at(1)).expect("lower_function");
        let body = ir.expect("expected function body");
        assert_eq!(body.blocks().len(), 1);
        let block = &body.blocks()[0];
        assert_eq!(
            block.insts.len(),
            2,
            "main should keep normal call+return lowering"
        );
        assert!(matches!(block.insts[1], Inst::Call { func: 0, .. }));
        assert!(matches!(
            block.terminator,
            Terminator::Return(Some(IrNode(1)))
        ));
    }

    #[test]
    fn lower_binary_and_compare_use_imm_operands_for_i32_const_inputs() {
        let module = mk_module(
            Some(0),
            vec![],
            vec![(
                mk_sig(&[ValType::I32], &[ValType::I32]),
                Some(AstFuncBody::new(
                    vec![],
                    vec![
                        Node::I32Const(10),                                        // 0
                        Node::LocalGet(0),                                         // 1
                        Node::Binary(BinOp::DivU, AstRef::new(1), AstRef::new(0)), // 2
                        Node::Compare(RelOp::Eq, AstRef::new(2), AstRef::new(0)),  // 3
                        Node::Return(Some(AstRef::new(3))),
                    ],
                )),
            )],
        );

        let ir = lower_function(&module, 0, module.body_at(0)).expect("lower_function");
        let body = ir.expect("expected function body");
        let block = &body.blocks()[0];

        assert!(matches!(
            block.insts[2],
            Inst::Binary(BinOp::DivU, IrNode(1), r) if r.imm_i32_value() == Some(10)
        ));
        assert!(matches!(
            block.insts[3],
            Inst::Compare(RelOp::Eq, IrNode(2), r) if r.imm_i32_value() == Some(10)
        ));
    }

    fn count_inst<F: Fn(&Inst) -> bool>(body: &IrFuncBody, pred: F) -> usize {
        body.blocks()
            .iter()
            .flat_map(|b| b.insts.iter())
            .filter(|i| pred(i))
            .count()
    }

    #[test]
    fn lower_try_catch_emits_dispatch_and_catch_bodies() {
        let module = mk_module_with_tag(
            vec![mk_sig(&[], &[])],
            vec![(
                mk_sig(&[], &[]),
                Some(AstFuncBody::new(
                    vec![],
                    vec![Node::Try {
                        body: vec![Node::Throw { tag: 0, arg: None }],
                        catches: vec![Catch {
                            tag_index: 0,
                            body: vec![],
                        }],
                        catch_all: None,
                        delegate: None,
                    }],
                )),
            )],
            0,
        );
        let ir = lower_function(&module, 0, module.body_at(0)).expect("lower");
        let body = ir.expect("body");

        // ExcSet{tag:0} emitted exactly once (for the throw).
        assert_eq!(
            count_inst(&body, |i| matches!(
                i,
                Inst::ExcSet { tag_index: 0 }
            )),
            1
        );
        // ExcTagGet present for dispatch; ExcClear at catch entry.
        assert!(count_inst(&body, |i| matches!(i, Inst::ExcTagGet)) >= 1);
        assert_eq!(count_inst(&body, |i| matches!(i, Inst::ExcClear)), 1);
    }

    #[test]
    fn lower_call_inside_try_emits_post_call_exc_flag_check() {
        let module = mk_module_with_tag(
            vec![mk_sig(&[], &[])],
            vec![
                (mk_sig(&[], &[]), None),
                (
                    mk_sig(&[], &[]),
                    Some(AstFuncBody::new(
                        vec![],
                        vec![Node::Try {
                            body: vec![Node::Call(0, vec![])],
                            catches: vec![Catch {
                                tag_index: 0,
                                body: vec![],
                            }],
                            catch_all: None,
                            delegate: None,
                        }],
                    )),
                ),
            ],
            0,
        );
        let ir = lower_function(&module, 1, module.body_at(1)).expect("lower");
        let body = ir.expect("body");

        // Post-call check: an ExcFlagGet is emitted after the call, and
        // at least one Branch uses it as its condition.
        assert!(count_inst(&body, |i| matches!(i, Inst::ExcFlagGet)) >= 1);
        let has_branch_on_flag = body.blocks().iter().any(|b| {
            matches!(&b.terminator, Terminator::Branch { .. })
                && b.insts.iter().any(|i| matches!(i, Inst::ExcFlagGet))
        });
        assert!(
            has_branch_on_flag,
            "expected a Branch terminator in a block containing ExcFlagGet"
        );
    }

    #[test]
    fn lower_module_without_tags_skips_eh_machinery() {
        // Same call pattern, but no tag registered: no EH lowering.
        let module = mk_module(
            None,
            vec![mk_sig(&[], &[])],
            vec![
                (mk_sig(&[], &[]), None),
                (
                    mk_sig(&[], &[]),
                    Some(AstFuncBody::new(vec![], vec![Node::Call(0, vec![])])),
                ),
            ],
        );
        let ir = lower_function(&module, 1, module.body_at(1)).expect("lower");
        let body = ir.expect("body");

        assert_eq!(count_inst(&body, |i| matches!(i, Inst::ExcFlagGet)), 0);
        assert_eq!(
            count_inst(&body, |i| matches!(i, Inst::ExcSet { .. })),
            0
        );
        assert_eq!(count_inst(&body, |i| matches!(i, Inst::ExcClear)), 0);
    }

    #[test]
    fn lower_try_delegate_zero_is_transparent() {
        // try { throw 0 } delegate 0 — no dispatch/catch blocks; throw
        // propagates to the outer (function-level) handler.
        let module = mk_module_with_tag(
            vec![mk_sig(&[], &[])],
            vec![(
                mk_sig(&[], &[]),
                Some(AstFuncBody::new(
                    vec![],
                    vec![Node::Try {
                        body: vec![Node::Throw { tag: 0, arg: None }],
                        catches: vec![],
                        catch_all: None,
                        delegate: Some(0),
                    }],
                )),
            )],
            0,
        );
        let ir = lower_function(&module, 0, module.body_at(0)).expect("lower");
        let body = ir.expect("body");

        // No catch-dispatch machinery: ExcTagGet and ExcClear are absent.
        assert_eq!(count_inst(&body, |i| matches!(i, Inst::ExcTagGet)), 0);
        assert_eq!(count_inst(&body, |i| matches!(i, Inst::ExcClear)), 0);
        // But the throw still fires.
        assert_eq!(
            count_inst(&body, |i| matches!(
                i,
                Inst::ExcSet { tag_index: 0 }
            )),
            1
        );
    }

    #[test]
    fn lower_rethrow_nonzero_depth_errors() {
        let module = mk_module_with_tag(
            vec![mk_sig(&[], &[])],
            vec![(
                mk_sig(&[], &[]),
                Some(AstFuncBody::new(
                    vec![],
                    vec![Node::Try {
                        body: vec![],
                        catches: vec![Catch {
                            tag_index: 0,
                            body: vec![Node::Rethrow(1)],
                        }],
                        catch_all: None,
                        delegate: None,
                    }],
                )),
            )],
            0,
        );
        let err = lower_function(&module, 0, module.body_at(0))
            .expect_err("expected unsupported depth error");
        assert!(
            format!("{err}").contains("rethrow"),
            "error should mention rethrow: {err}"
        );
    }
}
