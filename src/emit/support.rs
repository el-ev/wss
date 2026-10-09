use super::*;

const SLOT_ALIAS_PREFIXES: &[&str] = &["cri", "cro", "csi", "cso", "mri", "mro"];

/// Parses ` --{prefix}N: var(--{prefix}T);` (a slot-indicator alias) into
/// `(alias_name, canonical_name)`. Returns `None` for any other shape.
fn parse_slot_alias_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    let body = trimmed.strip_suffix(';')?.trim_end();
    let (name_with_dashes, value) = body.split_once(": ")?;
    let alias_full = name_with_dashes.trim();
    let alias = alias_full.strip_prefix("--")?;
    if !SLOT_ALIAS_PREFIXES.iter().any(|p| {
        alias
            .strip_prefix(p)
            .is_some_and(|r| r.bytes().all(|b| b.is_ascii_alphanumeric()))
    }) {
        return None;
    }
    let inner = value.trim().strip_prefix("var(")?.strip_suffix(')')?.trim();
    if !inner.starts_with("--") {
        return None;
    }
    Some((alias_full.to_string(), inner.to_string()))
}

/// Extracts the property name from an `@property --foo { ... }` line.
fn parse_property_name(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    let rest = trimmed.strip_prefix("@property ")?;
    let end = rest.find(|c: char| c.is_whitespace() || c == '{')?;
    Some(&rest[..end])
}

impl<'a> Emitter<'a> {
    pub(super) fn emit_support(&self, out: &mut String) {
        fn terminal_codes(
            uses_callstack: bool,
            mem_trap: bool,
            cs_trap: bool,
        ) -> impl Iterator<Item = TrapCode> {
            TrapCode::TERMINAL
                .into_iter()
                .filter(move |code| match code {
                    TrapCode::CallstackOverflow => uses_callstack && cs_trap,
                    TrapCode::InvalidMemoryAccess => mem_trap,
                    _ => true,
                })
        }

        self.emit_i2char(out);
        self.emit_cond_char(out);
        self.emit_hex_digit(out);
        if self.uses_bitcount {
            self.emit_byte_clz_lookup(out);
            self.emit_byte_ctz_lookup(out);
        }

        // Loads read memory as of the end of the previous cycle: the store
        // snapshot, which is also the merge's `prev`. The scheduler relies
        // on this when it lets a load share a cycle with a later store to
        // the same byte (the load must see the old value). The execute
        // snapshot (`--_0m*`) would not do: it is captured from the cells
        // in the same recalc that merges this cycle's stores, so a load
        // would see them. Increments read the same stage, which also keeps
        // their merge free of a dependency cycle.
        let mem_reads = (0..self.mem_names.len())
            .map(|i| (i, self.mem_prev_expr(i)))
            .collect::<Vec<_>>();
        Self::emit_chunked_read_function(out, "--read_m16", &mem_reads);

        if self.uses_callstack() {
            let cs_reads = self
                .cs_names
                .iter()
                .enumerate()
                .map(|(i, name)| (i, format!("var({})", Self::shadow_name(1, name))))
                .collect::<Vec<_>>();
            Self::emit_chunked_read_function(out, "--read_cs", &cs_reads);
        }

        if let Some(merge_fn) = self.mem_merge_fn.get() {
            out.push_str(merge_fn);
        }
        if self.uses_callstack() && self.max_cs_store_slots > 0 {
            self.emit_cs_dispatch_merge_function(out);
        }
        self.emit_keyframes(out);
        if self.features.contains(TemplateFeatures::MEM_VISUALIZER) {
            self.emit_memory_visualizer(out);
        }
        if self.features.contains(TemplateFeatures::CS_VISUALIZER) {
            self.emit_callstack_visualizer(out);
        }

        let codes: Vec<TrapCode> =
            terminal_codes(self.uses_callstack(), self.mem_trap, self.cs_trap).collect();
        out.push_str(".screen::after { white-space: pre-wrap; word-break: break-all; ");
        out.push_str("content: if(");
        for code in &codes {
            if *code == TrapCode::Exited {
                let _ = write!(
                    out,
                    "style(--pc: {}): var(--fb, \"\") \"\\a[Program exited with code \" var(--ra, \"{}\") \"]\"; ",
                    code.pc(),
                    DEFAULT_RA_DISPLAY
                );
                continue;
            }
            let _ = write!(
                out,
                "style(--pc: {}): var(--fb, \"\") \"\\a[{}]\"; ",
                code.pc(),
                code.screen_label().unwrap_or("")
            );
        }
        out.push_str("else: var(--fb, \"\")); ");
        out.push_str("color: if(");
        for code in &codes {
            if *code == TrapCode::Exited {
                continue;
            }
            let _ = write!(out, "style(--pc: {}): #d00; ", code.pc());
        }
        out.push_str("else: #222); display: block; flex: 0 0 auto; }\n");

        if !codes.is_empty() {
            let cond = codes
                .iter()
                .map(|code| format!("style(--pc: {})", code.pc()))
                .collect::<Vec<_>>()
                .join(" or ");
            let _ = writeln!(
                out,
                ".clk:has(.terminal) {{ @container {} {{ .terminal {{ animation-play-state: paused, paused !important; }} }} }}",
                cond
            );
        }
    }

    fn emit_cond_char(&self, out: &mut String) {
        out.push_str(
            "@function --cond_char(--c <integer>, --v <integer>) returns <string> { result: if(style(--c: 0): \"\"; else: --i2char(var(--v))); }\n",
        );
    }

    fn emit_i2char(&self, out: &mut String) {
        out.push_str("@function --i2char(--i <integer>) returns <string> { result: if(");
        out.push_str("style(--i: -1): \"\"; style(--i: 8): \"\"; style(--i: 9): \"    \"; style(--i: 10): \"\\a \"; ");
        for code in 32u8..=126u8 {
            let ch = code as char;
            let lit = match ch {
                '"' => "\\\"".to_string(),
                '\\' => "\\\\".to_string(),
                _ => ch.to_string(),
            };
            let _ = write!(out, "style(--i: {}): \"{}\"; ", code, lit);
        }
        out.push_str("else: \"?\"); }\n");
    }

    fn emit_hex_digit(&self, out: &mut String) {
        out.push_str("@function --hex(--i <integer>) returns <string> { result: if(");
        for v in 0..=15u8 {
            let _ = write!(out, "style(--i: {}): \"{:x}\"; ", v, v);
        }
        out.push_str("else: \"0\"); }\n");
    }

    fn emit_byte_lookup(out: &mut String, name: &str, f: impl Fn(u8) -> u8) {
        let reads = (0u16..=255u16)
            .map(|v| {
                let byte = v as u8;
                (v as usize, f(byte).to_string())
            })
            .collect::<Vec<_>>();
        Self::emit_chunked_read_function(out, name, &reads);
    }

    fn emit_byte_clz_lookup(&self, out: &mut String) {
        Self::emit_byte_lookup(out, "--byte_clz", |b| {
            if b == 0 { 8 } else { b.leading_zeros() as u8 }
        });
    }

    fn emit_byte_ctz_lookup(&self, out: &mut String) {
        Self::emit_byte_lookup(out, "--byte_ctz", |b| {
            if b == 0 { 8 } else { b.trailing_zeros() as u8 }
        });
    }

    pub(super) fn shadow_name(stage: u8, base: &str) -> String {
        let tail = base.strip_prefix("--").unwrap_or(base);
        format!("--_{}{}", stage, tail)
    }

    /// Memory cell `i` as of the last store: the store snapshot, or the
    /// initial value before the first store. Read in place rather than via
    /// a `--_1m*` alias: one declaration fewer per cell on every recalc.
    pub(super) fn mem_prev_expr(&self, i: usize) -> String {
        format!(
            "var({}, {})",
            Self::shadow_name(2, &self.mem_names[i]),
            self.mem_init[i]
        )
    }

    /// Emits `style(--i: A) or style(--i: B): V; style(--i: C): W; ...`,
    /// grouping consecutive arms with byte-identical values under a single
    /// `or` chain. Skips entries whose value is `"0"` (caller emits an
    /// `else: 0` arm). Preserves first-occurrence order across distinct
    /// values so output is stable.
    pub(super) fn emit_grouped_style_arms(out: &mut String, arms: &[(usize, String)]) {
        let mut order: Vec<String> = Vec::new();
        let mut by_val: std::collections::HashMap<String, Vec<usize>> =
            std::collections::HashMap::new();
        for (idx, val) in arms {
            let v = val.trim();
            if v == "0" {
                continue;
            }
            if !by_val.contains_key(v) {
                order.push(v.to_string());
            }
            by_val.entry(v.to_string()).or_default().push(*idx);
        }
        for val in &order {
            let idxs = &by_val[val];
            for (i, idx) in idxs.iter().enumerate() {
                if i != 0 {
                    out.push_str(" or ");
                }
                let _ = write!(out, "style(--i: {})", idx);
            }
            out.push_str(": ");
            out.push_str(val);
            out.push_str("; ");
        }
    }

    pub(super) fn emit_chunked_read_function(
        out: &mut String,
        func: &str,
        reads: &[(usize, String)],
    ) {
        if reads.is_empty() {
            let _ = writeln!(
                out,
                "@function {}(--i <integer>) returns <integer> {{ result: 0; }}",
                func
            );
            return;
        }
        let chunk_count = reads.len().div_ceil(READ_LOOKUP_CHUNK);
        let chunked: Vec<&[(usize, String)]> = reads.chunks(READ_LOOKUP_CHUNK).collect();
        let live_idx: Vec<usize> = (0..chunk_count)
            .filter(|c| !chunked[*c].iter().all(|(_, v)| v.trim() == "0"))
            .collect();

        if live_idx.len() == 1 {
            let arms = chunked[live_idx[0]];
            let _ = write!(
                out,
                "@function {}(--i <integer>) returns <integer> {{ result: if(",
                func
            );
            Self::emit_grouped_style_arms(out, arms);
            out.push_str("else: 0); }\n");
            return;
        }
        if live_idx.is_empty() {
            let _ = writeln!(
                out,
                "@function {}(--i <integer>) returns <integer> {{ result: 0; }}",
                func
            );
            return;
        }
        for chunk_idx in &live_idx {
            let chunk_reads = chunked[*chunk_idx];
            let _ = write!(
                out,
                "@function {}_{}(--i <integer>) returns <integer> {{ result: if(",
                func, chunk_idx
            );
            Self::emit_grouped_style_arms(out, chunk_reads);
            out.push_str("else: 0); }\n");
        }
        // Two-level lookup: pick the chunk from the index, then test only
        // that chunk's arms (instead of every arm of every chunk).
        let _ = writeln!(
            out,
            "@function {func}(--i <integer>) returns <integer> {{ result: {func}_d(round(down, var(--i) / {READ_LOOKUP_CHUNK}), var(--i)); }}"
        );
        let _ = write!(
            out,
            "@function {}_d(--c <integer>, --i <integer>) returns <integer> {{ result: if(",
            func
        );
        for chunk in &live_idx {
            let _ = write!(out, "style(--c: {chunk}): {func}_{chunk}(var(--i)); ");
        }
        out.push_str("else: 0); }\n");
    }

    pub(super) fn emit_chunked_shadow(
        out: &mut String,
        selector: &str,
        pseudo: &str,
        var_prefix: &str,
        entries: &[String],
    ) {
        if entries.is_empty() {
            let _ = writeln!(
                out,
                "{}{} {{ box-shadow: 0 0 transparent; }}",
                selector, pseudo
            );
            return;
        }
        let chunk_count = entries.len().div_ceil(VIS_SHADOW_CHUNK);
        for (chunk, chunk_entries) in entries.chunks(VIS_SHADOW_CHUNK).enumerate() {
            let value = chunk_entries.join(",");
            let _ = writeln!(
                out,
                "{} {{ --{}-{}: {}; }}",
                selector, var_prefix, chunk, value
            );
        }
        let _ = write!(out, "{}{} {{ box-shadow: ", selector, pseudo);
        for chunk in 0..chunk_count {
            let _ = write!(out, "var(--{}-{}),", var_prefix, chunk);
        }
        out.push_str("0 0 transparent; }\n");
    }

    pub(super) fn vis_slot_shadow_entry(
        idx_expr: &str,
        cols: usize,
        rgba: &str,
        alpha_expr: &str,
    ) -> String {
        let x = format!("calc((mod(({}), {}) * 8px) + 8px)", idx_expr, cols);
        let y = format!(
            "calc((round(down, calc(({}) / {})) * 8px) + 8px)",
            idx_expr, cols
        );
        format!("{} {} rgba({}, {})", x, y, rgba, alpha_expr)
    }

    /// The gray pixel for byte expression `byte`. Lightness takes the byte
    /// once, where `rgb(b, b, b)` would evaluate it three times: the pixels
    /// are most of a recalc's work, every cycle.
    fn vis_gray(byte: &str) -> String {
        format!("hsl(0 0% calc({} * 100% / 255))", byte)
    }

    pub(super) fn emit_memory_visualizer(&self, out: &mut String) {
        let mem_cols = VIS_COLS;
        let mem_rows = (self.memory_end as usize).div_ceil(mem_cols).max(1);
        let _ = writeln!(
            out,
            ".memvis {{ width: {}px; height: {}px; }}",
            mem_cols * 8,
            mem_rows * 8
        );
        out.push_str(".memvis::before { content: \"\"; position: absolute; left: -8px; top: -8px; width: 8px; height: 8px; background: transparent; pointer-events: none; z-index: 2; }\n");
        out.push_str(".memvis::after { z-index: 1; }\n");
        let mem_entries = (0..(self.memory_end as usize))
            .map(|i| {
                let x = (i % mem_cols) * 8 + 8;
                let y = (i / mem_cols) * 8 + 8;
                let name = &self.mem_names[i / 2];
                let byte = if i % 2 == 0 {
                    format!("--mlo(var({}))", name)
                } else {
                    format!("--mhi(var({}))", name)
                };
                format!("{}px {}px {}", x, y, Self::vis_gray(&byte))
            })
            .collect::<Vec<_>>();
        Self::emit_chunked_shadow(out, ".memvis", "::after", "mv", &mem_entries);

        let mut hl_entries = Vec::with_capacity(
            self.max_mem_read_slots
                .saturating_add(self.max_mem_store_slots * 2),
        );
        hl_entries.extend((0..self.max_mem_read_slots).map(|s| {
            Self::vis_slot_shadow_entry(
                &format!("var(--mri{})", s),
                mem_cols,
                "48, 170, 84",
                &format!("calc(0.72 * var(--mro{}))", s),
            )
        }));
        hl_entries.extend((0..self.max_mem_store_slots).flat_map(|s| {
            ["", "b"].into_iter().map(move |suffix| {
                Self::vis_slot_shadow_entry(
                    &format!(
                        "calc((var(--msc{}{}) * 2) + var(--msp{}{}))",
                        s, suffix, s, suffix
                    ),
                    mem_cols,
                    "38, 112, 255",
                    &format!("calc(0.88 * var(--mso{}{}))", s, suffix),
                )
            })
        }));
        Self::emit_chunked_shadow(out, ".memvis", "::before", "mvh", &hl_entries);
    }

    pub(super) fn emit_callstack_visualizer(&self, out: &mut String) {
        let cs_cols = VIS_COLS;
        let cs_bytes = self.cs_names.len() * 2;
        let cs_rows = cs_bytes.div_ceil(cs_cols).max(1);
        let _ = writeln!(
            out,
            ".csvis {{ width: {}px; height: {}px; }}",
            cs_cols * 8,
            cs_rows * 8
        );
        out.push_str(".csvis::before { content: \"\"; position: absolute; left: -8px; top: -8px; width: 8px; height: 8px; background: transparent; pointer-events: none; z-index: 2; }\n");
        out.push_str(".csvis::after { z-index: 1; }\n");

        let cs_entries = (0..cs_bytes)
            .map(|i| {
                let x = (i % cs_cols) * 8 + 8;
                let y = (i / cs_cols) * 8 + 8;
                let name = &self.cs_names[i / 2];
                let byte = if i % 2 == 0 {
                    format!("--mlo(var({}))", name)
                } else {
                    format!("--mhi(var({}))", name)
                };
                format!("{}px {}px {}", x, y, Self::vis_gray(&byte))
            })
            .collect::<Vec<_>>();
        Self::emit_chunked_shadow(out, ".csvis", "::after", "csv", &cs_entries);

        let mut hl_entries = Vec::with_capacity(
            self.max_cs_read_slots
                .saturating_add(self.max_cs_store_slots),
        );
        hl_entries.extend((0..self.max_cs_read_slots).map(|s| {
            Self::vis_slot_shadow_entry(
                &format!("calc((var(--cri{}) * 2) + --eq1(var(--crp{})))", s, s),
                cs_cols,
                "48, 170, 84",
                &format!("calc(0.72 * var(--cro{}))", s),
            )
        }));
        hl_entries.extend((0..self.max_cs_store_slots).map(|s| {
            Self::vis_slot_shadow_entry(
                &format!("calc((var(--csi{}) * 2) + --eq1(var(--csp{})))", s, s),
                cs_cols,
                "38, 112, 255",
                &format!("calc(0.88 * var(--cso{}))", s),
            )
        }));
        Self::emit_chunked_shadow(out, ".csvis", "::before", "csh", &hl_entries);
    }

    pub(super) fn pair_mem_stores(stores: Vec<MemStoreByte>) -> Vec<MemStorePair> {
        let mut out = Vec::with_capacity(stores.len().div_ceil(2));
        let mut it = stores.into_iter();
        while let Some(first) = it.next() {
            let second = it.next();
            out.push(MemStorePair { first, second });
        }
        out
    }

    /// `idx` when `ok` holds, else `-1` (an index no cell ever has).
    pub(super) fn gated_index_expr(ok: &str, idx: &str) -> String {
        if ok.trim() == "1" {
            return idx.to_string();
        }
        format!(
            "calc({} * ({} + 1) - 1)",
            Self::paren_if_needed(ok),
            Self::calc_inner(idx)
        )
    }

    /// Dirty-page flags for the dispatch merge: a page is dirty when some
    /// slot's page index (`--{page_prefix}{j}`, -1 when idle) equals it. An
    /// any-store gate makes the common no-store cycle one test per page.
    pub(super) fn emit_dispatch_page_flags(
        &self,
        out: &mut String,
        flag_prefix: &str,
        page_prefix: &str,
        slots: &[String],
        pages: usize,
    ) {
        for page in 0..pages {
            let flag = format!("--{}{}", flag_prefix, page);
            let _ = writeln!(
                out,
                " {}: if(style({}: 0):); {}: if(not style({}: 0):);",
                Self::page_skip_merge_name(flag_prefix, page),
                flag,
                Self::page_skip_prev_name(flag_prefix, page),
                flag
            );
        }
        if slots.is_empty() {
            for page in 0..pages {
                let _ = writeln!(out, " --{}{}: 0;", flag_prefix, page);
            }
            return;
        }
        let gate = format!("--{}_any", flag_prefix);
        let sum = slots
            .iter()
            .map(|j| format!("var(--{}{})", page_prefix, j))
            .collect::<Vec<_>>()
            .join(" + ");
        let _ = writeln!(out, " {}: calc({} + {});", gate, sum, slots.len());
        for page in 0..pages {
            let conds = slots
                .iter()
                .map(|j| format!("style(--{}{}: {})", page_prefix, j, page))
                .collect::<Vec<_>>()
                .join(" or ");
            // The gate's `0` arm equals the default, so the fold's
            // drop-redundant-arms rule would delete it (same value, but it
            // is a short-circuit): keep it by nesting the page tests.
            let _ = writeln!(
                out,
                " --{}{}: if(style({}: 0): 0; else: if({}: 1; else: 0));",
                flag_prefix, page, gate, conds
            );
        }
    }

    /// Unregistered selector that is empty while page `page` is clean and
    /// guaranteed-invalid while it is dirty.
    pub(super) fn page_skip_merge_name(flag_prefix: &str, page: usize) -> String {
        format!("--{}s{}", flag_prefix, page)
    }

    /// The complement of [`Self::page_skip_merge_name`]: empty while dirty.
    pub(super) fn page_skip_prev_name(flag_prefix: &str, page: usize) -> String {
        format!("--{}k{}", flag_prefix, page)
    }

    /// `prev` while page `page` is clean, else `merged`, as two `var()`
    /// fallbacks of which exactly one is substituted. Cheaper per cell than
    /// `if(style(--flag: 0): prev; else: merged)`: Blink re-parses an `if()`
    /// condition on every evaluation, while a `var()` hit just skips its
    /// fallback.
    pub(super) fn page_select_expr(
        flag_prefix: &str,
        page: usize,
        prev: &str,
        merged: &str,
    ) -> String {
        format!(
            "var({}, {}) var({}, {})",
            Self::page_skip_merge_name(flag_prefix, page),
            merged,
            Self::page_skip_prev_name(flag_prefix, page),
            prev
        )
    }

    /// Byte-wise merge of a memory cell against the active store slots: each
    /// byte is the value of the last slot targeting it, else unchanged. The
    /// body lives in `--mmerge16` so the per-cell declarations stay short
    /// (untaken `if()` text is still tokenized on every recalc).
    pub(super) fn emit_mem_dispatch_merge_function(
        &self,
        out: &mut String,
        consts: &MemSlotConsts,
        addr_arms: &[String],
        addr_arms_b: &[String],
    ) {
        let mut slots: Vec<(String, String)> = Vec::new();
        for s in 0..self.max_mem_store_slots {
            for (suffix, arms, row) in [
                ("", &addr_arms[s], &consts.ms_val),
                ("b", &addr_arms_b[s], &consts.ms_val_b),
            ] {
                if arms.is_empty() {
                    continue;
                }
                let val = row[s]
                    .clone()
                    .unwrap_or_else(|| format!("var(--msv{}{})", s, suffix));
                slots.push((format!("{}{}", s, suffix), val));
            }
        }
        let inc = self.uses_mem_inc;
        let byte = |addr: &str, high: bool, keep: &str| -> String {
            let mut arms = String::new();
            if inc {
                // Cells rewritten by an increment (already merged values).
                for j in 0..3 {
                    let v = if high {
                        format!("round(down, var(--miv{j}) / 256)")
                    } else {
                        format!("mod(var(--miv{j}), 256)")
                    };
                    let _ = write!(arms, "style(--mic{j}: var(--lo)): {v}; ");
                }
            }
            for (j, val) in slots.iter().rev() {
                let _ = write!(arms, "style(--mta{}: var({})): {}; ", j, addr, val);
            }
            format!("if({}else: {})", arms, keep)
        };
        let lo = byte("--lo", false, "--mlo(var(--prev))");
        let hi = byte("--hi", true, "--mhi(var(--prev))");
        let _ = writeln!(
            out,
            "@function --mmerge16(--lo <integer>, --hi <integer>, --prev <integer>) returns <integer> {{ result: calc({} + 256 * {}); }}",
            lo, hi
        );
    }

    /// Callstack counterpart of [`emit_mem_dispatch_merge_function`].
    pub(super) fn emit_cs_dispatch_merge_function(&self, out: &mut String) {
        let byte = |target: &str, value: &str, keep: &str| -> String {
            let mut arms = String::new();
            for s in (0..self.max_cs_store_slots).rev() {
                let _ = write!(
                    arms,
                    "style(--{}{}: var(--idx)): var(--{}{}); ",
                    target, s, value, s
                );
            }
            format!("if({}else: {})", arms, keep)
        };
        let lo = byte("ctl", "cvl", "--mlo(var(--prev))");
        let hi = byte("cth", "cvh", "--mhi(var(--prev))");
        let _ = writeln!(
            out,
            "@function --csmerge(--idx <integer>, --prev <integer>) returns <integer> {{ result: calc({} + 256 * {}); }}",
            lo, hi
        );
    }

    /// New values of the (up to) three cells an active `MemInc32` rewrites.
    /// Cells are read once into `<integer>` properties; the 48-bit window
    /// only ever lives inside one `calc()` (registered `<number>` values are
    /// serialized with 6 significant digits when substituted).
    pub(super) fn emit_mem_inc_values(&self, out: &mut String) {
        for j in 0..3 {
            let _ = writeln!(
                out,
                " --mic{j}: if(style(--mik: -1): -1; else: calc(2 * var(--mik) + {}));",
                2 * j
            );
            let _ = writeln!(
                out,
                " --mir{j}: if(style(--mik: -1): 0; else: --read_m16(calc(var(--mik) + {j})));"
            );
        }
        let window = "(var(--mir0) + 65536 * var(--mir1) + 4294967296 * var(--mir2))";
        let scale = "(1 + 255 * var(--mio))";
        let old = format!("mod(round(down, {window} / {scale}), 4294967296)");
        let new = format!("mod({old} + var(--mid), 4294967296)");
        let merged = format!("({window} + ({new} - {old}) * {scale})");
        let parts = [
            format!("mod({merged}, 65536)"),
            format!("mod(round(down, {merged} / 65536), 65536)"),
            format!("round(down, {merged} / 4294967296)"),
        ];
        for (j, part) in parts.iter().enumerate() {
            let _ = writeln!(out, " --miv{j}: if(style(--mik: -1): 0; else: {part});");
        }
    }

    pub(super) fn page_count(&self) -> usize {
        self.mem_names.len().div_ceil(MEM_DIRTY_PAGE_CELLS)
    }

    /// Sums `parts` and clamps to 0/1: `0` when empty, the single term when len 1,
    /// otherwise `min(1, calc(a + b + ...))`. Used to OR boolean (0/1) terms in CSS.
    pub(super) fn clamp_sum_or_zero(parts: &[String]) -> String {
        match parts.len() {
            0 => "0".to_string(),
            1 => parts[0].clone(),
            _ => format!("min(1, calc({}))", Self::join_peeled_terms(parts)),
        }
    }

    fn join_peeled_terms(parts: &[String]) -> String {
        parts
            .iter()
            .map(|p| {
                let t = p.trim();
                Self::peel_calc(t)
                    .map(str::to_string)
                    .unwrap_or_else(|| t.to_string())
            })
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// Collapses sequential bounds checks against the same base variable in
    /// a boolean-sum (truthy) context. The kept entry is the dominating one:
    /// for `--ge`, the easiest-to-satisfy effective threshold `K - I` (min);
    /// for `--lt`, the easiest-to-satisfy effective threshold (max). If any
    /// peer limit is non-integer, falls back to per-limit-string grouping
    /// keeping the extreme offset (largest I for `--ge`, smallest for `--lt`).
    /// `<base>` matches `var(--name)` (I=0) or `calc(var(--name) + I)`.
    pub(super) fn dedupe_bounds_checks(parts: &mut Vec<String>) {
        use std::collections::{HashMap, HashSet};
        let parse = |s: &str| -> Option<(&'static str, String, String, i64)> {
            let (cmp, rest) = if let Some(r) = s.strip_prefix("--ge(") {
                ("ge", r)
            } else {
                let r = s.strip_prefix("--lt(")?;
                ("lt", r)
            };
            let close = rest.rfind(", ")?;
            let inner = &rest[..close];
            let limit = rest[close + 2..].strip_suffix(')')?.trim().to_string();
            let (base, imm) = if let Some(t) = inner.strip_prefix("var(") {
                let name = t.strip_suffix(')')?;
                (name.to_string(), 0i64)
            } else {
                let t = inner.strip_prefix("calc(var(")?;
                let close_paren = t.find(')')?;
                let name = &t[..close_paren];
                let after = &t[close_paren + 1..];
                let body = after.strip_suffix(')')?;
                let imm = if let Some(p) = body.strip_prefix(" + ") {
                    p.parse::<i64>().ok()?
                } else {
                    let p = body.strip_prefix(" - ")?;
                    -p.parse::<i64>().ok()?
                };
                (name.to_string(), imm)
            };
            Some((cmp, base, limit, imm))
        };

        let mut by_base: HashMap<(&'static str, String), Vec<_>> = HashMap::new();
        for (i, part) in parts.iter().enumerate() {
            if let Some((cmp, base, limit, imm)) = parse(part) {
                by_base
                    .entry((cmp, base))
                    .or_default()
                    .push((i, limit, imm));
            }
        }
        if by_base.is_empty() {
            return;
        }

        let mut drop_idx: HashSet<usize> = HashSet::new();
        for ((cmp, _base), entries) in by_base {
            let parsed: Option<Vec<(usize, i64)>> = entries
                .iter()
                .map(|(i, lim, imm)| lim.parse::<i64>().ok().map(|k| (*i, k - imm)))
                .collect();
            if let Some(effs) = parsed {
                // All limits are integer literals: pick the dominating entry
                // across the whole group by effective threshold.
                if let Some(best) = effs.iter().copied().reduce(|a, b| {
                    let take = if cmp == "ge" { b.1 < a.1 } else { b.1 > a.1 };
                    if take { b } else { a }
                }) {
                    for (i, _) in effs {
                        if i != best.0 {
                            drop_idx.insert(i);
                        }
                    }
                }
            } else {
                // Mixed limits: fall back to per-limit-string subgrouping,
                // keeping the extreme offset within each subgroup.
                let mut by_lim: HashMap<String, (usize, i64)> = HashMap::new();
                for (i, lim, imm) in entries {
                    match by_lim.get(&lim) {
                        None => {
                            by_lim.insert(lim, (i, imm));
                        }
                        Some(&(prev_i, prev_imm)) => {
                            let take = if cmp == "ge" {
                                imm > prev_imm
                            } else {
                                imm < prev_imm
                            };
                            if take {
                                drop_idx.insert(prev_i);
                                by_lim.insert(lim, (i, imm));
                            } else {
                                drop_idx.insert(i);
                            }
                        }
                    }
                }
            }
        }

        if drop_idx.is_empty() {
            return;
        }
        let mut idx = 0usize;
        parts.retain(|_| {
            let keep = !drop_idx.contains(&idx);
            idx += 1;
            keep
        });
    }

    pub(super) fn cs_page_count(&self) -> usize {
        self.cs_names.len().div_ceil(CALLSTACK_DIRTY_PAGE_CELLS)
    }

    /// Finds long `style(--_1pc: N) or style(--_1pc: M) or ...` chains that
    /// Removes slot-indicator alias declarations of the form
    /// ` --criN: var(--criT);` (also `cro`, `csi`, `cso`, `mri`, `mro`), then
    /// rewrites every `var(--{prefix}N)` reference in both `logic` and
    /// `support` to use the canonical slot `T`. Also drops the now-unused
    /// `@property --{prefix}N { ... }` declarations from `props`. The
    /// canonical slot's `@property` carries the original initial value, so
    /// downstream consumers see a single source of truth without a redundant
    /// per-cycle `var()` indirection.
    pub(super) fn collapse_slot_aliases(
        logic: &mut String,
        support: &mut String,
        props: &mut String,
    ) {
        use std::collections::HashMap;
        let mut aliases: HashMap<String, String> = HashMap::new();
        let mut kept = String::with_capacity(logic.len());
        for line in logic.split_inclusive('\n') {
            if let Some((alias, canonical)) = parse_slot_alias_line(line) {
                aliases.insert(alias, canonical);
            } else {
                kept.push_str(line);
            }
        }
        if aliases.is_empty() {
            return;
        }
        *logic = kept;
        for (alias, canonical) in &aliases {
            let needle = format!("var({})", alias);
            let replacement = format!("var({})", canonical);
            *logic = logic.replace(&needle, &replacement);
            *support = support.replace(&needle, &replacement);
        }
        let alias_names: std::collections::HashSet<&str> =
            aliases.keys().map(String::as_str).collect();
        let mut new_props = String::with_capacity(props.len());
        for line in props.split_inclusive('\n') {
            if let Some(name) = parse_property_name(line)
                && alias_names.contains(name)
            {
                continue;
            }
            new_props.push_str(line);
        }
        *props = new_props;
    }

    /// appear multiple times in `logic` and replaces them with
    /// `style(--gK: 1)` indicator-property references, emitting the
    /// corresponding `@property --gK` declaration into `props` and the
    /// indicator's arm definition into `init_arms`.
    pub(super) fn dedupe_pc_groups(logic: &mut String, props: &mut String, init_arms: &mut String) {
        // 1. Scan logic for all maximal `style(--_1pc: N) or style(--_1pc: M) or ...`
        //    chains. Group by exact-string identity, count occurrences.
        use std::collections::HashMap;
        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut i = 0usize;
        while i < logic.len() {
            if let Some(end) = Self::match_pc_chain(logic, i)
                && logic[i..end].contains(" or ")
            {
                *counts.entry(logic[i..end].to_string()).or_insert(0) += 1;
                i = end;
            } else {
                i += 1;
            }
        }

        // 2. Select profitable chains: K * (L - replacement_len) > arm_overhead + L.
        let mut groups: Vec<(String, String)> = Vec::new();
        let mut next_id = 0usize;
        let mut entries: Vec<(String, usize)> = counts.into_iter().collect();
        entries.sort_by(|(a, _), (b, _)| b.len().cmp(&a.len()).then_with(|| a.cmp(b)));
        for (chain, count) in entries {
            let l = chain.len();
            let replacement_len = 16usize;
            let arm_overhead = 110usize;
            let savings = count.saturating_mul(l.saturating_sub(replacement_len));
            if count >= 2 && savings > arm_overhead + l {
                let name = format!("--pcg{}", next_id);
                next_id += 1;
                groups.push((chain, name));
            }
        }

        for (chain, name) in &groups {
            let _ = writeln!(
                props,
                "@property {} {{ syntax: \"<integer>\"; initial-value: 0; inherits: true; }}",
                name
            );
            let _ = writeln!(init_arms, " {}: if({}: 1; else: 0);", name, chain);
            *logic = logic.replace(chain, &format!("style({}: 1)", name));
        }
    }

    /// Returns the byte index just past the longest `style(--_1pc: N) or
    /// style(--_1pc: M) or ...` chain starting at byte `start`, or `None`
    /// when no chain starts there. The chain may be a single
    /// `style(--_1pc: N)`; the caller filters to require at least one `or`.
    fn match_pc_chain(s: &str, start: usize) -> Option<usize> {
        let mut end = Self::match_pc_atom(s, start)?;
        loop {
            let bridge = " or ";
            if s[end..].starts_with(bridge) {
                let next_start = end + bridge.len();
                if let Some(after) = Self::match_pc_atom(s, next_start) {
                    end = after;
                    continue;
                }
            }
            return Some(end);
        }
    }

    fn match_pc_atom(s: &str, start: usize) -> Option<usize> {
        let prefix = "style(--_1pc: ";
        if !s[start..].starts_with(prefix) {
            return None;
        }
        let bytes = s.as_bytes();
        let mut p = start + prefix.len();
        let digit_start = p;
        while p < bytes.len() && bytes[p].is_ascii_digit() {
            p += 1;
        }
        if p == digit_start || p >= bytes.len() || bytes[p] != b')' {
            return None;
        }
        Some(p + 1)
    }

    pub(super) fn if_or_fallback_decl(name: &str, arms: &str, fallback: &str) -> String {
        let pruned = Self::prune_redundant_arms(arms, fallback);
        let merged = Self::merge_arms_by_value(&pruned);
        if merged.is_empty() {
            format!(" {}: {};", name, fallback)
        } else {
            format!(" {}: if({}else: {});", name, merged, fallback)
        }
    }

    fn merge_arms_by_value(arms: &str) -> String {
        if arms.is_empty() {
            return String::new();
        }
        let mut parsed: Vec<(String, String)> = Vec::new();
        let bytes = arms.as_bytes();
        let mut start = 0usize;
        let mut depth = 0i32;
        let mut i = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b';' if depth == 0 => {
                    let arm = arms[start..i].trim();
                    if let Some(idx) = arm.rfind("): ") {
                        let cond = arm[..=idx].trim().to_string();
                        let value = arm[idx + 3..].trim().to_string();
                        parsed.push((cond, value));
                    } else if !arm.is_empty() {
                        // Unrecognized shape; bail out and keep arms verbatim.
                        return arms.to_string();
                    }
                    i += 1;
                    while i < bytes.len() && bytes[i] == b' ' {
                        i += 1;
                    }
                    start = i;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        if parsed.is_empty() {
            return String::new();
        }
        let mut order: Vec<String> = Vec::new();
        let mut groups: std::collections::HashMap<String, Vec<String>> =
            std::collections::HashMap::new();
        for (cond, value) in parsed {
            if !groups.contains_key(&value) {
                order.push(value.clone());
            }
            groups.entry(value).or_default().push(cond);
        }
        let mut out = String::new();
        for value in order {
            let conds = groups
                .get(&value)
                .expect("value was inserted via order push");
            out.push_str(&conds.join(" or "));
            out.push_str(": ");
            out.push_str(&value);
            out.push_str("; ");
        }
        out
    }

    fn prune_redundant_arms(arms: &str, fallback: &str) -> String {
        let fb = fallback.trim();
        let mut kept: Vec<&str> = Vec::new();
        let bytes = arms.as_bytes();
        let mut start = 0usize;
        let mut depth = 0i32;
        let mut i = 0usize;
        while i < bytes.len() {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b';' if depth == 0 => {
                    let arm = arms[start..i].trim();
                    if !arm.is_empty() {
                        if let Some(idx) = arm.rfind("): ") {
                            let value = arm[idx + 3..].trim();
                            if value != fb {
                                kept.push(arm);
                            }
                        } else {
                            kept.push(arm);
                        }
                    }
                    i += 1;
                    while i < bytes.len() && bytes[i] == b' ' {
                        i += 1;
                    }
                    start = i;
                    continue;
                }
                _ => {}
            }
            i += 1;
        }
        if kept.is_empty() {
            String::new()
        } else {
            let mut out = String::new();
            for arm in kept {
                out.push_str(arm);
                out.push_str("; ");
            }
            out
        }
    }

    pub(super) fn emit_if_or_fallback(out: &mut String, name: &str, arms: &str, fallback: &str) {
        let _ = writeln!(out, "{}", Self::if_or_fallback_decl(name, arms, fallback));
    }

    /// Emits one `--{prefix}{s}` declaration per slot. When an earlier slot
    /// has byte-identical arms, the later slot is aliased to `var(--{prefix}{t})`
    /// so the expensive `if(...)` chain is evaluated once and shared. CSS
    /// computed-value resolution handles the in-rule forward reference.
    pub(super) fn emit_slot_with_cse(
        out: &mut String,
        prefix: &str,
        arms: &[String],
        fallback: &str,
    ) {
        for s in 0..arms.len() {
            let alias = if arms[s].is_empty() {
                None
            } else {
                (0..s).find(|&t| !arms[t].is_empty() && arms[t] == arms[s])
            };
            if let Some(t) = alias {
                let _ = writeln!(out, " --{}{}: var(--{}{});", prefix, s, prefix, t);
            } else {
                Self::emit_if_or_fallback(out, &format!("--{}{}", prefix, s), &arms[s], fallback);
            }
        }
    }

    pub(super) fn emit_chunked_prefixed(
        out: &mut String,
        chunk_size: usize,
        line_prefix: &str,
        entries: impl IntoIterator<Item = String>,
    ) {
        let mut line = String::new();
        for (i, entry) in entries.into_iter().enumerate() {
            line.push_str(&entry);
            if (i + 1) % chunk_size == 0 {
                let _ = writeln!(out, "{}{}", line_prefix, line);
                line.clear();
            }
        }
        if !line.is_empty() {
            let _ = writeln!(out, "{}{}", line_prefix, line);
        }
    }

    pub(super) fn emit_keyframes(&self, out: &mut String) {
        let _ = writeln!(out, "@keyframes store {{");
        let _ = writeln!(out, "  0%, 100% {{");
        let _ = writeln!(out, "    --_2pc: var(--_0pc);");
        Self::emit_chunked_prefixed(
            out,
            8,
            "   ",
            (0..self.program.num_vregs).map(|r| format!(" --_2r{}: var(--_0r{});", r, r)),
        );
        for g in 0..self.program.global_init.len() as u32 {
            // TODO(i64): staged global snapshots are currently fixed to 4 byte lanes.
            let store_g_line = (0..4u8)
                .map(|lane| {
                    format!(
                        "    {}: var({});",
                        Self::staged_global_lane_name(2, g, lane),
                        Self::staged_global_lane_name(0, g, lane),
                    )
                })
                .collect::<String>();
            let _ = writeln!(out, "{}", store_g_line);
        }
        Self::emit_chunked_prefixed(
            out,
            8,
            "   ",
            self.mem_names.iter().map(|name| {
                format!(
                    " {}: var({});",
                    Self::shadow_name(2, name),
                    Self::shadow_name(0, name),
                )
            }),
        );
        if self.uses_callstack() {
            let _ = writeln!(out, "    --_2cs_sp: var(--_0cs_sp);");
            Self::emit_chunked_prefixed(
                out,
                8,
                "   ",
                self.cs_names.iter().map(|name| {
                    format!(
                        " {}: var({});",
                        Self::shadow_name(2, name),
                        Self::shadow_name(0, name)
                    )
                }),
            );
        }
        if self.uses_exceptions {
            let _ = writeln!(out, "    --_2exc_flag: var(--_0exc_flag);");
            for lane in 0..4u8 {
                let _ = writeln!(out, "    --_2exc_tag_{}: var(--_0exc_tag_{});", lane, lane);
            }
        }
        if self.uses_exc_payload {
            for lane in 0..4u8 {
                let _ = writeln!(
                    out,
                    "    --_2exc_payload_{}: var(--_0exc_payload_{});",
                    lane, lane
                );
            }
        }
        let _ = writeln!(out, "    --_2fb: var(--_0fb);");
        let _ = writeln!(out, "    --_2ra: var(--_0ra);");
        let _ = writeln!(out, "  }}");
        let _ = writeln!(out, "}}");

        let _ = writeln!(out, "@keyframes execute {{");
        let _ = writeln!(out, "  0%, 100% {{");
        let _ = writeln!(out, "    --_0pc: var(--pc);");
        Self::emit_chunked_prefixed(
            out,
            8,
            "   ",
            (0..self.program.num_vregs).map(|r| format!(" --_0r{}: var(--r{});", r, r)),
        );
        for g in 0..self.program.global_init.len() as u32 {
            // TODO(i64): staged global snapshots are currently fixed to 4 byte lanes.
            let exec_g_line = (0..4u8)
                .map(|lane| {
                    format!(
                        "    {}: var({});",
                        Self::staged_global_lane_name(0, g, lane),
                        Self::global_lane_name(g, lane)
                    )
                })
                .collect::<String>();
            let _ = writeln!(out, "{}", exec_g_line);
        }
        Self::emit_chunked_prefixed(
            out,
            8,
            "   ",
            self.mem_names
                .iter()
                .map(|name| format!(" {}: var({});", Self::shadow_name(0, name), name)),
        );
        if self.uses_callstack() {
            let _ = writeln!(out, "    --_0cs_sp: var(--cs_sp);");
            Self::emit_chunked_prefixed(
                out,
                8,
                "   ",
                self.cs_names
                    .iter()
                    .map(|name| format!(" {}: var({});", Self::shadow_name(0, name), name)),
            );
        }
        if self.uses_exceptions {
            let _ = writeln!(out, "    --_0exc_flag: var(--exc_flag);");
            for lane in 0..4u8 {
                let _ = writeln!(out, "    --_0exc_tag_{}: var(--exc_tag_{});", lane, lane);
            }
        }
        if self.uses_exc_payload {
            for lane in 0..4u8 {
                let _ = writeln!(
                    out,
                    "    --_0exc_payload_{}: var(--exc_payload_{});",
                    lane, lane
                );
            }
        }
        let _ = writeln!(out, "    --_0fb: var(--fb);");
        let _ = writeln!(out, "    --_0ra: var(--ra);");
        let _ = writeln!(out, "  }}");
        let _ = writeln!(out, "}}");
    }
}
