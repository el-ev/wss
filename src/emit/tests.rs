use super::test_consts::{CALLSTACK_SLOTS_CAP, MEMORY_BYTES_CAP};
use super::*;

const TEST_MEMORY_BYTES: u32 = 0;

fn empty_init_bytes() -> Vec<u8> {
    Vec::new()
}

fn minimal_program_with_cycle(cycle: crate::ir8::Cycle) -> Ir8Program {
    Ir8Program {
        entry_func: 0,
        num_vregs: 0,
        func_blocks: Vec::new(),
        cycles: vec![cycle],
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: Vec::new(),
    }
}

fn minimal_exit_program() -> Ir8Program {
    minimal_program_with_cycle(crate::ir8::Cycle {
        pc: crate::ir8::Pc::new(0),
        ops: Vec::new(),
        terminator: Terminator8::Exit { val: None },
    })
}

fn emit_program(program: &Ir8Program) -> anyhow::Result<String> {
    super::emit_program(program, EmitConfig::default())
}

fn emit_program_with_config(program: &Ir8Program, config: EmitConfig) -> anyhow::Result<String> {
    super::emit_program(program, config)
}

fn register_exprs(count: u16) -> HashMap<u16, String> {
    (0..count)
        .map(|idx| (idx, format!("var(--r{})", idx)))
        .collect()
}

#[test]
fn emitter_new_uses_program_memory_end_when_memory_cap_is_zero() {
    let mut program = minimal_exit_program();
    program.memory_end = 17;

    let emitter = Emitter::new(
        &program,
        EmitConfig {
            memory_bytes_cap: 0,
            ..EmitConfig::default()
        },
    )
    .expect("emitter should initialize");
    assert_eq!(emitter.memory_end, 17);
    assert_eq!(emitter.mem_names.len(), 9);
}

#[test]
fn expr_byte_hex_masks_input_to_single_byte() {
    let expr = Emitter::byte_hex_expr("foo");
    assert!(expr.contains("mod(calc((foo) + 256), 256)"));
    assert!(expr.contains("--hex("));
}

#[test]
fn expr_add32_byte_reads_only_required_prefix_bytes() {
    let lhs = Word::new(Val8::vreg(0), Val8::vreg(1), Val8::vreg(2), Val8::vreg(3));
    let rhs = Word::new(Val8::vreg(4), Val8::vreg(5), Val8::vreg(6), Val8::vreg(7));
    let now = HashMap::new();

    let expr = Emitter::add32_byte_expr(&now, lhs, rhs, 1);

    assert!(expr.contains("var(--_1r0)"));
    assert!(expr.contains("var(--_1r1)"));
    assert!(expr.contains("var(--_1r4)"));
    assert!(expr.contains("var(--_1r5)"));
    assert!(!expr.contains("var(--_1r2)"));
    assert!(!expr.contains("var(--_1r3)"));
    assert!(!expr.contains("var(--_1r6)"));
    assert!(!expr.contains("var(--_1r7)"));
    assert!(!expr.contains("--lt("));
    assert!(expr.contains("/ 256"));
}

#[test]
fn expr_sub32_byte_reads_only_required_prefix_bytes() {
    let lhs = Word::new(Val8::vreg(0), Val8::vreg(1), Val8::vreg(2), Val8::vreg(3));
    let rhs = Word::new(Val8::vreg(4), Val8::vreg(5), Val8::vreg(6), Val8::vreg(7));
    let now = HashMap::new();

    let expr = Emitter::sub32_byte_expr(&now, lhs, rhs, 2);

    assert!(expr.contains("var(--_1r0)"));
    assert!(expr.contains("var(--_1r1)"));
    assert!(expr.contains("var(--_1r2)"));
    assert!(expr.contains("var(--_1r4)"));
    assert!(expr.contains("var(--_1r5)"));
    assert!(expr.contains("var(--_1r6)"));
    assert!(!expr.contains("var(--_1r3)"));
    assert!(!expr.contains("var(--_1r7)"));
    assert!(!expr.contains("--lt("));
    assert!(expr.contains("/ 256"));
    assert!(!expr.contains("16777216"));
}

#[test]
fn expr_add32_byte_avoids_wide_constants_for_small_immediate() {
    let lhs = Word::new(Val8::vreg(0), Val8::vreg(1), Val8::vreg(2), Val8::vreg(3));
    let rhs = Word::from_u32_imm(1);
    let now = HashMap::new();

    let expr = Emitter::add32_byte_expr(&now, lhs, rhs, 3);

    assert!(!expr.contains("16777216"));
    assert!(!expr.contains("65536"));
    assert!(!expr.contains("--lt("));
    assert!(expr.contains("/ 256"));
}

#[test]
fn expr_byte_clz_avoids_nested_sel_calls() {
    let expr = Emitter::byte_clz_expr("var(--x)");
    assert!(!expr.contains("--sel("));
    assert!(expr.contains("var(--x)"));
}

#[test]
fn expr_byte_ctz_avoids_nested_sel_calls() {
    let expr = Emitter::byte_ctz_expr("var(--x)");
    assert!(!expr.contains("--sel("));
    assert!(expr.contains("var(--x)"));
}

#[test]
fn expr_word_hex_is_big_endian_without_u32_widening() {
    let mut now = HashMap::new();
    now.insert(10, "var(--b0)".to_string());
    now.insert(11, "var(--b1)".to_string());
    now.insert(12, "var(--b2)".to_string());
    now.insert(13, "var(--b3)".to_string());

    let word = Word::new(
        Val8::vreg(10),
        Val8::vreg(11),
        Val8::vreg(12),
        Val8::vreg(13),
    );
    let expr = Emitter::word_hex_expr(&now, word);

    assert!(expr.starts_with("\"0x\" "));
    let p3 = expr.find("var(--b3)").unwrap();
    let p2 = expr.find("var(--b2)").unwrap();
    let p1 = expr.find("var(--b1)").unwrap();
    let p0 = expr.find("var(--b0)").unwrap();
    assert!(p3 < p2 && p2 < p1 && p1 < p0);
    assert!(!expr.contains("4294967296"));
}

#[test]
fn eval_builtin_division_entries_trap_when_js_coprocessor_is_disabled() {
    let program = minimal_exit_program();
    let emitter = Emitter::new(&program, EmitConfig::default()).expect("emitter should initialize");

    let lhs = Word::new(Val8::vreg(0), Val8::vreg(1), Val8::vreg(2), Val8::vreg(3));
    let rhs = Word::new(Val8::vreg(4), Val8::vreg(5), Val8::vreg(6), Val8::vreg(7));
    let now = register_exprs(8);

    for builtin in [
        BuiltinId::DivU32,
        BuiltinId::RemU32,
        BuiltinId::DivS32,
        BuiltinId::RemS32,
    ] {
        let (ret, trap) = emitter.eval_builtin(builtin, &[lhs, rhs], &now);
        assert_eq!(trap, "1", "builtin {:?} should hard-trap", builtin);
        assert_eq!(ret, ["0", "0", "0", "0"]);
    }
}

#[test]
fn eval_builtin_division_uses_js_coprocessor_channel_when_enabled() {
    let program = minimal_exit_program();
    let emitter = Emitter::new(
        &program,
        EmitConfig {
            js_coprocessor: true,
            ..EmitConfig::default()
        },
    )
    .expect("emitter should initialize");

    let lhs = Word::new(Val8::vreg(0), Val8::vreg(1), Val8::vreg(2), Val8::vreg(3));
    let rhs = Word::new(Val8::vreg(4), Val8::vreg(5), Val8::vreg(6), Val8::vreg(7));
    let now = register_exprs(8);

    let (ret, trap) = emitter.eval_builtin(BuiltinId::DivU32, &[lhs, rhs], &now);
    assert_eq!(
        ret,
        [
            "var(--cop_o0)",
            "var(--cop_o1)",
            "var(--cop_o2)",
            "var(--cop_o3)"
        ]
    );
    assert_eq!(trap, "--lt(var(--cop_o0), 0)");

    let (_, shift_trap) = emitter.eval_builtin(BuiltinId::Shl32, &[lhs, rhs], &now);
    assert_eq!(shift_trap, "0");
}

#[test]
fn eval_builtin_clz_ctz_css_path_avoids_nested_sel_calls() {
    let program = minimal_exit_program();
    let emitter = Emitter::new(&program, EmitConfig::default()).expect("emitter should initialize");

    let lhs = Word::new(Val8::vreg(0), Val8::vreg(1), Val8::vreg(2), Val8::vreg(3));
    let now = register_exprs(4);

    let (clz_ret, clz_trap) = emitter.eval_builtin(BuiltinId::Clz32, &[lhs], &now);
    assert_eq!(clz_trap, "0");
    assert!(!clz_ret[0].contains("--sel("));

    let (ctz_ret, ctz_trap) = emitter.eval_builtin(BuiltinId::Ctz32, &[lhs], &now);
    assert_eq!(ctz_trap, "0");
    assert!(!ctz_ret[0].contains("--sel("));
}

#[test]
fn emit_html_includes_js_coprocessor_runtime_when_enabled() {
    let program = minimal_exit_program();
    let html = emit_program_with_config(
        &program,
        EmitConfig {
            js_coprocessor: true,
            ..EmitConfig::default()
        },
    )
    .expect("emit should succeed");
    assert!(html.contains("@property --cop_op"));
    assert!(html.contains("@property --cop_a0"));
    assert!(html.contains("@property --cop_b3"));
    assert!(html.contains("@property --cop_o3"));
    assert!(!html.contains("const jsCoprocessorEnabled"));
    assert!(html.contains(" --cop_op: "));
}

#[test]
fn emit_html_omits_js_clock_runtime_when_disabled() {
    let program = minimal_exit_program();
    let html = emit_program_with_config(
        &program,
        EmitConfig {
            js_clock: false,
            ..EmitConfig::default()
        },
    )
    .expect("emit should succeed");
    assert!(!html.contains("<script>"));
    assert!(!html.contains("const jsClockEnabled"));
    assert!(!html.contains("function tickInstruction()"));
    assert!(!html.contains("requestAnimationFrame(animate);"));
}

#[test]
fn emit_html_includes_js_clock_debugger_when_enabled() {
    let program = minimal_exit_program();
    let html = emit_program_with_config(
        &program,
        EmitConfig {
            js_clock: true,
            js_clock_debugger: true,
            ..EmitConfig::default()
        },
    )
    .expect("emit should succeed");
    assert!(html.contains("let paused = false;"));
    assert!(html.contains("paused = true;"));
    assert!(html.contains("popup.hidden = false;"));
    assert!(html.contains("className = \"wss-debug-trigger\";"));
    assert!(html.contains("data-wss-debug-step=\"1\""));
    assert!(html.contains("data-wss-debug-run=\"1\">Run</button>"));
    assert!(html.contains("data-wss-debug-state=\"1\">paused</dd>"));
    assert!(html.contains(".wss-debug-trigger {"));
}

#[test]
fn emit_html_omits_js_clock_debugger_css_when_disabled() {
    let program = minimal_exit_program();
    let html = emit_program_with_config(
        &program,
        EmitConfig {
            js_clock: true,
            js_clock_debugger: false,
            ..EmitConfig::default()
        },
    )
    .expect("emit should succeed");
    assert!(!html.contains(".wss-debug-trigger {"));
    assert!(!html.contains(".wss-debug-popup {"));
    assert!(!html.contains("className = \"wss-debug-trigger\";"));
    assert!(!html.contains("data-wss-debug-step=\"1\""));
    assert!(!html.contains("popup.hidden = false;"));
}

#[test]
fn emit_html_rejects_js_clock_debugger_without_js_clock() {
    let program = minimal_exit_program();
    let err = emit_program_with_config(
        &program,
        EmitConfig {
            js_clock: false,
            js_clock_debugger: true,
            ..EmitConfig::default()
        },
    )
    .expect_err("emit should fail without js_clock");
    assert!(
        err.to_string()
            .contains("js clock debugger requires js clock stepping to be enabled")
    );
}

#[test]
fn emit_html_omits_js_coprocessor_runtime_when_disabled() {
    let program = minimal_exit_program();
    let html = emit_program_with_config(
        &program,
        EmitConfig {
            js_clock: true,
            js_coprocessor: false,
            ..EmitConfig::default()
        },
    )
    .expect("emit should succeed");
    assert!(!html.contains("const jsCoprocessorEnabled"));
    assert!(!html.contains("function runCoprocessor("));
    assert!(!html.contains("runCoprocessor(getComputedStyle(terminalEl));"));
    assert!(!html.contains("writeCopOutputWord("));
}

#[test]
fn emit_html_includes_callstack_overflow_trap_ui_and_guard() {
    let alloc = (CALLSTACK_SLOTS_CAP + 1) as u16;
    let program = minimal_program_with_cycle(crate::ir8::Cycle {
        pc: crate::ir8::Pc::new(0),
        ops: vec![crate::ir8::Inst8::no_dst(Inst8Kind::CsAlloc(alloc))],
        terminator: Terminator8::Exit { val: None },
    });

    let html = emit_program(&program).expect("emit should succeed");
    assert!(html.contains("@container style(--pc: -5)"));
    assert!(html.contains("[Trap: callstack overflow]"));
    let alloc_expr = format!("calc(var(--_1cs_sp) + {})", alloc);
    assert!(html.contains(&alloc_expr));
    assert!(html.contains("--lt(-1, calc(var(--_1cs_sp) +"));
    assert!(html.contains(", -5,"));
}

#[test]
fn emit_html_screen_after_uses_pc_status_fallback() {
    let program = minimal_exit_program();

    let html = emit_program(&program).expect("emit should succeed");
    assert!(
        html.contains(
            ".screen::after { white-space: pre-wrap; word-break: break-all; content: if("
        )
    );
    assert!(html.contains("style(--pc: -4): var(--fb, \"\") \"\\a[Trap: division by zero]\";"));
    assert!(html.contains("color: if(style(--pc: -2): #d00; style(--pc: -3): #d00; style(--pc: -4): #d00; else: #222);"));
}

#[test]
fn emit_html_chunks_large_generated_css_sections() {
    let program = minimal_exit_program();
    let html = emit_program(&program).expect("emit should succeed");
    let max_line = html.lines().map(str::len).max().unwrap_or(0);
    assert!(
        max_line < 20_000,
        "expected chunked output, got line length {}",
        max_line
    );
    assert!(html.contains("@function --read_m16_0"));
    assert!(!html.contains("@function --read_cs_0"));
    assert!(html.contains(".memvis { width:"));
    assert!(html.contains("var(--mv-0)"));
    assert!(!html.contains("WSS_KEEP_"));
}

#[test]
fn emit_html_omits_callstack_state_when_unused() {
    let program = minimal_exit_program();
    let html = emit_program(&program).expect("emit should succeed");
    assert!(!html.contains("@property --cs_sp"));
    assert!(!html.contains("--_1cs_sp"));
    assert!(!html.contains("--read_cs("));
    assert!(!html.contains("[Trap: callstack overflow]"));
    assert!(!html.contains(".csvis { width:"));
    assert!(html.contains(".memvis { width:"));
    assert!(!html.contains(&format!(
        "{}:",
        Emitter::cell_name("cs", 0, Emitter::cell_offset_hex_width(CALLSTACK_SLOTS_CAP),)
    )));
}

#[test]
fn emit_html_memory_visualizer_tracks_read_and_write_slots() {
    let addr = crate::ir8::Addr::new(Val8::vreg(0), Val8::vreg(1));
    let program = Ir8Program {
        entry_func: 0,
        num_vregs: 3,
        func_blocks: Vec::new(),
        cycles: vec![crate::ir8::Cycle {
            pc: crate::ir8::Pc::new(0),
            ops: vec![
                crate::ir8::Inst8::with_dst(
                    Val8::vreg(2),
                    Inst8Kind::LoadMem {
                        base: 0,
                        addr,
                        lane: 0,
                    },
                ),
                crate::ir8::Inst8::no_dst(Inst8Kind::StoreMem {
                    base: 0,
                    addr,
                    lane: 1,
                    val: Val8::vreg(2),
                }),
            ],
            terminator: Terminator8::Exit { val: None },
        }],
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: Vec::new(),
    };

    let html = emit_program(&program).expect("emit should succeed");
    assert!(html.contains("--mri0:"));
    assert!(html.contains("--mro0:"));
    assert!(html.contains("--mvh-0"));
    assert!(html.contains(".memvis::before {"));
    assert!(html.contains("rgb(calc(var(--m000) / 256)"));
    assert!(!html.contains("rgb(mod(round(down, calc(var(--m000) / 256)), 256)"));
}

#[test]
fn emit_html_callstack_visualizer_tracks_read_and_write_slots() {
    let program = Ir8Program {
        entry_func: 0,
        num_vregs: 1,
        func_blocks: Vec::new(),
        cycles: vec![crate::ir8::Cycle {
            pc: crate::ir8::Pc::new(0),
            ops: vec![
                crate::ir8::Inst8::no_dst(Inst8Kind::CsStore {
                    offset: 0,
                    val: Val8::vreg(0),
                }),
                crate::ir8::Inst8::with_dst(Val8::vreg(0), Inst8Kind::CsLoad { offset: 0 }),
            ],
            terminator: Terminator8::Exit { val: None },
        }],
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: Vec::new(),
    };

    let html = emit_program(&program).expect("emit should succeed");
    let cs0_name = Emitter::cell_name("cs", 0, Emitter::cell_offset_hex_width(CALLSTACK_SLOTS_CAP));
    let cs0_shadow = Emitter::shadow_name(1, &cs0_name);
    assert!(html.contains(".csvis { width:"));
    assert!(html.contains("--cri0:"));
    assert!(html.contains("--cro0:"));
    assert!(html.contains("--csh-0"));
    assert!(html.contains("#ind-sp::before"));
    assert!(html.contains("id=\"ind-sp\""));
    assert!(
        html.contains("@function --csmerge(--idx <number>, --prev <number>) returns <integer>")
    );
    assert!(!html.contains("--csmerge_slot("));
    assert!(html.contains(" --csw_active: if("));
    assert!(html.contains(" --cswdp0:"));
    assert!(html.contains("@property --cswdp0 { syntax: \"<integer>\";"));
    assert!(html.contains(&format!(
            " {}: if(style(--csw_active: 1): if(style(--cswdp0: 1): --csmerge(0, var({})); else: var({})); else: var({}));",
            cs0_name, cs0_shadow, cs0_shadow, cs0_shadow
        )));
}

#[test]
fn emit_html_partitions_active_flags_when_many_writer_pcs() {
    let total = (ACTIVE_FLAG_ARMS_CHUNK as u16) + 3;
    let addr = crate::ir8::Addr::new(Val8::vreg(0), Val8::vreg(1));
    let mut cycles = Vec::with_capacity(total as usize);
    for i in 0..total {
        let term = if i + 1 < total {
            Terminator8::Goto(crate::ir8::Pc::new(i + 1))
        } else {
            Terminator8::Exit { val: None }
        };
        cycles.push(crate::ir8::Cycle {
            pc: crate::ir8::Pc::new(i),
            ops: vec![
                crate::ir8::Inst8::no_dst(Inst8Kind::StoreMem {
                    base: 0,
                    addr,
                    lane: 0,
                    val: Val8::vreg(2),
                }),
                crate::ir8::Inst8::no_dst(Inst8Kind::CsStore {
                    offset: 0,
                    val: Val8::vreg(3),
                }),
            ],
            terminator: term,
        });
    }
    let program = Ir8Program {
        entry_func: 0,
        num_vregs: 4,
        func_blocks: Vec::new(),
        cycles,
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: Vec::new(),
    };

    let html = emit_program(&program).expect("emit should succeed");
    assert!(html.contains(" --mw_activep0: if("));
    assert!(html.contains(" --mw_activep1: if("));
    assert!(html.contains(" --mw_active: min(1, calc("));
    assert!(html.contains(" --csw_activep0: if("));
    assert!(html.contains(" --csw_activep1: if("));
    assert!(html.contains(" --csw_active: min(1, calc("));
}

#[test]
fn emit_html_mload_helper_avoids_local_aliasing() {
    let addr = crate::ir8::Addr::new(Val8::vreg(0), Val8::vreg(1));
    let program = Ir8Program {
        entry_func: 0,
        num_vregs: 2,
        func_blocks: Vec::new(),
        cycles: vec![crate::ir8::Cycle {
            pc: crate::ir8::Pc::new(0),
            ops: vec![crate::ir8::Inst8::with_dst(
                Val8::vreg(0),
                Inst8Kind::LoadMem {
                    base: 0,
                    addr,
                    lane: 0,
                },
            )],
            terminator: Terminator8::Exit { val: None },
        }],
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: Vec::new(),
    };
    let html = emit_program(&program).expect("emit should succeed");
    assert!(html.contains("@function --mload(--byte <number>) returns <integer>"));
    assert!(html.contains("mod(--read_m16(--mhalf(var(--byte))), 256)"));
    assert!(!html.contains("@function --ne(--a <number>, --b <number>) returns <integer>"));
    assert!(!html.contains("--cell: --read_m16"));
}

#[test]
fn emit_html_omits_keyboard_ui_when_getchar_is_unused() {
    let program = minimal_exit_program();
    let html = emit_program(&program).expect("emit should succeed");
    assert!(!html.contains("@property --kb"));
    assert!(html.contains(" --wait_input: 0;"));
    assert!(!html.contains("class=\"kb\""));
    assert!(!html.contains("class=\"input-hint\""));
    assert!(!html.contains("data-key="));
    assert!(!html.contains(".key.dual"));
    assert!(!html.contains("--kb: -1;"));
}

#[test]
fn emit_html_includes_keyboard_ui_when_getchar_is_used() {
    let program = Ir8Program {
        entry_func: 0,
        num_vregs: 1,
        func_blocks: Vec::new(),
        cycles: vec![crate::ir8::Cycle {
            pc: crate::ir8::Pc::new(0),
            ops: vec![crate::ir8::Inst8::with_dst(
                Val8::vreg(0),
                Inst8Kind::Getchar,
            )],
            terminator: Terminator8::Exit { val: None },
        }],
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: Vec::new(),
    };
    let html = emit_program(&program).expect("emit should succeed");
    assert!(html.contains("@property --kb"));
    assert!(html.contains("@property --wait_input"));
    assert!(html.contains(" --wait_input: if("));
    assert!(html.contains("calc(1 - (--ne(var(--kb, -1), -1)))"));
    assert!(html.contains("@function --ne(--a <number>, --b <number>) returns <integer>"));
    assert!(html.contains("class=\"kb\""));
    assert!(html.contains("class=\"input-hint\""));
    assert!(html.contains("data-key=\"32\""));
    assert!(html.contains(".unsupported { --support: --support-test(); display: if(style(--support:2):none; else:flex); position: fixed; inset: 0; z-index: 100;"));
}

#[test]
fn emit_html_includes_ne_helper_when_clz_builtin_is_used() {
    let arg = Word::new(Val8::vreg(0), Val8::vreg(1), Val8::vreg(2), Val8::vreg(3));
    let program = Ir8Program {
        entry_func: 0,
        num_vregs: 4,
        func_blocks: Vec::new(),
        cycles: vec![
            crate::ir8::Cycle {
                pc: crate::ir8::Pc::new(0),
                ops: Vec::new(),
                terminator: Terminator8::CallSetup {
                    callee_entry: CallTarget::Builtin(BuiltinId::Clz32),
                    cont: crate::ir8::Pc::new(1),
                    args: vec![arg],
                    callee_arg_vregs: vec![arg],
                },
            },
            crate::ir8::Cycle {
                pc: crate::ir8::Pc::new(1),
                ops: Vec::new(),
                terminator: Terminator8::Exit { val: None },
            },
        ],
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: Vec::new(),
    };

    let html = emit_program(&program).expect("emit should succeed");
    assert!(html.contains("@function --ne(--a <number>, --b <number>) returns <integer>"));
}

#[test]
fn emit_html_untouched_registers_use_direct_fallback_without_empty_if() {
    let program = Ir8Program {
        entry_func: 0,
        num_vregs: 8,
        func_blocks: Vec::new(),
        cycles: vec![crate::ir8::Cycle {
            pc: crate::ir8::Pc::new(0),
            ops: Vec::new(),
            terminator: Terminator8::Exit { val: None },
        }],
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: Vec::new(),
    };
    let html = emit_program(&program).expect("emit should succeed");
    assert!(html.contains(" --r7: var(--_1r7);"));
    assert!(!html.contains(" --r7: if(else: var(--_1r7));"));
}

#[test]
fn emit_html_pc_fallback_increments_for_trivial_fallthrough() {
    let program = Ir8Program {
        entry_func: 0,
        num_vregs: 0,
        func_blocks: Vec::new(),
        cycles: vec![
            crate::ir8::Cycle {
                pc: crate::ir8::Pc::new(0),
                ops: Vec::new(),
                terminator: Terminator8::Goto(crate::ir8::Pc::new(1)),
            },
            crate::ir8::Cycle {
                pc: crate::ir8::Pc::new(1),
                ops: Vec::new(),
                terminator: Terminator8::Exit { val: None },
            },
        ],
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: Vec::new(),
    };

    let html = emit_program(&program).expect("emit should succeed");
    assert!(!html.contains("style(--_1pc: 0): 1;"));
    assert!(html.contains("--sel(--lt(var(--_1pc), 0), var(--_1pc), calc(var(--_1pc) + 1))"));
}

#[test]
fn emit_html_pc_keeps_explicit_arm_when_cycle_has_trap_guard() {
    let addr = crate::ir8::Addr::new(Val8::vreg(0), Val8::vreg(1));
    let program = Ir8Program {
        entry_func: 0,
        num_vregs: 2,
        func_blocks: Vec::new(),
        cycles: vec![
            crate::ir8::Cycle {
                pc: crate::ir8::Pc::new(0),
                ops: vec![crate::ir8::Inst8::no_dst(Inst8Kind::StoreMem {
                    base: 0,
                    addr,
                    lane: 0,
                    val: Val8::vreg(0),
                })],
                terminator: Terminator8::Goto(crate::ir8::Pc::new(1)),
            },
            crate::ir8::Cycle {
                pc: crate::ir8::Pc::new(1),
                ops: Vec::new(),
                terminator: Terminator8::Exit { val: None },
            },
        ],
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: Vec::new(),
    };

    let html = emit_program(&program).expect("emit should succeed");
    assert!(html.contains("style(--_1pc: 0): --sel("));
}

#[test]
fn emit_html_globals_emit_all_lanes_on_single_line_per_global() {
    let program = Ir8Program {
        entry_func: 0,
        num_vregs: 0,
        func_blocks: Vec::new(),
        cycles: vec![crate::ir8::Cycle {
            pc: crate::ir8::Pc::new(0),
            ops: Vec::new(),
            terminator: Terminator8::Exit { val: None },
        }],
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: vec![0x1122_3344],
    };
    let html = emit_program(&program).expect("emit should succeed");

    let g0_lane0 = Emitter::global_lane_name(0, 0);
    let g0_lane1 = Emitter::global_lane_name(0, 1);
    let g0_lane2 = Emitter::global_lane_name(0, 2);
    let g0_lane3 = Emitter::global_lane_name(0, 3);
    let _1g0_lane0 = Emitter::staged_global_lane_name(1, 0, 0);
    let _1g0_lane1 = Emitter::staged_global_lane_name(1, 0, 1);
    let _1g0_lane2 = Emitter::staged_global_lane_name(1, 0, 2);
    let _1g0_lane3 = Emitter::staged_global_lane_name(1, 0, 3);
    let _2g0_lane0 = Emitter::staged_global_lane_name(2, 0, 0);
    let _2g0_lane1 = Emitter::staged_global_lane_name(2, 0, 1);
    let _2g0_lane2 = Emitter::staged_global_lane_name(2, 0, 2);
    let _2g0_lane3 = Emitter::staged_global_lane_name(2, 0, 3);
    let _0g0_lane0 = Emitter::staged_global_lane_name(0, 0, 0);
    let _0g0_lane1 = Emitter::staged_global_lane_name(0, 0, 1);
    let _0g0_lane2 = Emitter::staged_global_lane_name(0, 0, 2);
    let _0g0_lane3 = Emitter::staged_global_lane_name(0, 0, 3);

    let g_props = html
        .lines()
        .find(|line| line.contains(&format!("@property {}", g0_lane0)))
        .expect("missing global properties line");
    assert!(g_props.contains(&format!("@property {}", g0_lane1)));
    assert!(g_props.contains(&format!("@property {}", g0_lane2)));
    assert!(g_props.contains(&format!("@property {}", g0_lane3)));

    let g_shadow = html
        .lines()
        .find(|line| line.contains(&_1g0_lane0))
        .expect("missing _1g line");
    assert!(g_shadow.contains(&_1g0_lane1));
    assert!(g_shadow.contains(&_1g0_lane2));
    assert!(g_shadow.contains(&_1g0_lane3));

    let g_line = html
        .lines()
        .find(|line| line.contains(&format!("{}:", g0_lane0)))
        .expect("missing --g line");
    assert!(g_line.contains(&format!("{}:", g0_lane1)));
    assert!(g_line.contains(&format!("{}:", g0_lane2)));
    assert!(g_line.contains(&format!("{}:", g0_lane3)));

    let g_store = html
        .lines()
        .find(|line| line.contains(&format!("{}: var({}", _2g0_lane0, _0g0_lane0)))
        .expect("missing _2g line");
    assert!(g_store.contains(&_2g0_lane1));
    assert!(g_store.contains(&_2g0_lane2));
    assert!(g_store.contains(&_2g0_lane3));
    assert!(g_store.contains(&format!("var({}, 68)", _0g0_lane0)));
    assert!(g_store.contains(&format!("var({}, 51)", _0g0_lane1)));
    assert!(g_store.contains(&format!("var({}, 34)", _0g0_lane2)));
    assert!(g_store.contains(&format!("var({}, 17)", _0g0_lane3)));

    let g_exec = html
        .lines()
        .find(|line| line.contains(&_0g0_lane0))
        .expect("missing _0g line");
    assert!(g_exec.contains(&_0g0_lane1));
    assert!(g_exec.contains(&_0g0_lane2));
    assert!(g_exec.contains(&_0g0_lane3));

    assert!(g_shadow.contains(&format!("var({}, 68)", _2g0_lane0)));
    assert!(g_shadow.contains(&format!("var({}, 51)", _2g0_lane1)));
    assert!(g_shadow.contains(&format!("var({}, 34)", _2g0_lane2)));
    assert!(g_shadow.contains(&format!("var({}, 17)", _2g0_lane3)));
}

#[test]
fn emit_html_memory_store_merge_expr_stays_compact() {
    let addr = crate::ir8::Addr::new(Val8::vreg(10), Val8::vreg(11));
    let program = Ir8Program {
        entry_func: 0,
        num_vregs: 16,
        func_blocks: Vec::new(),
        cycles: vec![crate::ir8::Cycle {
            pc: crate::ir8::Pc::new(0),
            ops: vec![
                crate::ir8::Inst8::no_dst(Inst8Kind::StoreMem {
                    base: 0,
                    addr,
                    lane: 0,
                    val: Val8::vreg(12),
                }),
                crate::ir8::Inst8::no_dst(Inst8Kind::StoreMem {
                    base: 0,
                    addr,
                    lane: 1,
                    val: Val8::vreg(13),
                }),
                crate::ir8::Inst8::no_dst(Inst8Kind::StoreMem {
                    base: 0,
                    addr,
                    lane: 2,
                    val: Val8::vreg(14),
                }),
                crate::ir8::Inst8::no_dst(Inst8Kind::StoreMem {
                    base: 0,
                    addr,
                    lane: 3,
                    val: Val8::vreg(15),
                }),
            ],
            terminator: Terminator8::Exit { val: None },
        }],
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: Vec::new(),
    };

    let html = emit_program(&program).expect("emit should succeed");
    let m0_name = Emitter::cell_name(
        "m",
        0,
        Emitter::cell_offset_hex_width((MEMORY_BYTES_CAP as usize).div_ceil(2)),
    );
    let m1_name = Emitter::cell_name(
        "m",
        1,
        Emitter::cell_offset_hex_width((MEMORY_BYTES_CAP as usize).div_ceil(2)),
    );
    let m0_start = html
        .find(&format!(" {}: ", m0_name))
        .expect("must emit first memory cell");
    let next_decl = html[m0_start + 1..]
        .find(&format!(" {}: ", m1_name))
        .map(|idx| m0_start + 1 + idx)
        .or_else(|| html[m0_start..].find('\n').map(|idx| m0_start + idx))
        .unwrap_or(html.len());
    let m0 = &html[m0_start..next_decl];
    assert!(
        m0.len() < 4_000,
        "expected compact merge expression, got {} chars",
        m0.len()
    );
    assert!(html.contains("--msc0"));
    assert!(!html.contains("--ms_half0"));
}

#[test]
fn emit_html_memory_store_merge_expr_flattens_slot_conditions() {
    let addr = crate::ir8::Addr::new(Val8::vreg(10), Val8::vreg(11));
    let program = Ir8Program {
        entry_func: 0,
        num_vregs: 16,
        func_blocks: Vec::new(),
        cycles: vec![crate::ir8::Cycle {
            pc: crate::ir8::Pc::new(0),
            ops: vec![
                crate::ir8::Inst8::no_dst(Inst8Kind::StoreMem {
                    base: 0,
                    addr,
                    lane: 0,
                    val: Val8::vreg(12),
                }),
                crate::ir8::Inst8::no_dst(Inst8Kind::StoreMem {
                    base: 0,
                    addr,
                    lane: 1,
                    val: Val8::vreg(13),
                }),
            ],
            terminator: Terminator8::Exit { val: None },
        }],
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: Vec::new(),
    };

    let html = emit_program(&program).expect("emit should succeed");
    let m0_name = Emitter::cell_name(
        "m",
        0,
        Emitter::cell_offset_hex_width((MEMORY_BYTES_CAP as usize).div_ceil(2)),
    );
    let m0_shadow = Emitter::shadow_name(1, &m0_name);
    assert!(!html.contains("@function --mmerge_byte("));
    assert!(
        html.contains("@function --mmerge16(--cell <number>, --prev <number>) returns <integer>")
    );
    assert!(html.contains("--eq1(var(--msp0))"));
    assert!(html.contains(" --mwdp0: min(1, calc("));
    assert!(html.contains("@property --mwdp0 { syntax: \"<integer>\";"));
    assert!(html.contains(" --mw_active: if("));
    assert!(html.contains(&format!(
            " {}: if(style(--mw_active: 1): if(style(--mwdp0: 1): --mmerge16(0, var({})); else: var({})); else: var({}));",
            m0_name, m0_shadow, m0_shadow, m0_shadow
        )));
    assert!(
        !html.contains("calc((calc(var(--mso0) * --eq(var(--msc0), 0))) * --eqz(var(--msp0)))")
    );
    assert!(
        html.contains("--msc0b: if("),
        "paired slot secondary fields should be emitted"
    );
}

#[test]
fn emit_html_memory_store_merge_expr_uses_byte_helpers_when_no_writes() {
    let program = minimal_exit_program();
    let html = emit_program(&program).expect("emit should succeed");
    let m0_name = Emitter::cell_name(
        "m",
        0,
        Emitter::cell_offset_hex_width((MEMORY_BYTES_CAP as usize).div_ceil(2)),
    );
    let m0_shadow = Emitter::shadow_name(1, &m0_name);
    assert!(html.contains("@function --mlo(--w <number>) returns <integer>"));
    assert!(html.contains("@function --mhi(--w <number>) returns <integer>"));
    assert!(html.contains("@function --m16(--lo <number>, --hi <number>) returns <integer>"));
    assert!(html.contains(&format!(" {}: var({});", m0_name, m0_shadow)));
    assert!(!html.contains("@function --mmerge16("));
    assert!(!html.contains("@function --mmerge_byte("));
    assert!(!html.contains(" --mw_active: if("));
}

#[test]
fn emit_html_return_falls_back_to_callstack_top_when_cs_load_pc_is_in_prior_cycle() {
    let program = Ir8Program {
        entry_func: 0,
        num_vregs: 0,
        func_blocks: Vec::new(),
        cycles: vec![
            crate::ir8::Cycle {
                pc: crate::ir8::Pc::new(0),
                ops: vec![crate::ir8::Inst8::no_dst(Inst8Kind::CsLoadPc { offset: 0 })],
                terminator: Terminator8::Goto(crate::ir8::Pc::new(1)),
            },
            crate::ir8::Cycle {
                pc: crate::ir8::Pc::new(1),
                ops: Vec::new(),
                terminator: Terminator8::Goto(crate::ir8::Pc::new(2)),
            },
            crate::ir8::Cycle {
                pc: crate::ir8::Pc::new(2),
                ops: Vec::new(),
                terminator: Terminator8::Return { val: None },
            },
        ],
        func_entries: Vec::new(),
        func_num_locals: Vec::new(),
        memory_end: TEST_MEMORY_BYTES,
        init_bytes: empty_init_bytes(),
        global_init: Vec::new(),
    };

    let html = emit_program(&program).expect("emit should succeed");
    assert!(html.contains("style(--_1pc: 2): --sel("));
    assert!(html.contains("style(--_1pc: 2): --sel(calc(--lt(-1, calc(var(--_1cs_sp) + 0))"));
    assert!(html.contains("--read_cs(calc(var(--_1cs_sp) + 0))"));
    assert!(!html.contains("@function --csmerge("));
    assert!(!html.contains(" --csw_active: if("));
}
