use crate::constants::{
    DEFAULT_CALLSTACK_SLOTS_CAP, DEFAULT_MAX_PHYS_REGS, DEFAULT_MEMORY_BYTES_CAP,
};
use crate::emit::{EmitConfig, emit_program};
use crate::lower8::{Lower8Config, lower8_module, lower8_module_with_config};
use crate::module::decode_module_info;
use crate::print::{print_ir8_program, print_module_ast, print_module_ir, print_program};
use crate::regalloc::regalloc;
use crate::schedule::schedule;
use crate::validate::validate;
use crate::{lower::lower_module, parse::parse_module};
use anyhow::{Context, Result};
use clap::{ArgAction, Parser};
use std::path::PathBuf;

mod ast;
mod constants;
mod emit;
mod ir;
mod ir8;
mod lower;
mod lower8;
mod module;
mod opt8;
mod parse;
mod print;
mod regalloc;
mod schedule;
mod validate;

#[derive(Debug, Clone, Copy, Default)]
struct DumpConfig {
    ast: bool,
    ir: bool,
    ir8: bool,
    ir8_opt: bool,
    program: bool,
}

#[derive(Debug, Parser)]
#[command(name = "wss")]
#[command(about = "Transpile WebAssembly into an HTML/CSS runtime")]
struct Cli {
    /// Input WebAssembly file.
    wasm_file: PathBuf,
    /// Output HTML path.
    #[arg(short, long, default_value = "a.html")]
    output: PathBuf,
    /// Runtime linear memory cap in bytes (max: 65536, 0 = use global 0 SP).
    #[arg(long = "memory-bytes", default_value_t = DEFAULT_MEMORY_BYTES_CAP)]
    memory_bytes: u32,
    /// Runtime callstack cap in 16-bit slots.
    #[arg(long = "stack-slots", default_value_t = DEFAULT_CALLSTACK_SLOTS_CAP)]
    stack_slots: usize,
    /// Enable JS-based clock stepping.
    #[arg(
        long = "js-clock",
        action = ArgAction::SetTrue,
        conflicts_with = "no_js_clock"
    )]
    js_clock: bool,
    /// Disable JS-based clock stepping.
    #[arg(
        long = "no-js-clock",
        action = ArgAction::SetTrue,
        conflicts_with = "js_clock"
    )]
    no_js_clock: bool,
    /// Enable JS coprocessor for div/rem and bitwise builtins.
    #[arg(
        long = "js-coprocessor",
        action = ArgAction::SetTrue,
        conflicts_with = "no_js_clock"
    )]
    js_coprocessor: bool,
    /// Enable JS clock debugger popup with speed controls and step execution.
    #[arg(
        long = "js-clock-debugger",
        action = ArgAction::SetTrue,
        conflicts_with = "no_js_clock"
    )]
    js_clock_debugger: bool,
    /// Max total physical regs (including reserved r0-r3) after register allocation.
    #[arg(long = "max-phys-regs", default_value_t = DEFAULT_MAX_PHYS_REGS)]
    max_phys_regs: u16,
    /// Dump all compiler stages to stdout.
    #[arg(long, action = ArgAction::SetTrue)]
    dump_all: bool,
    /// Dump parsed AST to stdout.
    #[arg(long, action = ArgAction::SetTrue)]
    dump_ast: bool,
    /// Dump lowered IR to stdout.
    #[arg(long, action = ArgAction::SetTrue)]
    dump_ir: bool,
    /// Dump pre-optimization IR8 to stdout.
    #[arg(long, action = ArgAction::SetTrue)]
    dump_ir8: bool,
    /// Dump post-optimization IR8 to stdout.
    #[arg(long, action = ArgAction::SetTrue)]
    dump_ir8_opt: bool,
    /// Dump scheduled program to stdout.
    #[arg(long, action = ArgAction::SetTrue)]
    dump_program: bool,
}

impl Cli {
    fn js_clock_enabled(&self) -> bool {
        !self.no_js_clock
    }

    fn dump_config(&self) -> DumpConfig {
        DumpConfig {
            ast: self.dump_all || self.dump_ast,
            ir: self.dump_all || self.dump_ir,
            ir8: self.dump_all || self.dump_ir8,
            ir8_opt: self.dump_all || self.dump_ir8_opt,
            program: self.dump_all || self.dump_program,
        }
    }
}

fn main() -> Result<()> {
    let args = Cli::parse();
    let js_clock = args.js_clock_enabled();
    let output_file = args.output.clone();
    let dump = args.dump_config();
    let emit_config = EmitConfig::new(
        args.memory_bytes,
        args.stack_slots,
        js_clock,
        args.js_coprocessor,
        args.js_clock_debugger,
    )?;

    let wasm_bytes = std::fs::read(&args.wasm_file).with_context(|| {
        format!(
            "failed to read WebAssembly file '{}'",
            args.wasm_file.display()
        )
    })?;

    let module_info = decode_module_info(&wasm_bytes)?;
    validate(&module_info, &wasm_bytes)?;

    let module = parse_module(module_info, &wasm_bytes)?;

    if dump.ast {
        println!("\n--- AST ---\n");
        print!("{}", print_module_ast(&module));
    }

    let module = lower_module(module)?;

    if dump.ir {
        println!("\n--- IR ---\n");
        print!("{}", print_module_ir(&module));
    }

    let mut ir8 = if args.js_coprocessor {
        lower8_module_with_config(
            &module,
            args.memory_bytes,
            Lower8Config {
                js_coprocessor: true,
            },
        )?
    } else {
        lower8_module(&module, args.memory_bytes)?
    };

    if dump.ir8 {
        println!("\n--- IR8 ---\n");
        print!("{}", print_ir8_program(&ir8));
    }

    opt8::run(&mut ir8);

    if dump.ir8_opt {
        println!("\n--- IR8 (opt) ---\n");
        print!("{}", print_ir8_program(&ir8));
    }

    let mut ir8 = regalloc(ir8, args.max_phys_regs)?;
    schedule(&mut ir8)?;

    if dump.program {
        println!("\n--- Program ---\n");
        print!("{}", print_program(&ir8));
    }

    let result = emit_program(&ir8, emit_config)?;
    std::fs::write(&output_file, result)
        .with_context(|| format!("failed to write output HTML '{}'", output_file.display()))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    #[test]
    fn dump_all_enables_all_dump_stages() {
        let args = Cli::try_parse_from(["wss", "input.wasm", "--dump-all"]).unwrap();
        let dump = args.dump_config();

        assert!(dump.ast);
        assert!(dump.ir);
        assert!(dump.ir8);
        assert!(dump.ir8_opt);
        assert!(dump.program);
    }

    #[test]
    fn individual_dump_flags_only_enable_requested_stages() {
        let args =
            Cli::try_parse_from(["wss", "input.wasm", "--dump-ast", "--dump-program"]).unwrap();
        let dump = args.dump_config();

        assert!(dump.ast);
        assert!(!dump.ir);
        assert!(!dump.ir8);
        assert!(!dump.ir8_opt);
        assert!(dump.program);
    }

    #[test]
    fn js_clock_defaults_to_enabled() {
        let args = Cli::try_parse_from(["wss", "input.wasm"]).unwrap();

        assert!(args.js_clock_enabled());
    }

    #[test]
    fn no_js_clock_disables_js_clock() {
        let args = Cli::try_parse_from(["wss", "input.wasm", "--no-js-clock"]).unwrap();

        assert!(!args.js_clock_enabled());
    }

    #[test]
    fn js_coprocessor_conflicts_with_no_js_clock() {
        let err = Cli::try_parse_from(["wss", "input.wasm", "--no-js-clock", "--js-coprocessor"])
            .expect_err("parse should fail");

        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn js_clock_debugger_conflicts_with_no_js_clock() {
        let err =
            Cli::try_parse_from(["wss", "input.wasm", "--no-js-clock", "--js-clock-debugger"])
                .expect_err("parse should fail");

        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn js_clock_conflicts_with_no_js_clock() {
        let err = Cli::try_parse_from(["wss", "input.wasm", "--js-clock", "--no-js-clock"])
            .expect_err("parse should fail");

        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }
}
