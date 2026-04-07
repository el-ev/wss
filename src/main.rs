use crate::constants::{
    DEFAULT_CALLSTACK_SLOTS_CAP, DEFAULT_MAX_PHYS_REGS, DEFAULT_MEMORY_BYTES_CAP,
};
use crate::emit::{EmitConfig, emit_program};
use crate::lower8::{Lower8Config, lower8_module, lower8_module_with_config};
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
    /// Enable JS-based clock stepping (`true` or `false`).
    #[arg(long = "js-clock", default_value_t = true, action = ArgAction::Set)]
    js_clock: bool,
    /// Enable JS coprocessor for div/rem and bitwise builtins (`true` or `false`).
    #[arg(
        long = "js-coprocessor",
        default_value_t = false,
        action = ArgAction::Set
    )]
    js_coprocessor: bool,
    /// Enable JS clock debugger popup with speed controls and step execution (`true` or `false`).
    #[arg(
        long = "js-clock-debugger",
        default_value_t = false,
        action = ArgAction::Set
    )]
    js_clock_debugger: bool,
    /// Max total physical regs (including reserved r0-r3) after register allocation.
    #[arg(long = "max-phys-regs", default_value_t = DEFAULT_MAX_PHYS_REGS)]
    max_phys_regs: u16,
}

fn main() -> Result<()> {
    let args = Cli::parse();
    let output_file = args.output.clone();
    let emit_config = EmitConfig {
        memory_bytes_cap: args.memory_bytes,
        callstack_slots_cap: args.stack_slots,
        js_clock: args.js_clock,
        js_coprocessor: args.js_coprocessor,
        js_clock_debugger: args.js_clock_debugger,
    };
    anyhow::ensure!(
        !args.js_coprocessor || args.js_clock,
        "--js-coprocessor requires --js-clock true"
    );
    anyhow::ensure!(
        !args.js_clock_debugger || args.js_clock,
        "--js-clock-debugger requires --js-clock true"
    );
    let dump = std::env::var_os("WSS_DUMP").is_some();

    let wasm_bytes = std::fs::read(&args.wasm_file).with_context(|| {
        format!(
            "failed to read WebAssembly file '{}'",
            args.wasm_file.display()
        )
    })?;

    validate(&wasm_bytes)?;

    let mut module = parse_module(&wasm_bytes)?;

    if dump {
        println!("\n--- AST ---\n");
        print!("{}", print_module_ast(&module));
    }

    lower_module(&mut module)?;

    if dump {
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

    if dump {
        println!("\n--- IR8 ---\n");
        print!("{}", print_ir8_program(&ir8));
    }

    opt8::run(&mut ir8);

    if dump {
        println!("\n--- IR8 (opt) ---\n");
        print!("{}", print_ir8_program(&ir8));
    }

    let mut ir8 = regalloc(ir8, args.max_phys_regs)?;
    schedule(&mut ir8)?;

    if dump {
        println!("\n--- Program ---\n");
        print!("{}", print_program(&ir8));
    }

    let result = emit_program(&ir8, emit_config)?;
    std::fs::write(&output_file, result)
        .with_context(|| format!("failed to write output HTML '{}'", output_file.display()))?;

    Ok(())
}
