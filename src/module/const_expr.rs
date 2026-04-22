use anyhow::Context;
use wasmparser::Operator;

use super::{ConstInit, RefConstInit};

pub(super) fn parse_const_expr(expr: wasmparser::ConstExpr<'_>) -> anyhow::Result<ConstInit> {
    let mut reader = expr.get_operators_reader();
    let op = reader.read().context("const expr")?;
    let val = match op {
        Operator::I32Const { value } => ConstInit::I32(value),
        Operator::I64Const { value } => ConstInit::I64(value),
        other => anyhow::bail!("unsupported const init operator: {:?}", other),
    };
    let end = reader.read().context("const expr end")?;
    if !matches!(end, Operator::End) {
        anyhow::bail!("expected End after const init, got {:?}", end);
    }
    Ok(val)
}

pub(super) fn parse_ref_const_expr(
    expr: wasmparser::ConstExpr<'_>,
) -> anyhow::Result<RefConstInit> {
    let mut reader = expr.get_operators_reader();
    let op = reader.read().context("const expr")?;
    let val = match op {
        Operator::RefNull { .. } => RefConstInit::Null,
        Operator::RefFunc { function_index } => RefConstInit::Func(function_index),
        other => anyhow::bail!("unsupported const ref operator: {:?}", other),
    };
    let end = reader.read().context("const expr end")?;
    if !matches!(end, Operator::End) {
        anyhow::bail!("expected End after const ref init, got {:?}", end);
    }
    Ok(val)
}
