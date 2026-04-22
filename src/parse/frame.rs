use anyhow::{Context, bail};
use wasmparser::ValType;

use crate::ast::{AstRef, Catch, Node, TypedAstRef};

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
    pub(super) dummy_refs: Vec<TypedAstRef>,
}

impl BlockFrame {
    pub(super) fn emit(&mut self, inst: Node) -> AstRef {
        let r = AstRef::new(self.insts.len());
        self.insts.push(inst);
        r
    }

    pub(super) fn ensure_dummy(&mut self, ty: ValType) -> TypedAstRef {
        if let Some(existing) = self.dummy_refs.iter().find(|existing| existing.ty == ty) {
            return *existing;
        }
        let value = match ty {
            ValType::I32 => self.emit(Node::I32Const(0)),
            ValType::I64 => self.emit(Node::I64Const(0)),
            _ => unreachable!("unsupported dummy type {:?}", ty),
        };
        let typed = TypedAstRef::new(value, ty);
        self.dummy_refs.push(typed);
        typed
    }

    pub(super) fn pop_ref(
        &mut self,
        ref_stack: &mut Vec<TypedAstRef>,
        expected_ty: ValType,
    ) -> anyhow::Result<TypedAstRef> {
        if self.unreachable {
            Ok(self.ensure_dummy(expected_ty))
        } else {
            let typed = ref_stack.pop().context("stack underflow")?;
            if typed.ty != expected_ty {
                bail!(
                    "type mismatch: expected {:?}, got {:?}",
                    expected_ty,
                    typed.ty
                );
            }
            Ok(typed)
        }
    }

    pub(super) fn pop_ref_or_dummy(
        &mut self,
        ref_stack: &mut Vec<TypedAstRef>,
        expected_ty: ValType,
    ) -> anyhow::Result<TypedAstRef> {
        if self.unreachable || ref_stack.is_empty() {
            Ok(self.ensure_dummy(expected_ty))
        } else {
            let typed = ref_stack
                .pop()
                .context("stack underflow in pop_ref_or_dummy")?;
            if typed.ty != expected_ty {
                bail!(
                    "type mismatch: expected {:?}, got {:?}",
                    expected_ty,
                    typed.ty
                );
            }
            Ok(typed)
        }
    }
}

pub(super) fn materialize_ref_stack(
    ref_stack: &mut Vec<TypedAstRef>,
    block_stack: &mut [BlockFrame],
    locals: &mut Vec<ValType>,
) -> anyhow::Result<Vec<u32>> {
    let refs = std::mem::take(ref_stack);
    let parent = current_frame_mut(block_stack)?;
    let mut temps = Vec::with_capacity(refs.len());
    for typed in refs {
        locals.push(typed.ty);
        let temp = (locals.len() - 1) as u32;
        temps.push(temp);
        parent.emit(Node::LocalSet(temp, typed.value));
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
    ref_stack: &mut Vec<TypedAstRef>,
    param_types: &[ValType],
    op_name: &str,
) -> anyhow::Result<Vec<AstRef>> {
    let arg_count = param_types.len();
    let stack_len = ref_stack.len();
    if stack_len < arg_count {
        bail!(
            "{}: stack has {} value(s) but function expects {} argument(s)",
            op_name,
            stack_len,
            arg_count
        );
    }
    let mut out = Vec::with_capacity(arg_count);
    for (typed, expected_ty) in ref_stack.drain(stack_len - arg_count..).zip(param_types) {
        if typed.ty != *expected_ty {
            bail!(
                "{}: argument type mismatch, expected {:?}, got {:?}",
                op_name,
                expected_ty,
                typed.ty
            );
        }
        out.push(typed.value);
    }
    Ok(out)
}
