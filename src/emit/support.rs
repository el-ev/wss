use super::*;

impl<'a> Emitter<'a> {
    pub(super) fn emit_support(&self, out: &mut String) {
        fn terminal_codes(uses_callstack: bool) -> impl Iterator<Item = TrapCode> {
            TrapCode::TERMINAL
                .into_iter()
                .filter(move |code| uses_callstack || *code != TrapCode::CallstackOverflow)
        }

        out.push_str("@function --pow2(--s <integer>) returns <number> { result: if(");
        // TODO(i64): pow2 lookup is currently capped for 32-bit bit-manipulation helpers.
        for s in 0..=32u32 {
            let v = 1u64 << s;
            let _ = write!(out, "style(--s: {}): {}; ", s, v);
        }
        out.push_str("else: 1); }\n");

        self.emit_i2char(out);
        self.emit_hex_digit(out);
        self.emit_byte_clz_lookup(out);
        self.emit_byte_ctz_lookup(out);

        let mem_shadow = self
            .mem_names
            .iter()
            .map(|n| Self::shadow_name(0, n))
            .collect::<Vec<_>>();
        let mem_reads = mem_shadow
            .iter()
            .enumerate()
            .map(|(i, name)| (i, format!("var({})", name)))
            .collect::<Vec<_>>();
        Self::emit_chunked_read_function(out, "--read_m16", &mem_reads);

        if self.uses_callstack {
            let cs_shadow = self
                .cs_names
                .iter()
                .map(|n| Self::shadow_name(1, n))
                .collect::<Vec<_>>();
            let cs_reads = cs_shadow
                .iter()
                .enumerate()
                .map(|(i, name)| (i, format!("var({})", name)))
                .collect::<Vec<_>>();
            Self::emit_chunked_read_function(out, "--read_cs", &cs_reads);
        }

        self.emit_memory_merge_function(out);
        self.emit_callstack_merge_function(out);
        self.emit_keyframes(out);
        if self.features.contains(TemplateFeatures::MEM_VISUALIZER) {
            self.emit_memory_visualizer(out);
        }
        if self.features.contains(TemplateFeatures::CS_VISUALIZER) {
            self.emit_callstack_visualizer(out);
        }

        out.push_str(".screen::after { white-space: pre-wrap; word-break: break-all; ");
        out.push_str("content: if(");
        // TODO(i64): exit-code display currently formats return values as 32-bit hex words.
        for code in terminal_codes(self.uses_callstack) {
            if code == TrapCode::Exited {
                let _ = write!(
                    out,
                    "style(--pc: {}): var(--fb, \"\") \"\\a[Program exited with code \" var(--ra, \"0x00000000\") \"]\"; ",
                    code.pc()
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
        for code in terminal_codes(self.uses_callstack) {
            if code == TrapCode::Exited {
                continue;
            }
            let _ = write!(out, "style(--pc: {}): #d00; ", code.pc());
        }
        out.push_str("else: #222); display: block; flex: 0 0 auto; }\n");

        for code in terminal_codes(self.uses_callstack) {
            if code == TrapCode::Exited {
                let _ = writeln!(
                    out,
                    "@container style(--pc: {}) {{ .screen::after {{ content: var(--fb, \"\") \"\\a[Program exited with code \" var(--ra, \"0x00000000\") \"]\"; }} }}",
                    code.pc()
                );
                continue;
            }
            let _ = writeln!(
                out,
                "@container style(--pc: {}) {{ .screen::after {{ content: var(--fb, \"\") \"\\a[{}]\"; color: #d00; }} }}",
                code.pc(),
                code.screen_label().unwrap_or("")
            );
        }

        for code in terminal_codes(self.uses_callstack) {
            let _ = writeln!(
                out,
                ".clk:has(.terminal) {{ @container style(--pc: {}) {{ .terminal {{ animation-play-state: paused, paused !important; }} }} }}",
                code.pc()
            );
        }
    }

    pub(super) fn emit_i2char(&self, out: &mut String) {
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

    pub(super) fn emit_hex_digit(&self, out: &mut String) {
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

    pub(super) fn emit_byte_clz_lookup(&self, out: &mut String) {
        Self::emit_byte_lookup(out, "--byte_clz", |b| {
            if b == 0 { 8 } else { b.leading_zeros() as u8 }
        });
    }

    pub(super) fn emit_byte_ctz_lookup(&self, out: &mut String) {
        Self::emit_byte_lookup(out, "--byte_ctz", |b| {
            if b == 0 { 8 } else { b.trailing_zeros() as u8 }
        });
    }

    pub(super) fn shadow_name(stage: u8, base: &str) -> String {
        let tail = base.strip_prefix("--").unwrap_or(base);
        format!("--_{}{}", stage, tail)
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
        for (chunk, chunk_reads) in reads.chunks(READ_LOOKUP_CHUNK).enumerate() {
            let _ = write!(
                out,
                "@function {}_{}(--i <integer>) returns <integer> {{ result: if(",
                func, chunk
            );
            for (idx, expr) in chunk_reads {
                let _ = write!(out, "style(--i: {}): {}; ", idx, expr);
            }
            out.push_str("else: 0); }\n");
        }
        if chunk_count == 1 {
            let _ = writeln!(
                out,
                "@function {}(--i <integer>) returns <integer> {{ result: {}_0(var(--i)); }}",
                func, func
            );
            return;
        }
        let _ = write!(
            out,
            "@function {}(--i <integer>) returns <integer> {{ result: calc(",
            func
        );
        for chunk in 0..chunk_count {
            if chunk != 0 {
                out.push_str(" + ");
            }
            let _ = write!(out, "{}_{}(var(--i))", func, chunk);
        }
        out.push_str("); }\n");
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
                    format!("mod(var({}), 256)", name)
                } else {
                    format!("calc(var({}) / 256)", name)
                };
                format!("{}px {}px rgb({}, {}, {})", x, y, byte, byte, byte)
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
                    format!("mod(var({}), 256)", name)
                } else {
                    format!("calc(var({}) / 256)", name)
                };
                format!("{}px {}px rgb({}, {}, {})", x, y, byte, byte, byte)
            })
            .collect::<Vec<_>>();
        Self::emit_chunked_shadow(out, ".csvis", "::after", "csv", &cs_entries);

        let mut hl_entries = Vec::with_capacity(
            self.max_cs_read_slots
                .saturating_add(self.max_cs_store_slots),
        );
        hl_entries.extend((0..self.max_cs_read_slots).map(|s| {
            Self::vis_slot_shadow_entry(
                &format!(
                    "calc((var(--cri{}) * 2) + --sel(--eq1(var(--crp{})), 1, 0))",
                    s, s
                ),
                cs_cols,
                "48, 170, 84",
                &format!("calc(0.72 * var(--cro{}))", s),
            )
        }));
        hl_entries.extend((0..self.max_cs_store_slots).map(|s| {
            Self::vis_slot_shadow_entry(
                &format!(
                    "calc((var(--csi{}) * 2) + --sel(--eq1(var(--csp{})), 1, 0))",
                    s, s
                ),
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

    pub(super) fn merge_byte_expr_for_cell(
        &self,
        cell_expr: &str,
        par: usize,
        prev: &str,
    ) -> String {
        let parity_fn = if par == 0 { "--eqz" } else { "--eq1" };
        let mut expr = prev.to_string();
        for s in 0..self.max_mem_store_slots {
            for suffix in ["", "b"] {
                let cond = format!(
                    "calc(var(--mso{}{}) * --eq(var(--msc{}{}), {}) * {}(var(--msp{}{})))",
                    s, suffix, s, suffix, cell_expr, parity_fn, s, suffix
                );
                expr = format!(
                    "calc(({}) * (var(--msv{}{})) + (1 - ({})) * ({}))",
                    cond, s, suffix, cond, expr
                );
            }
        }
        expr
    }

    pub(super) fn merge_word_expr_for_cell(&self, cell_expr: &str, prev_word: &str) -> String {
        let lo_prev = format!("mod(({}), 256)", prev_word);
        let hi_prev = format!("mod(round(down, calc(({}) / 256)), 256)", prev_word);
        let lo = self.merge_byte_expr_for_cell(cell_expr, 0, &lo_prev);
        let hi = self.merge_byte_expr_for_cell(cell_expr, 1, &hi_prev);
        format!("calc(({}) + 256 * ({}))", lo, hi)
    }

    pub(super) fn emit_memory_merge_function(&self, out: &mut String) {
        if self.max_mem_store_slots == 0 {
            return;
        }
        let body = self.merge_word_expr_for_cell("var(--cell)", "var(--prev)");
        let _ = writeln!(
            out,
            "@function --mmerge16(--cell <number>, --prev <number>) returns <integer> {{ result: {}; }}",
            body
        );
    }

    pub(super) fn merge_callstack_expr_for_index(&self, idx_expr: &str, prev: &str) -> String {
        let mut lo_expr = format!("--mlo({})", prev);
        let mut hi_expr = format!("--mhi({})", prev);
        for s in 0..self.max_cs_store_slots {
            let same_slot = format!(
                "calc(var(--cso{}) * --eq(var(--csi{}), {}))",
                s, s, idx_expr
            );
            let is_word = format!("--eq(var(--csp{}), 2)", s);
            let is_lo = format!("--eqz(var(--csp{}))", s);
            let is_hi = format!("--eq1(var(--csp{}))", s);

            let lo_cond = format!(
                "calc(({}) * min(1, calc(({}) + ({}))))",
                same_slot, is_lo, is_word
            );
            let hi_cond = format!(
                "calc(({}) * min(1, calc(({}) + ({}))))",
                same_slot, is_hi, is_word
            );

            let lo_val = format!(
                "calc(({}) * --mlo(var(--csv{})) + (1 - ({})) * var(--csv{}))",
                is_word, s, is_word, s
            );
            let hi_val = format!(
                "calc(({}) * --mhi(var(--csv{})) + (1 - ({})) * var(--csv{}))",
                is_word, s, is_word, s
            );

            lo_expr = format!(
                "calc(({}) * ({}) + (1 - ({})) * ({}))",
                lo_cond, lo_val, lo_cond, lo_expr
            );
            hi_expr = format!(
                "calc(({}) * ({}) + (1 - ({})) * ({}))",
                hi_cond, hi_val, hi_cond, hi_expr
            );
        }
        format!("--m16({}, {})", lo_expr, hi_expr)
    }

    pub(super) fn emit_callstack_merge_function(&self, out: &mut String) {
        if !self.uses_callstack || self.max_cs_store_slots == 0 {
            return;
        }
        let body = self.merge_callstack_expr_for_index("var(--idx)", "var(--prev)");
        let _ = writeln!(
            out,
            "@function --csmerge(--idx <number>, --prev <number>) returns <integer> {{ result: {}; }}",
            body
        );
    }

    pub(super) fn page_count(&self) -> usize {
        self.mem_names.len().div_ceil(MEM_DIRTY_PAGE_CELLS)
    }

    pub(super) fn dirty_page_expr(&self, page: usize) -> String {
        if self.max_mem_store_slots == 0 {
            return "0".to_string();
        }
        let terms = (0..self.max_mem_store_slots)
            .flat_map(|s| {
                ["", "b"].into_iter().map(move |suffix| {
                    format!(
                        "calc(var(--mso{}{}) * --eq(round(down, calc(var(--msc{}{}) / {})), {}))",
                        s, suffix, s, suffix, MEM_DIRTY_PAGE_CELLS, page
                    )
                })
            })
            .collect::<Vec<_>>();
        if terms.len() == 1 {
            terms[0].clone()
        } else {
            format!("min(1, calc({}))", terms.join(" + "))
        }
    }

    pub(super) fn cs_page_count(&self) -> usize {
        self.cs_names.len().div_ceil(CALLSTACK_DIRTY_PAGE_CELLS)
    }

    pub(super) fn cs_dirty_page_expr(&self, page: usize) -> String {
        if self.max_cs_store_slots == 0 {
            return "0".to_string();
        }
        let terms = (0..self.max_cs_store_slots)
            .map(|s| {
                format!(
                    "calc(var(--cso{}) * --eq(round(down, calc(var(--csi{}) / {})), {}))",
                    s, s, CALLSTACK_DIRTY_PAGE_CELLS, page
                )
            })
            .collect::<Vec<_>>();
        if terms.len() == 1 {
            terms[0].clone()
        } else {
            format!("min(1, calc({}))", terms.join(" + "))
        }
    }

    pub(super) fn if_or_fallback_decl(name: &str, arms: &str, fallback: &str) -> String {
        if arms.is_empty() {
            format!(" {}: {};", name, fallback)
        } else {
            format!(" {}: if({}else: {});", name, arms, fallback)
        }
    }

    pub(super) fn emit_if_or_fallback(out: &mut String, name: &str, arms: &str, fallback: &str) {
        let _ = writeln!(out, "{}", Self::if_or_fallback_decl(name, arms, fallback));
    }

    pub(super) fn emit_partitioned_active_flag(out: &mut String, name: &str, pcs: &[u16]) {
        if pcs.is_empty() {
            let _ = writeln!(out, " {}: 0;", name);
            return;
        }
        let arms_for = |chunk: &[u16]| {
            chunk
                .iter()
                .map(|pc| format!("style(--_1pc: {}): 1; ", pc))
                .collect::<String>()
        };
        if pcs.len() <= ACTIVE_FLAG_ARMS_CHUNK {
            let arms = arms_for(pcs);
            Self::emit_if_or_fallback(out, name, &arms, "0");
            return;
        }

        let mut parts = Vec::new();
        for (chunk_idx, chunk) in pcs.chunks(ACTIVE_FLAG_ARMS_CHUNK).enumerate() {
            let part_name = format!("{}p{}", name, chunk_idx);
            parts.push(part_name.clone());
            let arms = arms_for(chunk);
            Self::emit_if_or_fallback(out, &part_name, &arms, "0");
        }
        let joined = parts
            .iter()
            .map(|n| format!("var({})", n))
            .collect::<Vec<_>>()
            .join(" + ");
        let _ = writeln!(out, " {}: min(1, calc({}));", name, joined);
    }

    fn emit_chunked_prefixed(
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
        let _ = writeln!(out, "    --_2pc: var(--_0pc, {});", self.entry_pc);
        Self::emit_chunked_prefixed(
            out,
            8,
            "   ",
            (0..self.program.num_vregs).map(|r| format!(" --_2r{}: var(--_0r{}, 0);", r, r)),
        );
        for g in 0..self.global_count {
            let gv = self.global_init[g as usize];
            // TODO(i64): staged global snapshots are currently fixed to 4 byte lanes.
            let store_g_line = (0..4u8)
                .map(|lane| {
                    let init = (gv >> (u32::from(lane) * 8)) & 0xff;
                    format!(
                        "    {}: var({}, {});",
                        Self::staged_global_lane_name(2, g, lane),
                        Self::staged_global_lane_name(0, g, lane),
                        init
                    )
                })
                .collect::<String>();
            let _ = writeln!(out, "{}", store_g_line);
        }
        Self::emit_chunked_prefixed(
            out,
            8,
            "   ",
            self.mem_names.iter().enumerate().map(|(i, name)| {
                let init = self.mem_init.get(i).copied().unwrap_or(0);
                format!(
                    " {}: var({}, {});",
                    Self::shadow_name(2, name),
                    Self::shadow_name(0, name),
                    init
                )
            }),
        );
        if self.uses_callstack {
            let _ = writeln!(out, "    --_2cs_sp: var(--_0cs_sp, 0);");
            Self::emit_chunked_prefixed(
                out,
                8,
                "   ",
                self.cs_names.iter().map(|name| {
                    format!(
                        " {}: var({}, 0);",
                        Self::shadow_name(2, name),
                        Self::shadow_name(0, name)
                    )
                }),
            );
        }
        if self.uses_exceptions {
            let _ = writeln!(out, "    --_2exc_flag: var(--_0exc_flag, 0);");
            for lane in 0..4u8 {
                let _ = writeln!(
                    out,
                    "    --_2exc_tag_{}: var(--_0exc_tag_{}, 0);",
                    lane, lane
                );
            }
        }
        if self.uses_exc_payload {
            for lane in 0..4u8 {
                let _ = writeln!(
                    out,
                    "    --_2exc_payload_{}: var(--_0exc_payload_{}, 0);",
                    lane, lane
                );
            }
        }
        let _ = writeln!(out, "    --_2fb: var(--_0fb, \"\");");
        let _ = writeln!(out, "    --_2ra: var(--_0ra, \"0x00000000\");");
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
        for g in 0..self.global_count {
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
        if self.uses_callstack {
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
                let _ = writeln!(
                    out,
                    "    --_0exc_tag_{}: var(--exc_tag_{});",
                    lane, lane
                );
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
