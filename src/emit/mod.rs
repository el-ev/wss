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

use bitflags::bitflags;
use paste::paste;

#[cfg(test)]
use crate::constants::{
    DEFAULT_CALLSTACK_SLOTS_CAP, DEFAULT_JS_CLOCK_DEBUGGER_ENABLED, DEFAULT_JS_CLOCK_ENABLED,
    DEFAULT_JS_COPROCESSOR_ENABLED, DEFAULT_MEMORY_BYTES_CAP,
};
use crate::ir8::{BuiltinId, CallTarget, Inst8Kind, Ir8Program, Terminator8, TrapCode, Val8, Word};

const BASE_HTML: &str = include_str!("base.html");
const PROPS_PLACEHOLDER: &str = "/*__WSS_PROPS__*/";
const LOGIC_PLACEHOLDER: &str = "/*__WSS_LOGIC__*/";
const SUPPORT_PLACEHOLDER: &str = "/*__WSS_SUPPORT__*/";
const READ_LOOKUP_CHUNK: usize = 128;
const VIS_SHADOW_CHUNK: usize = 8;
const VIS_COLS: usize = 128;
const MEM_DIRTY_PAGE_CELLS: usize = 16;
const CALLSTACK_DIRTY_PAGE_CELLS: usize = 16;
const ACTIVE_FLAG_ARMS_CHUNK: usize = 64;
const COP_OP_NONE: u8 = 0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmitConfig {
    memory_bytes_cap: u32,
    callstack_slots_cap: usize,
    js_clock: bool,
    js_coprocessor: bool,
    js_clock_debugger: bool,
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
        )
        .expect("default emit config should be valid")
    }
}

impl EmitConfig {
    pub fn new(
        memory_bytes_cap: u32,
        callstack_slots_cap: usize,
        js_clock: bool,
        js_coprocessor: bool,
        js_clock_debugger: bool,
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

    #[cfg(test)]
    pub fn with_memory_bytes_cap(self, memory_bytes_cap: u32) -> anyhow::Result<Self> {
        Self::new(
            memory_bytes_cap,
            self.callstack_slots_cap(),
            self.js_clock(),
            self.js_coprocessor(),
            self.js_clock_debugger(),
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
        (FN_SEL, "FN_SEL"),
        (FN_EQZ, "FN_EQZ"),
        (FN_NEZ, "FN_NEZ"),
        (FN_EQ, "FN_EQ"),
        (FN_NE, "FN_NE"),
        (FN_LT, "FN_LT"),
        (FN_ADDR16, "FN_ADDR16"),
        (FN_MHALF, "FN_MHALF"),
        (FN_MPAR, "FN_MPAR"),
        (FN_MLO, "FN_MLO"),
        (FN_MHI, "FN_MHI"),
        (FN_M16, "FN_M16"),
        (FN_MLOAD, "FN_MLOAD"),
        (SP_IND_CSS, "SP_INDICATOR_CSS"),
        (MEM_VIS_CSS, "MEM_VISUALIZER_CSS"),
        (CS_VIS_CSS, "CS_VISUALIZER_CSS"),
        (KB_CSS, "KB_CSS"),
        (KB_HINT_CSS, "KB_HINT_CSS"),
        (KB_CLK_DEFAULT, "KB_CLK_DEFAULT"),
        (KB_INPUT_CSS, "KB_INPUT_CSS"),
        (JS_CLOCK_DEBUGGER_CSS, "JS_CLOCK_DEBUGGER_CSS"),
        (JS_CLOCK_DEBUGGER_RUNTIME, "JS_CLOCK_DEBUGGER_RUNTIME"),
        (JS_COPROCESSOR_RUNTIME, "JS_COPROCESSOR_RUNTIME")
    ],
    html: [
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
    }
}

#[derive(Default)]
struct UsageScan {
    uses_getchar: bool,
    uses_memory_addr: bool,
    uses_memory_load: bool,
    uses_eq: bool,
    uses_ne: bool,
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

struct Emitter<'a> {
    program: &'a Ir8Program,
    entry_pc: u16,
    js_clock: bool,
    js_coprocessor: bool,
    js_clock_debugger: bool,
    features: TemplateFeatures,
    memory_end: u32,
    mem_names: Vec<String>,
    mem_init: Vec<u16>,
    uses_callstack: bool,
    cs_names: Vec<String>,
    global_count: u32,
    global_init: Vec<u32>,
    max_mem_store_slots: usize,
    max_mem_read_slots: usize,
    max_cs_store_slots: usize,
    max_cs_read_slots: usize,
}

pub fn emit_program(program: &Ir8Program, config: EmitConfig) -> anyhow::Result<String> {
    let emitter = Emitter::new(program, config)?;

    let mut props_css = String::new();
    let mut logic_css = String::new();
    let mut support_css = String::new();

    emitter.emit_properties(&mut props_css);
    emitter.emit_logic(&mut logic_css);
    emitter.emit_support(&mut support_css);

    let html = BASE_HTML
        .replace(PROPS_PLACEHOLDER, &props_css)
        .replace(LOGIC_PLACEHOLDER, &logic_css)
        .replace(SUPPORT_PLACEHOLDER, &support_css);
    Ok(emitter.apply_template_features(html))
}

impl<'a> Emitter<'a> {
    fn new(program: &'a Ir8Program, config: EmitConfig) -> anyhow::Result<Self> {
        let entry_pc = (program.entry_func as u16) * crate::ir8::PC_STRIDE;

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
        let cs_names = if uses_callstack {
            let cs_slots = config.callstack_slots_cap();
            let cs_hex_width = Self::cell_offset_hex_width(cs_slots);
            (0..cs_slots)
                .map(|i| Self::cell_name("cs", i, cs_hex_width))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };

        let global_count = Self::scan_global_count(program).max(program.global_init.len() as u32);
        let mut global_init = program.global_init.clone();
        global_init.resize(global_count as usize, 0);

        let mut max_mem_store_slots = 0usize;
        let mut max_mem_read_slots = 0usize;
        let mut max_cs_store_slots = 0usize;
        let mut max_cs_read_slots = 0usize;
        for cycle in &program.cycles {
            let mut mem_stores = 0usize;
            let mut mem_reads = 0usize;
            let mut cs_stores = 0usize;
            let mut cs_reads = 0usize;
            for op in &cycle.ops {
                match op.kind {
                    Inst8Kind::StoreMem { .. } => mem_stores += 1,
                    Inst8Kind::LoadMem { .. } => mem_reads += 1,
                    Inst8Kind::CsStore { .. } | Inst8Kind::CsStorePc { .. } => cs_stores += 1,
                    Inst8Kind::CsLoad { .. } | Inst8Kind::CsLoadPc { .. } => cs_reads += 1,
                    _ => {}
                }
            }
            max_mem_store_slots = max_mem_store_slots.max(mem_stores.div_ceil(2));
            max_mem_read_slots = max_mem_read_slots.max(mem_reads);
            max_cs_store_slots = max_cs_store_slots.max(cs_stores);
            max_cs_read_slots = max_cs_read_slots.max(cs_reads);
        }
        let usage = Self::scan_usage(program);
        let features = Self::compute_template_features(
            &usage,
            uses_callstack,
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
            features,
            memory_end,
            mem_names,
            mem_init,
            uses_callstack,
            cs_names,
            global_count,
            global_init,
            max_mem_store_slots,
            max_mem_read_slots,
            max_cs_store_slots,
            max_cs_read_slots,
        })
    }

    fn scan_global_count(program: &Ir8Program) -> u32 {
        let mut max_idx = 0u32;
        let mut seen = false;
        for cycle in &program.cycles {
            for op in &cycle.ops {
                match op.kind {
                    Inst8Kind::GlobalGetByte { global_idx, .. }
                    | Inst8Kind::GlobalSetByte { global_idx, .. } => {
                        seen = true;
                        max_idx = max_idx.max(global_idx);
                    }
                    _ => {}
                }
            }
        }
        if seen { max_idx + 1 } else { 0 }
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

    fn scan_usage(program: &Ir8Program) -> UsageScan {
        let mut usage = UsageScan::default();
        for cycle in &program.cycles {
            for op in &cycle.ops {
                match &op.kind {
                    Inst8Kind::Getchar => {
                        usage.uses_getchar = true;
                        usage.uses_ne = true;
                    }
                    Inst8Kind::LoadMem { .. } => {
                        usage.uses_memory_addr = true;
                        usage.uses_memory_load = true;
                    }
                    Inst8Kind::StoreMem { .. } => usage.uses_memory_addr = true,
                    Inst8Kind::Eq(_, _) => usage.uses_eq = true,
                    Inst8Kind::Ne(_, _) => usage.uses_ne = true,
                    _ => {}
                }
            }
            if let Terminator8::CallSetup {
                callee_entry: CallTarget::Builtin(BuiltinId::Clz32 | BuiltinId::Ctz32),
                ..
            } = cycle.terminator
            {
                usage.uses_ne = true;
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

        features |= TemplateFeatures::MEM_VISUALIZER;

        if usage.uses_getchar {
            features |= TemplateFeatures::PROP_KB;
        }
        if usage.uses_memory_addr {
            features |= TemplateFeatures::FN_ADDR16
                | TemplateFeatures::FN_MHALF
                | TemplateFeatures::FN_MPAR;
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
            features |= TemplateFeatures::FN_NE | TemplateFeatures::FN_NEZ;
        }
        if uses_callstack {
            features |= TemplateFeatures::SP_INDICATOR;
            features |= TemplateFeatures::CS_VISUALIZER;
        }

        features
    }

    fn apply_marked_section(out: &mut String, start: &str, end: &str, keep: bool) {
        fn marker_line_bounds(s: &str, marker_idx: usize) -> (usize, usize) {
            let line_start = s[..marker_idx].rfind('\n').map(|i| i + 1).unwrap_or(0);
            let line_end = s[marker_idx..]
                .find('\n')
                .map(|i| marker_idx + i + 1)
                .unwrap_or(s.len());
            (line_start, line_end)
        }

        while let Some(start_idx) = out.find(start) {
            let after_start = start_idx + start.len();
            let Some(rel_end_idx) = out[after_start..].find(end) else {
                break;
            };
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
    }

    fn apply_template_features(&self, mut html: String) -> String {
        macro_rules! section {
            ($keep:expr, $stem:ident) => {
                paste! {
                    Self::apply_marked_section(
                        &mut html,
                        [<KEEP_ $stem _START>],
                        [<KEEP_ $stem _END>],
                        $keep,
                    );
                }
            };
        }
        let f = &self.features;
        section!(f.contains(TemplateFeatures::PROP_FB), PROP_FB);
        section!(f.contains(TemplateFeatures::PROP_RA), PROP_RA);
        section!(f.contains(TemplateFeatures::PROP_KB), PROP_KB);
        section!(f.contains(TemplateFeatures::FN_SEL), FN_SEL);
        section!(f.contains(TemplateFeatures::FN_EQZ), FN_EQZ);
        section!(f.contains(TemplateFeatures::FN_NEZ), FN_NEZ);
        section!(f.contains(TemplateFeatures::FN_EQ), FN_EQ);
        section!(f.contains(TemplateFeatures::FN_NE), FN_NE);
        section!(f.contains(TemplateFeatures::FN_LT), FN_LT);
        section!(f.contains(TemplateFeatures::FN_ADDR16), FN_ADDR16);
        section!(f.contains(TemplateFeatures::FN_MHALF), FN_MHALF);
        section!(f.contains(TemplateFeatures::FN_MPAR), FN_MPAR);
        section!(f.contains(TemplateFeatures::FN_MLO), FN_MLO);
        section!(f.contains(TemplateFeatures::FN_MHI), FN_MHI);
        section!(f.contains(TemplateFeatures::FN_M16), FN_M16);
        section!(f.contains(TemplateFeatures::FN_MLOAD), FN_MLOAD);
        section!(f.contains(TemplateFeatures::SP_INDICATOR), SP_IND_CSS);
        section!(f.contains(TemplateFeatures::SP_INDICATOR), SP_IND_HTML);
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
        html
    }
}
