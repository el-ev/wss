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
use std::path::{Path, PathBuf};

mod ast;
mod constants;
mod css;
mod dirty;
mod emit;
mod ir;
mod ir8;
mod lower;
mod lower8;
mod module;
mod opt8;
mod page;
mod parse;
mod print;
mod regalloc;
mod schedule;
mod validate;

/// A `--flag[=SEED]` argument: bare flag uses a random seed, `--flag=N` uses that seed.
#[derive(Debug, Clone, Copy)]
struct OptionalSeed(Option<u64>);

impl OptionalSeed {
    fn resolve(self, fallback: u64) -> u64 {
        self.0.unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos() as u64)
                .unwrap_or(fallback)
        })
    }
}

impl std::str::FromStr for OptionalSeed {
    type Err = std::num::ParseIntError;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s.is_empty() {
            Ok(Self(None))
        } else {
            Ok(Self(Some(s.parse()?)))
        }
    }
}

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
    /// Input WebAssembly file (.wasm or .wat).
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
    /// Permute the program counter labels assigned by the scheduler.
    #[arg(
        long = "randomize-pc",
        num_args = 0..=1,
        default_missing_value = "",
        require_equals = true,
        value_name = "SEED",
        conflicts_with_all = ["js_clock_debugger", "sparse_pc"]
    )]
    randomize_pc: Option<OptionalSeed>,
    /// Sample PC labels uniformly from the full 16-bit space.
    #[arg(
        long = "sparse-pc",
        num_args = 0..=1,
        default_missing_value = "",
        require_equals = true,
        value_name = "SEED",
        conflicts_with = "js_clock_debugger"
    )]
    sparse_pc: Option<OptionalSeed>,
    /// Rename CSS custom properties to short identifiers and alphabetise decls.
    #[arg(
        long = "minify-vars",
        num_args = 0..=1,
        default_missing_value = "",
        require_equals = true,
        value_name = "SEED",
        conflicts_with = "js_clock_debugger"
    )]
    minify_vars: Option<OptionalSeed>,
    /// Shuffle mutually exclusive `if()` arms.
    #[arg(
        long = "shuffle-arms",
        num_args = 0..=1,
        default_missing_value = "",
        require_equals = true,
        value_name = "SEED",
        conflicts_with = "js_clock_debugger"
    )]
    shuffle_arms: Option<OptionalSeed>,
    /// Reorder operands of commutative CSS operations.
    #[arg(
        long = "shuffle-ops",
        num_args = 0..=1,
        default_missing_value = "",
        require_equals = true,
        value_name = "SEED",
        conflicts_with = "js_clock_debugger"
    )]
    shuffle_ops: Option<OptionalSeed>,
    /// Permute `@property` and `@function` definitions.
    #[arg(
        long = "shuffle-at-rules",
        num_args = 0..=1,
        default_missing_value = "",
        require_equals = true,
        value_name = "SEED",
        conflicts_with = "js_clock_debugger"
    )]
    shuffle_at_rules: Option<OptionalSeed>,
    /// Add decoy integer fallbacks to `var()` references.
    #[arg(
        long = "decoy-fallbacks",
        num_args = 0..=1,
        default_missing_value = "",
        require_equals = true,
        value_name = "SEED",
        conflicts_with = "js_clock_debugger"
    )]
    decoy_fallbacks: Option<OptionalSeed>,
    /// Inject unreachable decoy arms into LUT `@function` definitions.
    #[arg(
        long = "decoy-arms",
        num_args = 0..=1,
        default_missing_value = "",
        require_equals = true,
        value_name = "SEED",
        conflicts_with = "js_clock_debugger"
    )]
    decoy_arms: Option<OptionalSeed>,
    /// Minify embedded `<script>` blocks.
    #[arg(
        long = "minify-js",
        action = ArgAction::SetTrue,
        conflicts_with = "js_clock_debugger"
    )]
    minify_js: bool,
    /// Split PC-keyed `if()` chains into helper decls (`--__{N}`).
    #[arg(
        long = "split-pc",
        num_args = 0..=1,
        default_missing_value = "",
        require_equals = true,
        value_name = "SEED",
        conflicts_with = "js_clock_debugger"
    )]
    split_pc: Option<OptionalSeed>,
    /// Max total physical regs (including reserved r0-r3) after register allocation.
    #[arg(long = "max-phys-regs", default_value_t = DEFAULT_MAX_PHYS_REGS)]
    max_phys_regs: u16,
    /// Skip embedding the invoking compile command as an HTML comment at the top of the artifact.
    #[arg(long = "no-embed-compile-command", action = ArgAction::SetTrue)]
    no_embed_compile_command: bool,
    /// Disable memory and callstack visualizers in the emitted runtime.
    #[arg(long = "no-visualizers", action = ArgAction::SetTrue)]
    no_visualizers: bool,
    /// Skip the linear-memory bounds check; OOB accesses become silent.
    #[arg(long = "no-memory-trap", action = ArgAction::SetTrue)]
    no_memory_trap: bool,
    /// Skip the callstack-overflow check; pushes past the cap wrap silently.
    #[arg(long = "no-callstack-trap", action = ArgAction::SetTrue)]
    no_callstack_trap: bool,
    /// Drop the PC / SP / G0 indicator panel.
    #[arg(long = "no-indicators", action = ArgAction::SetTrue)]
    no_indicators: bool,
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
        !args.no_visualizers,
        !args.no_memory_trap,
        !args.no_callstack_trap,
        !args.no_indicators,
    )?;

    let wasm_bytes = read_wasm_bytes(&args.wasm_file)?;

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

    let mut seeds_used: Vec<(&'static str, u64)> = Vec::new();
    let mut take = |name: &'static str, slot: Option<OptionalSeed>, fallback: u64| {
        let s = slot?.resolve(fallback);
        seeds_used.push((name, s));
        Some(s)
    };
    if let Some(seed) = take("randomize-pc", args.randomize_pc, 0xDEADBEEF) {
        dirty::randomize::randomize_pcs(&mut ir8, seed)?;
    }
    if let Some(seed) = take("sparse-pc", args.sparse_pc, 0x005C_477E_5EED) {
        dirty::randomize::sparsify_pcs(&mut ir8, seed)?;
    }

    if dump.program {
        println!("\n--- Program ---\n");
        print!("{}", print_program(&ir8));
    }

    let html = emit_program(&ir8, emit_config)?;
    let mut page = page::Page::from_html(&html);
    if let Some(seed) = take("split-pc", args.split_pc, 0x5912_5912) {
        dirty::minify::split_pc_branches(&mut page, seed);
    }
    if let Some(seed) = take("minify-vars", args.minify_vars, 0xC0FFEE) {
        dirty::minify::minify(&mut page, seed);
    }
    if let Some(seed) = take("shuffle-arms", args.shuffle_arms, 0xBADF00D) {
        dirty::minify::shuffle_arms_in_styles(&mut page, seed);
    }
    if let Some(seed) = take("shuffle-ops", args.shuffle_ops, 0xFEEDFACE) {
        dirty::minify::shuffle_commutative_ops(&mut page, seed);
    }
    if let Some(seed) = take("shuffle-at-rules", args.shuffle_at_rules, 0xD15EA5E) {
        dirty::minify::shuffle_at_rule_order(&mut page, seed);
    }
    if let Some(seed) = take("decoy-fallbacks", args.decoy_fallbacks, 0xDEC0_FA11) {
        dirty::minify::inject_var_fallbacks(&mut page, seed);
    }
    if let Some(seed) = take("decoy-arms", args.decoy_arms, 0xBADC_0DE0) {
        dirty::minify::inject_lut_decoy_arms(&mut page, seed);
    }
    if args.minify_js {
        dirty::minify::minify_embedded_js(&mut page);
    }
    let mut result = page.print();
    if !args.no_embed_compile_command {
        let cmd = std::env::args()
            .map(shell_quote)
            .collect::<Vec<_>>()
            .join(" ")
            .replace("-->", "--&gt;");
        let seeds_line = if seeds_used.is_empty() {
            String::new()
        } else {
            let pairs: Vec<String> = seeds_used.iter().map(|(n, s)| format!("{n}={s}")).collect();
            format!("\n<!-- seeds: {} -->", pairs.join(" "))
        };
        result = format!("<!-- compile command: {cmd} -->{seeds_line}\n{result}");
    }
    std::fs::write(&output_file, result)
        .with_context(|| format!("failed to write output HTML '{}'", output_file.display()))?;

    Ok(())
}

fn read_wasm_bytes(path: &Path) -> Result<Vec<u8>> {
    if path.extension().is_some_and(|ext| ext == "wat") {
        wat::parse_file(path)
            .with_context(|| format!("failed to parse WAT file '{}'", path.display()))
    } else {
        std::fs::read(path)
            .with_context(|| format!("failed to read WebAssembly file '{}'", path.display()))
    }
}

fn shell_quote(arg: String) -> String {
    if !arg.is_empty()
        && arg.bytes().all(|b| {
            b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b'/' | b'=' | b':' | b',')
        })
    {
        arg
    } else {
        format!("'{}'", arg.replace('\'', "'\\''"))
    }
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

    #[test]
    fn randomize_pc_conflicts_with_js_clock_debugger() {
        let err =
            Cli::try_parse_from(["wss", "input.wasm", "--randomize-pc", "--js-clock-debugger"])
                .expect_err("parse should fail");

        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn minify_vars_conflicts_with_js_clock_debugger() {
        let err =
            Cli::try_parse_from(["wss", "input.wasm", "--minify-vars", "--js-clock-debugger"])
                .expect_err("parse should fail");

        assert_eq!(err.kind(), ErrorKind::ArgumentConflict);
    }

    #[test]
    fn randomize_pc_and_minify_vars_coexist() {
        let args = Cli::try_parse_from(["wss", "input.wasm", "--randomize-pc=42", "--minify-vars"])
            .unwrap();

        assert_eq!(args.randomize_pc.expect("set").0, Some(42));
        assert!(args.minify_vars.is_some());
        assert!(args.minify_vars.unwrap().0.is_none());
    }

    #[test]
    fn read_wasm_bytes_parses_wat_file() {
        let dir = tempfile::tempdir().unwrap();
        let wat_path = dir.path().join("test.wat");
        std::fs::write(
            &wat_path,
            "(module (func (export \"_start\") (result i32) i32.const 0))",
        )
        .unwrap();

        let bytes = read_wasm_bytes(&wat_path).unwrap();

        assert_eq!(&bytes[..4], b"\0asm");
    }

    #[test]
    fn read_wasm_bytes_reads_wasm_file_as_raw_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let wasm_path = dir.path().join("test.wasm");
        let raw = b"\0asm\x01\x00\x00\x00";
        std::fs::write(&wasm_path, raw).unwrap();

        let bytes = read_wasm_bytes(&wasm_path).unwrap();

        assert_eq!(bytes, raw);
    }

    #[test]
    fn read_wasm_bytes_rejects_invalid_wat() {
        let dir = tempfile::tempdir().unwrap();
        let wat_path = dir.path().join("bad.wat");
        std::fs::write(&wat_path, "(module (invalid-syntax").unwrap();

        let err = read_wasm_bytes(&wat_path).unwrap_err();

        assert!(
            format!("{:#}", err).contains("failed to parse WAT"),
            "unexpected error: {:#}",
            err
        );
    }

    #[test]
    fn read_wasm_bytes_returns_error_for_missing_file() {
        let err = read_wasm_bytes(Path::new("nonexistent.wat")).unwrap_err();

        assert!(
            format!("{:#}", err).contains("failed to parse WAT"),
            "unexpected error: {:#}",
            err
        );
    }

    #[test]
    fn read_wasm_bytes_returns_error_for_missing_wasm_file() {
        let err = read_wasm_bytes(Path::new("nonexistent.wasm")).unwrap_err();

        assert!(
            format!("{:#}", err).contains("failed to read WebAssembly"),
            "unexpected error: {:#}",
            err
        );
    }
}
