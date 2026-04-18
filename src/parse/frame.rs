use anyhow::{Context, bail};
use wasmparser::ValType;

use crate::ast::{AstRef, Catch, Node};

#[derive(Debug)]
pub(super) enum BlockKind {
    Function,
    Block,
    Loop,
    If {
        cond_ref: AstRef,
    },
    Else {
        cond_ref: AstRef,
        then_insts: Vec<Node>,
    },
    Try,
    TryCatch {
        try_insts: Vec<Node>,
        prior_catches: Vec<Catch>,
        /// Set once `catch_all` has been seen. The currently parsing segment
        /// is the catch_all body while `current_tag` is `None`.
        catch_all_seen: bool,
        /// `Some(tag)` while parsing a specific catch body; `None` while
        /// parsing the catch_all body.
        current_tag: Option<u32>,
    },
}

pub(super) struct BlockFrame {
    pub(super) kind: BlockKind,
    pub(super) return_types: Vec<ValType>,
    pub(super) temp_locals: Vec<u32>,
    pub(super) insts: Vec<Node>,
    pub(super) unreachable: bool,
    pub(super) dummy_ref: Option<AstRef>,
}

impl BlockFrame {
    pub(super) fn emit(&mut self, inst: Node) -> AstRef {
        let r = AstRef::new(self.insts.len());
        self.insts.push(inst);
        r
    }

    pub(super) fn ensure_dummy(&mut self) {
        if self.dummy_ref.is_none() {
            // TODO(i64): unreachable-stack dummy values are currently materialized as i32 consts.
            self.dummy_ref = Some(self.emit(Node::I32Const(0)));
        }
    }

    pub(super) fn pop_ref(&mut self, ref_stack: &mut Vec<AstRef>) -> anyhow::Result<AstRef> {
        if self.unreachable {
            self.ensure_dummy();
            self.dummy_ref
                .context("ice: missing dummy ref in unreachable frame")
        } else {
            ref_stack.pop().context("stack underflow")
        }
    }

    pub(super) fn pop_ref_or_dummy(
        &mut self,
        ref_stack: &mut Vec<AstRef>,
    ) -> anyhow::Result<AstRef> {
        if self.unreachable || ref_stack.is_empty() {
            self.ensure_dummy();
            self.dummy_ref
                .context("ice: missing dummy ref in pop_ref_or_dummy")
        } else {
            ref_stack
                .pop()
                .context("stack underflow in pop_ref_or_dummy")
        }
    }
}

pub(super) fn materialize_ref_stack(
    ref_stack: &mut Vec<AstRef>,
    block_stack: &mut [BlockFrame],
    locals: &mut Vec<ValType>,
) -> anyhow::Result<Vec<u32>> {
    let refs = std::mem::take(ref_stack);
    let parent = current_frame_mut(block_stack)?;
    let mut temps = Vec::with_capacity(refs.len());
    for r in refs {
        // TODO(i64): spilled temporary refs are currently forced to i32 locals.
        locals.push(ValType::I32);
        let temp = (locals.len() - 1) as u32;
        temps.push(temp);
        parent.emit(Node::LocalSet(temp, r));
    }
    Ok(temps)
}

pub(super) fn current_frame_mut(block_stack: &mut [BlockFrame]) -> anyhow::Result<&mut BlockFrame> {
    block_stack.last_mut().context("ice: block stack is empty")
}

pub(super) fn target_block_index(
    block_depth: usize,
    relative_depth: u32,
    op_name: &str,
) -> anyhow::Result<usize> {
    block_depth
        .checked_sub(1 + relative_depth as usize)
        .with_context(|| {
            format!(
                "{}: relative depth {} exceeds block depth",
                op_name, relative_depth
            )
        })
}

pub(super) fn pop_call_args(
    ref_stack: &mut Vec<AstRef>,
    arg_count: usize,
    op_name: &str,
) -> anyhow::Result<Vec<AstRef>> {
    let stack_len = ref_stack.len();
    if stack_len < arg_count {
        bail!(
            "{}: stack has {} value(s) but function expects {} argument(s)",
            op_name,
            stack_len,
            arg_count
        );
    }
    Ok(ref_stack.drain(stack_len - arg_count..).collect())
}
