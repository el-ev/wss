mod inline;
mod logic;
mod properties;
mod support;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod test_consts {
    pub(super) const MEMORY_BYTES_CAP: u32 = crate::constants::DEFAULT_MEMORY_BYTES_CAP;
    pub(super) const CALLSTACK_SLOTS_CAP: usize = crate::constants::DEFAULT_CALLSTACK_SLOTS_CAP;
}

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write;

use anyhow::Context;
use bitflags::bitflags;
use pastey::paste;

#[cfg(test)]
use crate::constants::{
    DEFAULT_CALLSTACK_SLOTS_CAP, DEFAULT_JS_CLOCK_DEBUGGER_ENABLED, DEFAULT_JS_CLOCK_ENABLED,
    DEFAULT_JS_COPROCESSOR_ENABLED, DEFAULT_MEMORY_BYTES_CAP,
};
use crate::css::skip_css_string;
use crate::ir8::{BuiltinId, CallTarget, Inst8Kind, Ir8Program, Terminator8, TrapCode, Val8, Word};

const BASE_HTML: &str = include_str!("base.html");
const PROPS_PLACEHOLDER: &str = "/*__WSS_PROPS__*/";
const LOGIC_PLACEHOLDER: &str = "/*__WSS_LOGIC__*/";
const SUPPORT_PLACEHOLDER: &str = "/*__WSS_SUPPORT__*/";
const TERMINAL_PCS_PLACEHOLDER: &str = "/*__WSS_TERMINAL_PCS__*/";
const DEBUGGER_CYCLES_PLACEHOLDER: &str = "/*__WSS_DEBUGGER_CYCLES__*/";
const READ_LOOKUP_CHUNK: usize = 128;
const VIS_SHADOW_CHUNK: usize = 8;
const VIS_COLS: usize = 128;
const MEM_DIRTY_PAGE_CELLS: usize = 16;
const CALLSTACK_DIRTY_PAGE_CELLS: usize = 16;
const COP_OP_NONE: u8 = 0;
// TODO(i64): exit-code display currently formats return values as 32-bit hex words.
pub(super) const DEFAULT_RA_DISPLAY: &str = "0x00000000";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmitConfig {
    memory_bytes_cap: u32,
    callstack_slots_cap: usize,
    js_clock: bool,
    js_coprocessor: bool,
    js_clock_debugger: bool,
    visualizers: bool,
    mem_trap: bool,
    cs_trap: bool,
    indicators: bool,
}

#[cfg(test)]
impl Default for EmitConfig {
    fn default() -> Self {
        Self::new(
            DEFAULT_MEMORY_BYTES_CAP,
            DEFAULT_CALLSTACK_SLOTS_CAP,
            DEFAULT_JS_CLOCK_ENABLED,
            DEFAULT_JS_COPROCESSOR_ENABLED,
            DEFAULT_JS_CLOCK_DEBUGGER_ENABLED,
            true,
            true,
            true,
            true,
        )
        .expect("default emit config should be valid")
    }
}

impl EmitConfig {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        memory_bytes_cap: u32,
        callstack_slots_cap: usize,
        js_clock: bool,
        js_coprocessor: bool,
        js_clock_debugger: bool,
        visualizers: bool,
        mem_trap: bool,
        cs_trap: bool,
        indicators: bool,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(
            callstack_slots_cap > 0,
            "callstack slots cap must be greater than zero"
        );
        anyhow::ensure!(
            !js_coprocessor || js_clock,
            "js coprocessor requires js clock stepping to be enabled"
        );
        anyhow::ensure!(
            !js_clock_debugger || js_clock,
            "js clock debugger requires js clock stepping to be enabled"
        );
        Ok(Self {
            memory_bytes_cap,
            callstack_slots_cap,
            js_clock,
            js_coprocessor,
            js_clock_debugger,
            visualizers,
            mem_trap,
            cs_trap,
            indicators,
        })
    }

    pub fn memory_bytes_cap(&self) -> u32 {
        self.memory_bytes_cap
    }

    pub fn callstack_slots_cap(&self) -> usize {
        self.callstack_slots_cap
    }

    pub fn js_clock(&self) -> bool {
        self.js_clock
    }

    pub fn js_coprocessor(&self) -> bool {
        self.js_coprocessor
    }

    pub fn js_clock_debugger(&self) -> bool {
        self.js_clock_debugger
    }

    pub fn visualizers(&self) -> bool {
        self.visualizers
    }

    pub fn mem_trap(&self) -> bool {
        self.mem_trap
    }

    pub fn cs_trap(&self) -> bool {
        self.cs_trap
    }

    pub fn indicators(&self) -> bool {
        self.indicators
    }

    #[cfg(test)]
    pub fn with_memory_bytes_cap(self, memory_bytes_cap: u32) -> anyhow::Result<Self> {
        Self::new(
            memory_bytes_cap,
            self.callstack_slots_cap(),
            self.js_clock(),
            self.js_coprocessor(),
            self.js_clock_debugger(),
            self.visualizers(),
            self.mem_trap(),
            self.cs_trap(),
            self.indicators(),
        )
    }

    #[cfg(test)]
    pub fn with_js_clock(self, js_clock: bool) -> anyhow::Result<Self> {
        Self::new(
            self.memory_bytes_cap(),
            self.callstack_slots_cap(),
            js_clock,
            self.js_coprocessor(),
            self.js_clock_debugger(),
            self.visualizers(),
            self.mem_trap(),
            self.cs_trap(),
            self.indicators(),
        )
    }

    #[cfg(test)]
    pub fn with_js_coprocessor(self, js_coprocessor: bool) -> anyhow::Result<Self> {
        Self::new(
            self.memory_bytes_cap(),
            self.callstack_slots_cap(),
            self.js_clock(),
            js_coprocessor,
            self.js_clock_debugger(),
            self.visualizers(),
            self.mem_trap(),
            self.cs_trap(),
            self.indicators(),
        )
    }

    #[cfg(test)]
    pub fn with_js_clock_debugger(self, js_clock_debugger: bool) -> anyhow::Result<Self> {
        Self::new(
            self.memory_bytes_cap(),
            self.callstack_slots_cap(),
            self.js_clock(),
            self.js_coprocessor(),
            js_clock_debugger,
            self.visualizers(),
            self.mem_trap(),
            self.cs_trap(),
            self.indicators(),
        )
    }
}

macro_rules! keep_pairs {
    (
        css: [$(($css_stem:ident, $css_name:literal)),* $(,)?],
        html: [$(($html_stem:ident, $html_name:literal)),* $(,)?]
    ) => {
        paste! {
            $(
                const [<KEEP_ $css_stem _START>]: &str =
                    concat!("/*__WSS_KEEP_", $css_name, "_START__*/");
                const [<KEEP_ $css_stem _END>]: &str =
                    concat!("/*__WSS_KEEP_", $css_name, "_END__*/");
            )*
            $(
                const [<KEEP_ $html_stem _START>]: &str =
                    concat!("<!--__WSS_KEEP_", $html_name, "_START__-->");
                const [<KEEP_ $html_stem _END>]: &str =
                    concat!("<!--__WSS_KEEP_", $html_name, "_END__-->");
            )*
        }
    };
}

keep_pairs! {
    css: [
        (PROP_FB, "PROP_FB"),
        (PROP_RA, "PROP_RA"),
        (PROP_KB, "PROP_KB"),
        (PROP_RNG, "PROP_RNG"),
        (FN_SEL, "FN_SEL"),
        (FN_EQZ, "FN_EQZ"),
        (FN_NEZ, "FN_NEZ"),
        (FN_EQ, "FN_EQ"),
        (FN_NE, "FN_NE"),
        (FN_LT, "FN_LT"),
        (FN_GE, "FN_GE"),
        (FN_INRANGE, "FN_INRANGE"),
        (FN_ADDR16, "FN_ADDR16"),
        (FN_MHALF, "FN_MHALF"),
        (FN_MPAR, "FN_MPAR"),
        (FN_MLO, "FN_MLO"),
        (FN_MHI, "FN_MHI"),
        (FN_M16, "FN_M16"),
        (FN_MLOAD, "FN_MLOAD"),
        (INDS_CSS, "INDS_CSS"),
        (SP_IND_CSS, "SP_INDICATOR_CSS"),
        (MEM_VIS_CSS, "MEM_VISUALIZER_CSS"),
        (CS_VIS_CSS, "CS_VISUALIZER_CSS"),
        (KB_CSS, "KB_CSS"),
        (KB_HINT_CSS, "KB_HINT_CSS"),
        (KB_CLK_DEFAULT, "KB_CLK_DEFAULT"),
        (KB_INPUT_CSS, "KB_INPUT_CSS"),
        (JS_CLOCK_DEBUGGER_CSS, "JS_CLOCK_DEBUGGER_CSS"),
        (JS_CLOCK_DEBUGGER_RUNTIME, "JS_CLOCK_DEBUGGER_RUNTIME"),
        (DEBUGGER_RNG, "DEBUGGER_RNG"),
        (JS_COPROCESSOR_RUNTIME, "JS_COPROCESSOR_RUNTIME")
    ],
    html: [
        (INDS_HTML, "INDS_HTML"),
        (SP_IND_HTML, "SP_INDICATOR_HTML"),
        (MEM_VIS_HTML, "MEM_VISUALIZER_HTML"),
        (CS_VIS_HTML, "CS_VISUALIZER_HTML"),
        (KB_HTML, "KB_HTML"),
        (JS_CLOCK_RUNTIME, "JS_CLOCK_RUNTIME")
    ]
}

bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct TemplateFeatures: u64 {
        const PROP_FB = 1 << 0;
        const PROP_RA = 1 << 1;
        const PROP_KB = 1 << 2;
        const PROP_RNG = 1 << 21;
        const FN_SEL = 1 << 3;
        const FN_EQZ = 1 << 4;
        const FN_NEZ = 1 << 5;
        const FN_EQ = 1 << 6;
        const FN_NE = 1 << 7;
        const FN_LT = 1 << 8;
        const FN_ADDR16 = 1 << 9;
        const FN_MHALF = 1 << 10;
        const FN_MPAR = 1 << 11;
        const FN_MLO = 1 << 12;
        const FN_MHI = 1 << 13;
        const FN_M16 = 1 << 14;
        const FN_MLOAD = 1 << 15;
        const SP_INDICATOR = 1 << 16;
        const MEM_VISUALIZER = 1 << 17;
        const CS_VISUALIZER = 1 << 18;
        const FN_GE = 1 << 19;
        const FN_INRANGE = 1 << 20;
    }
}

#[derive(Default)]
struct UsageScan {
    uses_getchar: bool,
    uses_random: bool,
    uses_memory_addr: bool,
    uses_memory_load: bool,
    uses_eq: bool,
    uses_ne: bool,
    uses_ge: bool,
    uses_bitcount: bool,
}

#[derive(Clone)]
struct MemStoreByte {
    cell: String,
    parity: String,
    val: String,
    ok: String,
}

#[derive(Clone)]
struct MemStorePair {
    first: MemStoreByte,
    second: Option<MemStoreByte>,
}

#[derive(Clone)]
struct MemRead {
    byte: String,
    ok: String,
}

#[derive(Clone)]
struct CsStore {
    idx: String,
    parity: String,
    val: String,
    ok: String,
}

#[derive(Clone)]
struct CsRead {
    idx: String,
    parity: String,
    ok: String,
}

struct TermResult {
    pc_expr: String,
    trap_expr: String,
    exit_code_expr: Option<String>,
}

struct JsCoprocessorSetup {
    op_code: u8,
    lhs: [String; 4],
    rhs: [String; 4],
}

/// Per-slot constancy info computed during logic emission, consumed by the
/// support emitter to inline literals into the memory-merge helper.
#[derive(Default)]
pub(super) struct MemSlotConsts {
    /// `ms_val[s]` (low byte of slot s) is Some(literal) when every PC writing
    /// to slot s uses the same integer literal. None means the value depends
    /// on PC (or on a non-literal expression).
    pub(super) ms_val: Vec<Option<String>>,
    /// Same as `ms_val` but for the high-byte half of paired stores.
    pub(super) ms_val_b: Vec<Option<String>>,
}

struct Emitter<'a> {
    program: &'a Ir8Program,
    entry_pc: u16,
    js_clock: bool,
    js_coprocessor: bool,
    js_clock_debugger: bool,
    mem_trap: bool,
    cs_trap: bool,
    indicators: bool,
    features: TemplateFeatures,
    memory_end: u32,
    mem_names: Vec<String>,
    mem_init: Vec<u16>,
    cs_names: Vec<String>,
    uses_exceptions: bool,
    uses_exc_payload: bool,
    uses_bitcount: bool,
    max_mem_store_slots: usize,
    max_mem_read_slots: usize,
    max_mem_addr_slots: usize,
    max_cs_store_slots: usize,
    max_cs_read_slots: usize,
    mem_slot_consts: std::cell::OnceCell<MemSlotConsts>,
}

pub fn emit_program(program: &Ir8Program, config: EmitConfig) -> anyhow::Result<String> {
    let emitter = Emitter::new(program, config)?;

    let mut props_css = String::new();
    let mut logic_css = String::new();
    let mut support_css = String::new();

    emitter.emit_properties(&mut props_css);
    emitter.emit_logic(&mut logic_css);
    emitter.emit_support(&mut support_css);

    let mut group_arms = String::new();
    Emitter::dedupe_pc_groups(&mut logic_css, &mut props_css, &mut group_arms);
    logic_css.push_str(&group_arms);

    Emitter::collapse_slot_aliases(&mut logic_css, &mut support_css, &mut props_css);
    inline::inline_slot_indicators(&mut logic_css, &mut support_css);
    inline::fold_value_expressions(&mut logic_css, &mut support_css);
    inline::eliminate_dead_decls(&mut logic_css, &mut support_css, BASE_HTML);

    let html = replace_placeholder_once(BASE_HTML, PROPS_PLACEHOLDER, &props_css)?;
    let html = replace_placeholder_once(&html, LOGIC_PLACEHOLDER, &logic_css)?;
    let html = replace_placeholder_once(&html, SUPPORT_PLACEHOLDER, &support_css)?;
    let terminal_pcs = TrapCode::TERMINAL
        .iter()
        .map(|code| code.pc().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    let html = replace_placeholder_once(&html, TERMINAL_PCS_PLACEHOLDER, &terminal_pcs)?;
    let debugger_cycles = if emitter.js_clock_debugger {
        emitter.build_debugger_cycles_json()
    } else {
        String::from("{}")
    };
    let html = replace_placeholder_once(&html, DEBUGGER_CYCLES_PLACEHOLDER, &debugger_cycles)?;
    let html = emitter.apply_template_features(html)?;
    Ok(compact_style_whitespace(&html))
}

fn compact_style_whitespace(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut search_from = 0;
    while let Some(rel) = s[search_from..].find("<style") {
        let open_tag = search_from + rel;
        let Some(gt_rel) = s[open_tag..].find('>') else {
            break;
        };
        let body_start = open_tag + gt_rel + 1;
        let Some(close_rel) = s[body_start..].find("</style>") else {
            break;
        };
        let body_end = body_start + close_rel;
        out.push_str(&s[search_from..body_start]);
        compact_css_run(&s[body_start..body_end], &mut out);
        search_from = body_end;
    }
    out.push_str(&s[search_from..]);
    out
}

fn compact_css_run(s: &str, out: &mut String) {
    let bytes = s.as_bytes();
    let mut i = 0;
    let mut last_was_ws = false;
    let mut saw_newline = false;
    let mut last_non_ws: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' || b == b'\'' {
            let end = skip_css_string(bytes, i);
            out.push_str(&s[i..end]);
            last_was_ws = false;
            saw_newline = false;
            last_non_ws = Some(bytes[end - 1]);
            i = end;
            continue;
        }
        if b == b' ' || b == b'\t' || b == b'\n' || b == b'\r' {
            saw_newline |= b == b'\n';
            if !last_was_ws {
                let break_safe = matches!(last_non_ws, Some(b';' | b'{' | b'}'));
                if saw_newline && break_safe {
                    out.push('\n');
                } else {
                    out.push(' ');
                }
                last_was_ws = true;
            }
            i += 1;
            continue;
        }
        out.push(b as char);
        last_was_ws = false;
        saw_newline = false;
        last_non_ws = Some(b);
        i += 1;
    }
}

fn json_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn replace_placeholder_once(
    template: &str,
    placeholder: &str,
    replacement: &str,
) -> anyhow::Result<String> {
    let idx = template.find(placeholder).with_context(|| {
        format!(
            "template placeholder {} must appear exactly once (found 0)",
            placeholder
        )
    })?;
    let tail = &template[idx + placeholder.len()..];
    anyhow::ensure!(
        !tail.contains(placeholder),
        "template placeholder {} must appear exactly once",
        placeholder
    );
    let mut out = String::with_capacity(template.len() - placeholder.len() + replacement.len());
    out.push_str(&template[..idx]);
    out.push_str(replacement);
    out.push_str(tail);
    Ok(out)
}

impl<'a> Emitter<'a> {
    fn new(program: &'a Ir8Program, config: EmitConfig) -> anyhow::Result<Self> {
        let entry_pc = program
            .func_entries
            .get(program.entry_func as usize)
            .map(|p| p.index())
            .unwrap_or((program.entry_func as u16) * crate::ir8::PC_STRIDE);

        let memory_end = if config.memory_bytes_cap() == 0 {
            program.memory_end
        } else {
            config.memory_bytes_cap()
        };
        let mem_cells = (memory_end as usize).div_ceil(2);

        let mut mem_init = vec![0u16; mem_cells];
        for (i, cell) in mem_init.iter_mut().enumerate() {
            let lo = program.init_bytes.get(i * 2).copied().unwrap_or(0) as u16;
            let hi = program.init_bytes.get(i * 2 + 1).copied().unwrap_or(0) as u16;
            *cell = lo | (hi << 8);
        }

        let mem_hex_width = Self::cell_offset_hex_width(mem_cells);
        let mem_names = (0..mem_cells)
            .map(|i| Self::cell_name("m", i, mem_hex_width))
            .collect::<Vec<_>>();

        let uses_callstack = Self::scan_uses_callstack(program);
        let (uses_exceptions, uses_exc_payload) = Self::scan_exception_usage(program);
        let cs_names = if uses_callstack {
            let cs_slots = config.callstack_slots_cap();
            let cs_hex_width = Self::cell_offset_hex_width(cs_slots);
            (0..cs_slots)
                .map(|i| Self::cell_name("cs", i, cs_hex_width))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let mut max_mem_store_slots = 0usize;
        let mut max_mem_read_slots = 0usize;
        let mut max_mem_addr_slots = 0usize;
        let mut max_cs_store_slots = 0usize;
        let mut max_cs_read_slots = 0usize;
        for cycle in &program.cycles {
            let mut mem_stores = 0usize;
            let mut mem_reads = 0usize;
            let mut cs_stores = 0usize;
            let mut cs_reads = 0usize;
            let mut addr_counts: HashMap<crate::ir8::Addr, usize> = HashMap::new();
            for op in &cycle.ops {
                match op.kind {
                    Inst8Kind::StoreMem { addr, .. } => {
                        mem_stores += 1;
                        *addr_counts.entry(addr).or_insert(0) += 1;
                    }
                    Inst8Kind::LoadMem { addr, .. } => {
                        mem_reads += 1;
                        *addr_counts.entry(addr).or_insert(0) += 1;
                    }
                    Inst8Kind::CsStore { .. } | Inst8Kind::CsStorePc { .. } => cs_stores += 1,
                    Inst8Kind::CsLoad { .. } | Inst8Kind::CsLoadPc { .. } => cs_reads += 1,
                    _ => {}
                }
            }
            let addr_slots = addr_counts.values().filter(|&&n| n >= 2).count();
            max_mem_store_slots = max_mem_store_slots.max(mem_stores.div_ceil(2));
            max_mem_read_slots = max_mem_read_slots.max(mem_reads);
            max_mem_addr_slots = max_mem_addr_slots.max(addr_slots);
            max_cs_store_slots = max_cs_store_slots.max(cs_stores);
            max_cs_read_slots = max_cs_read_slots.max(cs_reads);
        }
        let usage = Self::scan_usage(program);
        let features = Self::compute_template_features(
            &usage,
            uses_callstack,
            config.visualizers(),
            max_mem_store_slots,
            max_mem_read_slots,
            max_cs_store_slots,
            max_cs_read_slots,
        );

        Ok(Self {
            program,
            entry_pc,
            js_clock: config.js_clock(),
            js_coprocessor: config.js_coprocessor(),
            js_clock_debugger: config.js_clock_debugger(),
            mem_trap: config.mem_trap(),
            cs_trap: config.cs_trap(),
            indicators: config.indicators(),
            features,
            memory_end,
            mem_names,
            mem_init,
            cs_names,
            uses_exceptions,
            uses_exc_payload,
            uses_bitcount: usage.uses_bitcount,
            max_mem_store_slots,
            max_mem_read_slots,
            max_mem_addr_slots,
            max_cs_store_slots,
            max_cs_read_slots,
            mem_slot_consts: std::cell::OnceCell::new(),
        })
    }

    fn uses_callstack(&self) -> bool {
        !self.cs_names.is_empty()
    }

    fn cell_offset_hex_width(cell_count: usize) -> usize {
        let max_offset = cell_count.saturating_sub(1).saturating_mul(2);
        let mut width = 1usize;
        let mut value = max_offset;
        while value >= 16 {
            value /= 16;
            width += 1;
        }
        width
    }

    fn cell_name(prefix: &str, cell_index: usize, hex_width: usize) -> String {
        format!(
            "--{}{offset:0width$x}",
            prefix,
            offset = cell_index * 2,
            width = hex_width
        )
    }

    fn global_name(global_idx: u32) -> String {
        format!("g{}", global_idx)
    }

    fn global_lane_name(global_idx: u32, lane: u8) -> String {
        format!("--{}_{}", Self::global_name(global_idx), lane)
    }

    fn staged_global_lane_name(stage: u8, global_idx: u32, lane: u8) -> String {
        format!("--_{}{}_{}", stage, Self::global_name(global_idx), lane)
    }

    fn scan_uses_callstack(program: &Ir8Program) -> bool {
        program.cycles.iter().any(|cycle| {
            cycle.ops.iter().any(|op| {
                matches!(
                    op.kind,
                    Inst8Kind::CsStore { .. }
                        | Inst8Kind::CsLoad { .. }
                        | Inst8Kind::CsStorePc { .. }
                        | Inst8Kind::CsLoadPc { .. }
                        | Inst8Kind::CsAlloc(_)
                        | Inst8Kind::CsFree(_)
                )
            })
        })
    }

    fn scan_exception_usage(program: &Ir8Program) -> (bool, bool) {
        let mut uses = false;
        let mut payload = false;
        for cycle in &program.cycles {
            for op in &cycle.ops {
                match op.kind {
                    Inst8Kind::ExcPayloadSet { .. } | Inst8Kind::ExcPayloadGet { .. } => {
                        uses = true;
                        payload = true;
                    }
                    Inst8Kind::ExcFlagSet { .. }
                    | Inst8Kind::ExcFlagGet
                    | Inst8Kind::ExcTagSet { .. }
                    | Inst8Kind::ExcTagGet { .. } => uses = true,
                    _ => {}
                }
                if uses && payload {
                    return (true, true);
                }
            }
        }
        (uses, payload)
    }

    fn scan_usage(program: &Ir8Program) -> UsageScan {
        let mut usage = UsageScan::default();
        for cycle in &program.cycles {
            for op in &cycle.ops {
                match &op.kind {
                    Inst8Kind::Getchar => {
                        usage.uses_getchar = true;
                        usage.uses_ne = true;
                    }
                    Inst8Kind::RandomByte { .. } => {
                        usage.uses_random = true;
                    }
                    Inst8Kind::LoadMem { .. } => {
                        usage.uses_memory_addr = true;
                        usage.uses_memory_load = true;
                    }
                    Inst8Kind::StoreMem { .. } => usage.uses_memory_addr = true,
                    Inst8Kind::Eq(_, _) => usage.uses_eq = true,
                    Inst8Kind::Ne(_, _) => usage.uses_ne = true,
                    Inst8Kind::GeU(_, _) => usage.uses_ge = true,
                    _ => {}
                }
            }
            if let Terminator8::CallSetup {
                callee_entry: CallTarget::Builtin(BuiltinId::Clz32 | BuiltinId::Ctz32),
                ..
            } = cycle.terminator
            {
                usage.uses_ne = true;
                usage.uses_bitcount = true;
            }
            if matches!(cycle.terminator, Terminator8::Switch { .. }) {
                usage.uses_eq = true;
            }
        }
        usage
    }

    fn compute_template_features(
        usage: &UsageScan,
        uses_callstack: bool,
        visualizers: bool,
        max_mem_store_slots: usize,
        max_mem_read_slots: usize,
        max_cs_store_slots: usize,
        max_cs_read_slots: usize,
    ) -> TemplateFeatures {
        let mut features = TemplateFeatures::PROP_FB
            | TemplateFeatures::PROP_RA
            | TemplateFeatures::FN_SEL
            | TemplateFeatures::FN_LT
            | TemplateFeatures::FN_MLO
            | TemplateFeatures::FN_MHI
            | TemplateFeatures::FN_M16;

        if visualizers {
            features |= TemplateFeatures::MEM_VISUALIZER;
        }

        if usage.uses_getchar {
            features |= TemplateFeatures::PROP_KB;
        }
        if usage.uses_random {
            features |= TemplateFeatures::PROP_RNG;
        }
        if usage.uses_memory_addr {
            features |= TemplateFeatures::FN_ADDR16
                | TemplateFeatures::FN_MHALF
                | TemplateFeatures::FN_MPAR
                | TemplateFeatures::FN_GE;
        }
        if usage.uses_memory_load {
            features |= TemplateFeatures::FN_MLOAD
                | TemplateFeatures::FN_MHALF
                | TemplateFeatures::FN_MPAR
                | TemplateFeatures::FN_EQZ
                | TemplateFeatures::FN_NEZ;
        }
        if usage.uses_eq
            || max_mem_store_slots > 0
            || max_mem_read_slots > 0
            || (uses_callstack && (max_cs_store_slots > 0 || max_cs_read_slots > 0))
        {
            features |= TemplateFeatures::FN_EQ | TemplateFeatures::FN_EQZ;
        }
        if usage.uses_ne {
            features |=
                TemplateFeatures::FN_NE | TemplateFeatures::FN_NEZ | TemplateFeatures::FN_EQZ;
        }
        if usage.uses_ge {
            features |= TemplateFeatures::FN_GE;
        }
        if uses_callstack {
            features |= TemplateFeatures::SP_INDICATOR;
            if visualizers {
                features |= TemplateFeatures::CS_VISUALIZER;
            }
            features |= TemplateFeatures::FN_GE;
            features |= TemplateFeatures::FN_INRANGE;
        }

        features
    }

    fn apply_marked_section(
        out: &mut String,
        start: &str,
        end: &str,
        keep: bool,
    ) -> anyhow::Result<()> {
        fn marker_line_bounds(s: &str, marker_idx: usize) -> (usize, usize) {
            let line_start = s[..marker_idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let line_end = s[marker_idx..]
                .find('\n')
                .map(|i| marker_idx + i + 1)
                .unwrap_or(s.len());
            (line_start, line_end)
        }

        let start_count = out.match_indices(start).count();
        let end_count = out.match_indices(end).count();
        anyhow::ensure!(
            start_count == end_count,
            "template marker counts differ for {} and {} ({} vs {})",
            start,
            end,
            start_count,
            end_count
        );

        while let Some(start_idx) = out.find(start) {
            let after_start = start_idx + start.len();
            let rel_end_idx = out[after_start..].find(end).with_context(|| {
                format!("template marker {} is missing matching {}", start, end)
            })?;
            let end_idx = after_start + rel_end_idx;
            let (start_line_start, start_line_end) = marker_line_bounds(out, start_idx);
            let (end_line_start, end_line_end) = marker_line_bounds(out, end_idx);
            if keep {
                out.replace_range(end_line_start..end_line_end, "");
                out.replace_range(start_line_start..start_line_end, "");
            } else {
                out.replace_range(start_line_start..end_line_end, "");
            }
        }
        Ok(())
    }

    fn build_debugger_cycles_json(&self) -> String {
        let mut out = String::from("{");
        for (i, cycle) in self.program.cycles.iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            let _ = write!(out, "\"{}\":[", cycle.pc.index());
            for op in &cycle.ops {
                let _ = write!(out, "{},", json_string(&crate::print::format_inst8(op)));
            }
            let _ = write!(
                out,
                "{}]",
                json_string(&crate::print::format_term8(&cycle.terminator))
            );
        }
        out.push('}');
        out
    }

    fn apply_template_features(&self, mut html: String) -> anyhow::Result<String> {
        macro_rules! section {
            ($keep:expr, $stem:ident) => {
                paste! {
                    Self::apply_marked_section(
                        &mut html,
                        [<KEEP_ $stem _START>],
                        [<KEEP_ $stem _END>],
                        $keep,
                    )?;
                }
            };
        }
        let f = &self.features;
        section!(f.contains(TemplateFeatures::PROP_FB), PROP_FB);
        section!(f.contains(TemplateFeatures::PROP_RA), PROP_RA);
        section!(f.contains(TemplateFeatures::PROP_KB), PROP_KB);
        section!(f.contains(TemplateFeatures::PROP_RNG), PROP_RNG);
        section!(f.contains(TemplateFeatures::FN_SEL), FN_SEL);
        section!(f.contains(TemplateFeatures::FN_EQZ), FN_EQZ);
        section!(f.contains(TemplateFeatures::FN_NEZ), FN_NEZ);
        section!(f.contains(TemplateFeatures::FN_EQ), FN_EQ);
        section!(f.contains(TemplateFeatures::FN_NE), FN_NE);
        section!(f.contains(TemplateFeatures::FN_LT), FN_LT);
        section!(f.contains(TemplateFeatures::FN_GE), FN_GE);
        section!(f.contains(TemplateFeatures::FN_INRANGE), FN_INRANGE);
        section!(f.contains(TemplateFeatures::FN_ADDR16), FN_ADDR16);
        section!(f.contains(TemplateFeatures::FN_MHALF), FN_MHALF);
        section!(f.contains(TemplateFeatures::FN_MPAR), FN_MPAR);
        section!(f.contains(TemplateFeatures::FN_MLO), FN_MLO);
        section!(f.contains(TemplateFeatures::FN_MHI), FN_MHI);
        section!(f.contains(TemplateFeatures::FN_M16), FN_M16);
        section!(f.contains(TemplateFeatures::FN_MLOAD), FN_MLOAD);
        section!(self.indicators, INDS_CSS);
        section!(self.indicators, INDS_HTML);
        section!(
            self.indicators && f.contains(TemplateFeatures::SP_INDICATOR),
            SP_IND_CSS
        );
        section!(
            self.indicators && f.contains(TemplateFeatures::SP_INDICATOR),
            SP_IND_HTML
        );
        section!(f.contains(TemplateFeatures::MEM_VISUALIZER), MEM_VIS_CSS);
        section!(f.contains(TemplateFeatures::MEM_VISUALIZER), MEM_VIS_HTML);
        section!(f.contains(TemplateFeatures::CS_VISUALIZER), CS_VIS_CSS);
        section!(f.contains(TemplateFeatures::CS_VISUALIZER), CS_VIS_HTML);
        section!(f.contains(TemplateFeatures::PROP_KB), KB_CSS);
        section!(f.contains(TemplateFeatures::PROP_KB), KB_HINT_CSS);
        section!(f.contains(TemplateFeatures::PROP_KB), KB_CLK_DEFAULT);
        section!(f.contains(TemplateFeatures::PROP_KB), KB_INPUT_CSS);
        section!(f.contains(TemplateFeatures::PROP_KB), KB_HTML);
        section!(self.js_coprocessor, JS_COPROCESSOR_RUNTIME);
        section!(self.js_clock, JS_CLOCK_RUNTIME);
        section!(self.js_clock_debugger, JS_CLOCK_DEBUGGER_CSS);
        section!(self.js_clock_debugger, JS_CLOCK_DEBUGGER_RUNTIME);
        section!(
            self.js_clock_debugger && f.contains(TemplateFeatures::PROP_RNG),
            DEBUGGER_RNG
        );
        Ok(html)
    }
}
