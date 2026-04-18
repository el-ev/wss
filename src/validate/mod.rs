use anyhow::{Context, bail};
use wasmparser::{BlockType, Operator, Parser, Payload::*, RefType, ValType};

use crate::module::{FuncType, ModuleInfo};

mod operators;

use operators::validate_operator;

pub fn validate(module: &ModuleInfo, wasm_bytes: &[u8]) -> anyhow::Result<()> {
    validate_imports_exports(module)?;
    validate_types(module)?;
    validate_globals(module)?;
    validate_tables(module)?;
    validate_code_section(wasm_bytes, module)?;
    Ok(())
}

fn validate_globals(module: &ModuleInfo) -> anyhow::Result<()> {
    module
        .globals()
        .iter()
        .enumerate()
        .try_for_each(|(i, g)| validate_valtype(g.content_type(), &format!("global[{i}]")))
}

fn validate_tables(module: &ModuleInfo) -> anyhow::Result<()> {
    module
        .tables()
        .iter()
        .enumerate()
        .try_for_each(|(i, table)| {
            if table.element_type() != RefType::FUNCREF {
                bail!(
                    "table[{i}] has unsupported element type {:?} (expected funcref)",
                    table.element_type()
                );
            }
            if table.entries().len() > 256 {
                bail!(
                    "table[{i}] has {} entries, but maximum supported size is 256",
                    table.entries().len()
                );
            }
            Ok(())
        })
}

fn validate_imports_exports(module: &ModuleInfo) -> anyhow::Result<()> {
    if module.num_imported_funcs() > 2 {
        bail!(
            "expected at most 2 imported functions (getchar, putchar), got {}",
            module.num_imported_funcs()
        );
    }
    if let Some(idx) = module.putchar_import() {
        let f = func_signature(module, idx)?;
        // TODO(i64): imported runtime ABI is currently fixed to i32 signatures.
        if f.params() != [ValType::I32] || f.results() != [ValType::I32] {
            bail!("putchar must have signature (i32) -> i32");
        }
    }
    if let Some(idx) = module.getchar_import() {
        let f = func_signature(module, idx)?;
        // TODO(i64): imported runtime ABI is currently fixed to i32 signatures.
        if !f.params().is_empty() || f.results() != [ValType::I32] {
            bail!("getchar must have signature () -> i32");
        }
    }
    let entry_idx = module
        .entry_export()
        .context("module must export '_start'")?;
    let entry_func = func_signature(module, entry_idx)?;
    // TODO(i64): top-level ABI currently requires `_start` to return i32.
    if entry_func.results() != [ValType::I32] {
        bail!("exported '_start' must return i32");
    }
    Ok(())
}

fn validate_types(module: &ModuleInfo) -> anyhow::Result<()> {
    module.types().iter().enumerate().try_for_each(|(i, ty)| {
        ty.params()
            .iter()
            .enumerate()
            .try_for_each(|(j, v)| validate_valtype(*v, &format!("type[{i}].param[{j}]")))?;
        if ty.results().len() > 1 {
            bail!("multi-value returns not supported (type[{i}])");
        }
        ty.results()
            .iter()
            .enumerate()
            .try_for_each(|(j, v)| validate_valtype(*v, &format!("type[{i}].result[{j}]")))?;
        Ok(())
    })
}

fn validate_valtype(v: ValType, location: &str) -> anyhow::Result<()> {
    match v {
        // TODO(i64): validator accepts only i32 value types today.
        ValType::I32 => Ok(()),
        _ => bail!("unsupported value type {:?} at {}", v, location),
    }
}

fn validate_block_type(
    blockty: BlockType,
    module: &ModuleInfo,
    location: &str,
) -> anyhow::Result<()> {
    match blockty {
        BlockType::Empty => Ok(()),
        BlockType::Type(v) => validate_valtype(v, location),
        BlockType::FuncType(type_index) => {
            let ty = module.type_at(type_index).with_context(|| {
                format!(
                    "block type index {} out of bounds at {}",
                    type_index, location
                )
            })?;
            for &v in ty.results() {
                validate_valtype(v, location)?;
            }
            Ok(())
        }
    }
}

fn validate_code_section(wasm_bytes: &[u8], module: &ModuleInfo) -> anyhow::Result<()> {
    let parser = Parser::new(0);
    let mut code_index = 0usize;
    for payload in parser.parse_all(wasm_bytes) {
        let payload = payload.context("WASM parse")?;
        if let CodeSectionEntry(body) = &payload {
            let func_index = module.num_imported_funcs() + code_index;
            let sig = func_signature(module, func_index as u32)?;
            let loc = format!("function {}", func_index);
            for v in sig.params() {
                validate_valtype(*v, &format!("{} param", loc))?;
            }
            for v in sig.results() {
                validate_valtype(*v, &format!("{} result", loc))?;
            }
            let mut locals_reader = body.get_locals_reader().with_context(|| loc.clone())?;
            for _ in 0..locals_reader.get_count() {
                let (_, val_type) = locals_reader.read().with_context(|| loc.clone())?;
                validate_valtype(val_type, &format!("{} local", loc))?;
            }
            let mut ops_reader = body.get_operators_reader().with_context(|| loc.clone())?;
            while !ops_reader.eof() {
                let op = ops_reader.read().with_context(|| loc.clone())?;
                validate_operator(&op, module, &loc)?;
            }
            code_index += 1;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::validate;
    use crate::module::decode_module_info;

    #[test]
    fn validate_scans_ops_after_nested_end() {
        let wasm_bytes = [
            0x00, 0x61, 0x73, 0x6d, // magic
            0x01, 0x00, 0x00, 0x00, // version
            0x01, 0x05, 0x01, 0x60, 0x00, 0x01, 0x7f, // type section: [] -> [i32]
            0x03, 0x02, 0x01, 0x00, // function section
            0x07, 0x0a, 0x01, 0x06, 0x5f, 0x73, 0x74, 0x61, 0x72, 0x74, 0x00,
            0x00, // export _start
            0x0a, 0x0c, 0x01, 0x0a, 0x00, 0x02, 0x40, 0x0b, 0x42, 0x00, 0x1a, 0x41,
            0x00, 0x0b, // code: block/end, then invalid i64.const
        ];

        let module = decode_module_info(&wasm_bytes).expect("module should decode");
        let err = validate(&module, &wasm_bytes).expect_err("nested trailing i64 op should fail");
        let msg = format!("{err:#}");
        assert!(
            msg.contains("i64 not supported"),
            "expected i64 validation failure, got: {msg}"
        );
    }
}

fn func_signature(module: &ModuleInfo, func_index: u32) -> anyhow::Result<&FuncType> {
    module
        .func_type_at(func_index)
        .with_context(|| format!("function index {} out of bounds", func_index))
}
