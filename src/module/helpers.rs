use anyhow::Context;
use wasmparser::TableType;

use super::{FuncType, ModuleInfo, TableInfo};

pub(super) fn clone_func_type(module: &ModuleInfo, type_index: u32) -> anyhow::Result<FuncType> {
    module
        .type_at(type_index)
        .context(format!("type index out of bounds: {}", type_index))
        .cloned()
}

pub(super) fn build_table_info(
    table_ty: TableType,
    init: Option<u32>,
) -> anyhow::Result<TableInfo> {
    anyhow::ensure!(
        !table_ty.table64,
        "table64 is not supported (table initial={})",
        table_ty.initial
    );
    let len = usize::try_from(table_ty.initial).context("table initial size too large")?;
    Ok(TableInfo::new(table_ty.element_type, vec![init; len]))
}

pub(super) fn is_entry_export_name(name: &str) -> bool {
    name == "_start"
}
