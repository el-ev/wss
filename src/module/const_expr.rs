use anyhow::Context;
use wasmparser::Operator;

use crate::ast::Node;

pub(super) fn parse_const_expr(expr: wasmparser::ConstExpr<'_>) -> anyhow::Result<Node> {
    let mut reader = expr.get_operators_reader();
    let op = reader.read().context("const expr")?;
    let val = match op {
        // TODO(i64): const-init parsing only accepts i32 const expressions today.
        Operator::I32Const { value } => Node::I32Const(value),
        other => anyhow::bail!("unsupported const init operator: {:?}", other),
    };
    let end = reader.read().context("const expr end")?;
    if !matches!(end, Operator::End) {
        anyhow::bail!("expected End after const init, got {:?}", end);
    }
    Ok(val)
}

pub(super) fn parse_ref_const_expr(expr: wasmparser::ConstExpr<'_>) -> anyhow::Result<Option<u32>> {
    let mut reader = expr.get_operators_reader();
    let op = reader.read().context("const expr")?;
    let val = match op {
        Operator::RefNull { .. } => None,
        Operator::RefFunc { function_index } => Some(function_index),
        other => anyhow::bail!("unsupported const ref operator: {:?}", other),
    };
    let end = reader.read().context("const expr end")?;
    if !matches!(end, Operator::End) {
        anyhow::bail!("expected End after const ref init, got {:?}", end);
    }
    Ok(val)
}
