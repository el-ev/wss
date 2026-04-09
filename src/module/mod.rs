use anyhow::{Context, bail};
use wasmparser::{
    CompositeInnerType, DataKind, ElementItems, ElementKind, ExternalKind, Parser, Payload::*,
    RefType, TableInit, TypeRef, ValType,
};

use crate::ast::Node;
use crate::ir::{BasicBlock, BlockId};

mod const_expr;
mod helpers;

use const_expr::{parse_const_expr, parse_ref_const_expr};
use helpers::{build_table_info, clone_func_type, is_entry_export_name};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ConstInit {
    I32(i32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RefConstInit {
    Null,
    Func(u32),
}

#[derive(Clone, Debug)]
pub(crate) struct GlobalInfo {
    content_type: ValType,
    init: ConstInit,
}

impl GlobalInfo {
    pub(crate) fn new(content_type: ValType, init: ConstInit) -> Self {
        Self { content_type, init }
    }

    pub(crate) fn content_type(&self) -> ValType {
        self.content_type
    }

    pub(crate) fn init(&self) -> ConstInit {
        self.init
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TableInfo {
    element_type: RefType,
    entries: Vec<Option<u32>>,
}

impl TableInfo {
    pub(crate) fn new(element_type: RefType, entries: Vec<Option<u32>>) -> Self {
        Self {
            element_type,
            entries,
        }
    }

    pub(crate) fn element_type(&self) -> RefType {
        self.element_type
    }

    pub(crate) fn entries(&self) -> &[Option<u32>] {
        &self.entries
    }

    pub(crate) fn set_entry(&mut self, entry_index: usize, value: Option<u32>) {
        self.entries[entry_index] = value;
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ModuleInfo {
    types: Vec<FuncType>,
    functions: Vec<FuncType>,
    globals: Vec<GlobalInfo>,
    tables: Vec<TableInfo>,
    num_pages: u64,
    preloaded_data: Vec<(usize, Vec<u8>)>,
    num_imported_funcs: usize,
    putchar_import: Option<u32>,
    getchar_import: Option<u32>,
    entry_export: Option<u32>,
}

#[derive(Clone, Debug)]
pub(crate) struct FuncType {
    params: Box<[ValType]>,
    results: Box<[ValType]>,
}

impl FuncType {
    pub(crate) fn new(params: Box<[ValType]>, results: Box<[ValType]>) -> Self {
        Self { params, results }
    }

    pub(crate) fn params(&self) -> &[ValType] {
        &self.params
    }

    pub(crate) fn results(&self) -> &[ValType] {
        &self.results
    }
}

fn parse_i32_const_offset(init: ConstInit, nonnegative_label: &str) -> anyhow::Result<usize> {
    match init {
        // TODO(i64): element/data offsets are parsed from i32 consts only.
        ConstInit::I32(v) => {
            usize::try_from(v).with_context(|| format!("{nonnegative_label} must be >= 0, got {v}"))
        }
    }
}

fn ensure_function_exists(func_index: u32, func_count: usize, context: &str) -> anyhow::Result<()> {
    if func_index as usize >= func_count {
        bail!(
            "{context} references function {} but module only has {} functions",
            func_index,
            func_count
        );
    }
    Ok(())
}

impl ModuleInfo {
    pub(crate) fn types(&self) -> &[FuncType] {
        &self.types
    }

    pub(crate) fn functions(&self) -> &[FuncType] {
        &self.functions
    }

    pub(crate) fn globals(&self) -> &[GlobalInfo] {
        &self.globals
    }

    pub(crate) fn tables(&self) -> &[TableInfo] {
        &self.tables
    }

    pub(crate) fn num_pages(&self) -> u64 {
        self.num_pages
    }

    pub(crate) fn set_num_pages(&mut self, num_pages: u64) {
        self.num_pages = num_pages;
    }

    pub(crate) fn preloaded_data(&self) -> &[(usize, Vec<u8>)] {
        &self.preloaded_data
    }

    pub(crate) fn num_imported_funcs(&self) -> usize {
        self.num_imported_funcs
    }

    pub(crate) fn set_num_imported_funcs(&mut self, num_imported_funcs: usize) {
        self.num_imported_funcs = num_imported_funcs;
    }

    pub(crate) fn putchar_import(&self) -> Option<u32> {
        self.putchar_import
    }

    pub(crate) fn set_putchar_import(&mut self, putchar_import: Option<u32>) {
        self.putchar_import = putchar_import;
    }

    pub(crate) fn getchar_import(&self) -> Option<u32> {
        self.getchar_import
    }

    pub(crate) fn entry_export(&self) -> Option<u32> {
        self.entry_export
    }

    pub(crate) fn set_entry_export(&mut self, entry_export: Option<u32>) {
        self.entry_export = entry_export;
    }

    pub(crate) fn type_at(&self, type_index: u32) -> Option<&FuncType> {
        self.types.get(type_index as usize)
    }

    pub(crate) fn func_type_at(&self, func_index: u32) -> Option<&FuncType> {
        self.functions.get(func_index as usize)
    }

    pub(crate) fn table_at(&self, table_index: u32) -> Option<&TableInfo> {
        self.tables.get(table_index as usize)
    }

    pub(crate) fn table_mut_at(&mut self, table_index: u32) -> Option<&mut TableInfo> {
        self.tables.get_mut(table_index as usize)
    }
}

#[cfg(test)]
impl ModuleInfo {
    pub(crate) fn types_mut(&mut self) -> &mut Vec<FuncType> {
        &mut self.types
    }

    pub(crate) fn functions_mut(&mut self) -> &mut Vec<FuncType> {
        &mut self.functions
    }

    pub(crate) fn globals_mut(&mut self) -> &mut Vec<GlobalInfo> {
        &mut self.globals
    }

    pub(crate) fn tables_mut(&mut self) -> &mut Vec<TableInfo> {
        &mut self.tables
    }

    pub(crate) fn preloaded_data_mut(&mut self) -> &mut Vec<(usize, Vec<u8>)> {
        &mut self.preloaded_data
    }
}

pub(crate) fn decode_module_info(wasm_bytes: &[u8]) -> anyhow::Result<ModuleInfo> {
    let parser = Parser::new(0);
    let mut module = ModuleInfo::default();
    let mut num_imports = 0usize;

    for payload in parser.parse_all(wasm_bytes) {
        let payload = payload.context("WASM parse")?;
        match payload {
            TypeSection(s) => {
                for group in s {
                    let group = group.context("type group")?;
                    for sub_ty in group.into_types() {
                        if let CompositeInnerType::Func(ft) = sub_ty.composite_type.inner {
                            module.types.push(FuncType::new(
                                ft.params().to_vec().into_boxed_slice(),
                                ft.results().to_vec().into_boxed_slice(),
                            ));
                        }
                    }
                }
            }
            ImportSection(s) => {
                for import in s.into_imports() {
                    let import = import.context("import")?;
                    match import.ty {
                        TypeRef::Func(type_index) => {
                            let ty = clone_func_type(&module, type_index)?;
                            module.functions.push(ty);
                            if import.name == "putchar" {
                                module.set_putchar_import(Some(num_imports as u32));
                            }
                            if import.name == "getchar" {
                                module.getchar_import = Some(num_imports as u32);
                            }
                            num_imports += 1;
                        }
                        TypeRef::Table(_) => {
                            bail!("imported tables are not supported");
                        }
                        _ => {}
                    }
                }
            }
            FunctionSection(s) => {
                for func in s {
                    let type_index = func.context("function")?;
                    let ty = clone_func_type(&module, type_index)?;
                    module.functions.push(ty);
                }
            }
            TableSection(s) => {
                for table in s {
                    let table = table.context("table")?;
                    let init = match table.init {
                        TableInit::RefNull => None,
                        TableInit::Expr(expr) => match parse_ref_const_expr(expr)
                            .context("table init expression must be ref.null or ref.func")?
                        {
                            RefConstInit::Null => None,
                            RefConstInit::Func(function_index) => Some(function_index),
                        },
                    };
                    if let Some(func_index) = init {
                        ensure_function_exists(func_index, module.functions.len(), "table init")?;
                    }
                    module
                        .tables
                        .push(build_table_info(table.ty, init).context("table section")?);
                }
            }
            ExportSection(s) => {
                for export in s {
                    let export = export.context("export")?;
                    if is_entry_export_name(export.name) && export.kind == ExternalKind::Func {
                        module.set_entry_export(Some(export.index));
                    }
                }
            }
            GlobalSection(s) => {
                for global in s {
                    let global = global.context("global")?;
                    module.globals.push(GlobalInfo::new(
                        global.ty.content_type,
                        parse_const_expr(global.init_expr)?,
                    ));
                }
            }
            MemorySection(s) => {
                for memory in s {
                    let memory = memory.context("memory")?;
                    module.set_num_pages(memory.initial);
                }
            }
            DataSection(s) => {
                for data in s {
                    let data = data.context("data")?;
                    let bytes = data.data.to_vec();
                    match data.kind {
                        DataKind::Active {
                            memory_index,
                            offset_expr,
                        } => {
                            if memory_index != 0 {
                                bail!("only memory index 0 is supported for active data segments");
                            }
                            let offset = parse_const_expr(offset_expr)
                                .context("active data segment offset expr")?;
                            let offset =
                                parse_i32_const_offset(offset, "active data segment offset")?;
                            module.preloaded_data.push((offset, bytes));
                        }
                        DataKind::Passive => {
                            bail!("passive data segments are not supported");
                        }
                    }
                }
            }
            ElementSection(s) => {
                for element in s {
                    let element = element.context("element")?;
                    let (table_index, offset_expr) = match element.kind {
                        ElementKind::Active {
                            table_index,
                            offset_expr,
                        } => (table_index.unwrap_or(0), offset_expr),
                        ElementKind::Passive => {
                            bail!("passive element segments are not supported");
                        }
                        ElementKind::Declared => {
                            bail!("declared element segments are not supported");
                        }
                    };
                    let table_len = module
                        .table_at(table_index)
                        .with_context(|| {
                            format!("element segment targets missing table {}", table_index)
                        })?
                        .entries()
                        .len();
                    let offset =
                        parse_const_expr(offset_expr).context("element segment offset expr")?;
                    let offset = parse_i32_const_offset(offset, "element segment offset")?;

                    let mut updates = Vec::new();
                    match element.items {
                        ElementItems::Functions(functions) => {
                            for (i, func) in functions.into_iter().enumerate() {
                                updates.push((offset + i, Some(func.context("element function")?)));
                            }
                        }
                        ElementItems::Expressions(ref_ty, exprs) => {
                            if ref_ty != RefType::FUNCREF {
                                bail!("unsupported element expression ref type {:?}", ref_ty);
                            }
                            for (i, expr) in exprs.into_iter().enumerate() {
                                let value =
                                    match parse_ref_const_expr(expr.context("element expr")?)
                                        .context("unsupported element expression")?
                                    {
                                        RefConstInit::Null => None,
                                        RefConstInit::Func(function_index) => Some(function_index),
                                    };
                                updates.push((offset + i, value));
                            }
                        }
                    }

                    let func_count = module.functions.len();
                    let table = module.table_mut_at(table_index).with_context(|| {
                        format!(
                            "element segment targets missing mutable table {}",
                            table_index
                        )
                    })?;
                    for (entry_index, value) in updates {
                        if entry_index >= table_len {
                            bail!(
                                "element segment writes table[{}] index {} but table length is {}",
                                table_index,
                                entry_index,
                                table_len
                            );
                        }
                        if let Some(func_index) = value {
                            ensure_function_exists(func_index, func_count, "element segment")?;
                        }
                        table.set_entry(entry_index, value);
                    }
                }
            }
            _ => {}
        }
    }

    module.set_num_imported_funcs(num_imports);
    Ok(module)
}

#[derive(Clone, Debug)]
pub(crate) struct AstModule {
    info: ModuleInfo,
    bodies: Vec<Option<AstFuncBody>>,
}

impl AstModule {
    pub(crate) fn new(info: ModuleInfo, bodies: Vec<Option<AstFuncBody>>) -> Self {
        Self { info, bodies }
    }

    pub(crate) fn info(&self) -> &ModuleInfo {
        &self.info
    }

    pub(crate) fn into_info(self) -> ModuleInfo {
        self.info
    }

    #[cfg(test)]
    pub(crate) fn body_at(&self, func_index: u32) -> Option<&AstFuncBody> {
        self.bodies.get(func_index as usize)?.as_ref()
    }

    pub(crate) fn set_body(
        &mut self,
        func_index: u32,
        body: Option<AstFuncBody>,
    ) -> anyhow::Result<()> {
        let slot = self
            .bodies
            .get_mut(func_index as usize)
            .with_context(|| format!("invalid function index {}", func_index))?;
        if slot.is_some() {
            bail!("function index {} already has a body", func_index);
        }
        *slot = body;
        Ok(())
    }

    pub(crate) fn bodies(&self) -> &[Option<AstFuncBody>] {
        &self.bodies
    }

    pub(crate) fn type_at(&self, type_index: u32) -> Option<&FuncType> {
        self.info.type_at(type_index)
    }

    pub(crate) fn func_type_at(&self, func_index: u32) -> Option<&FuncType> {
        self.info.func_type_at(func_index)
    }

    pub(crate) fn globals(&self) -> &[GlobalInfo] {
        self.info.globals()
    }

    pub(crate) fn num_imported_funcs(&self) -> usize {
        self.info.num_imported_funcs()
    }

    pub(crate) fn putchar_import(&self) -> Option<u32> {
        self.info.putchar_import()
    }

    pub(crate) fn getchar_import(&self) -> Option<u32> {
        self.info.getchar_import()
    }

    pub(crate) fn entry_export(&self) -> Option<u32> {
        self.info.entry_export()
    }
}

impl AstModule {
    pub(crate) fn bodies_mut(&mut self) -> &mut Vec<Option<AstFuncBody>> {
        &mut self.bodies
    }
}

#[derive(Clone, Debug)]
pub(crate) struct AstFuncBody {
    locals: Vec<ValType>,
    insts: Vec<Node>,
}

impl AstFuncBody {
    pub(crate) fn new(locals: Vec<ValType>, insts: Vec<Node>) -> Self {
        Self { locals, insts }
    }

    pub(crate) fn locals(&self) -> &[ValType] {
        &self.locals
    }

    pub(crate) fn insts(&self) -> &[Node] {
        &self.insts
    }
}

#[derive(Clone, Debug)]
pub(crate) struct IrModule {
    info: ModuleInfo,
    bodies: Vec<Option<IrFuncBody>>,
}

impl IrModule {
    pub(crate) fn new(info: ModuleInfo, bodies: Vec<Option<IrFuncBody>>) -> Self {
        Self { info, bodies }
    }

    pub(crate) fn body_at(&self, func_index: u32) -> Option<&IrFuncBody> {
        self.bodies.get(func_index as usize)?.as_ref()
    }

    pub(crate) fn bodies(&self) -> &[Option<IrFuncBody>] {
        &self.bodies
    }

    pub(crate) fn type_at(&self, type_index: u32) -> Option<&FuncType> {
        self.info.type_at(type_index)
    }

    pub(crate) fn func_type_at(&self, func_index: u32) -> Option<&FuncType> {
        self.info.func_type_at(func_index)
    }

    pub(crate) fn globals(&self) -> &[GlobalInfo] {
        self.info.globals()
    }

    pub(crate) fn table_at(&self, table_index: u32) -> Option<&TableInfo> {
        self.info.table_at(table_index)
    }

    pub(crate) fn num_pages(&self) -> u64 {
        self.info.num_pages()
    }

    pub(crate) fn preloaded_data(&self) -> &[(usize, Vec<u8>)] {
        self.info.preloaded_data()
    }

    pub(crate) fn putchar_import(&self) -> Option<u32> {
        self.info.putchar_import()
    }

    pub(crate) fn getchar_import(&self) -> Option<u32> {
        self.info.getchar_import()
    }

    pub(crate) fn entry_export(&self) -> Option<u32> {
        self.info.entry_export()
    }
}

#[cfg(test)]
impl IrModule {
    pub(crate) fn set_num_pages(&mut self, num_pages: u64) {
        self.info.set_num_pages(num_pages);
    }

    pub(crate) fn set_entry_export(&mut self, entry_export: Option<u32>) {
        self.info.set_entry_export(entry_export);
    }

    pub(crate) fn set_num_imported_funcs(&mut self, num_imported_funcs: usize) {
        self.info.set_num_imported_funcs(num_imported_funcs);
    }

    pub(crate) fn set_putchar_import(&mut self, putchar_import: Option<u32>) {
        self.info.set_putchar_import(putchar_import);
    }

    pub(crate) fn types_mut(&mut self) -> &mut Vec<FuncType> {
        self.info.types_mut()
    }

    pub(crate) fn functions_mut(&mut self) -> &mut Vec<FuncType> {
        self.info.functions_mut()
    }

    pub(crate) fn globals_mut(&mut self) -> &mut Vec<GlobalInfo> {
        self.info.globals_mut()
    }

    pub(crate) fn tables_mut(&mut self) -> &mut Vec<TableInfo> {
        self.info.tables_mut()
    }

    pub(crate) fn preloaded_data_mut(&mut self) -> &mut Vec<(usize, Vec<u8>)> {
        self.info.preloaded_data_mut()
    }

    pub(crate) fn bodies_mut(&mut self) -> &mut Vec<Option<IrFuncBody>> {
        &mut self.bodies
    }
}

#[derive(Clone, Debug)]
pub(crate) struct IrFuncBody {
    locals: Vec<ValType>,
    entry: BlockId,
    blocks: Vec<BasicBlock>,
}

impl IrFuncBody {
    pub(crate) fn new(locals: Vec<ValType>, entry: BlockId, blocks: Vec<BasicBlock>) -> Self {
        Self {
            locals,
            entry,
            blocks,
        }
    }

    pub(crate) fn locals(&self) -> &[ValType] {
        &self.locals
    }

    pub(crate) fn entry(&self) -> BlockId {
        self.entry
    }

    pub(crate) fn blocks(&self) -> &[BasicBlock] {
        &self.blocks
    }
}
