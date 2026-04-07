use super::*;

impl<'a> Emitter<'a> {
    pub(super) fn emit_properties(&self, out: &mut String) {
        let _ = writeln!(
            out,
            "@property --pc {{ syntax: \"<integer>\"; initial-value: {}; inherits: true; }}",
            self.entry_pc
        );
        let _ = writeln!(
            out,
            "@property --wait_input {{ syntax: \"<integer>\"; initial-value: 0; inherits: true; }}"
        );
        if self.js_coprocessor {
            let _ = writeln!(
                out,
                "@property --cop_op {{ syntax: \"<integer>\"; initial-value: 0; inherits: true; }}"
            );
            for suffix in ["a", "b", "o"] {
                // TODO(i64): coprocessor lane properties are fixed to 4 bytes.
                for lane in 0..4u8 {
                    let _ = writeln!(
                        out,
                        "@property --cop_{}{} {{ syntax: \"<integer>\"; initial-value: 0; inherits: true; }}",
                        suffix, lane
                    );
                }
            }
        }

        Self::emit_chunked_entries(
            out,
            8,
            (0..self.program.num_vregs).map(|r| {
                format!(
                    "@property --r{} {{ syntax: \"<integer>\"; initial-value: 0; inherits: true; }}",
                    r
                )
            }),
        );

        for g in 0..self.global_count {
            let gv = self.global_init[g as usize];
            // TODO(i64): global CSS properties currently expose exactly 4 byte lanes per global.
            let g_props_line = (0..4u8)
                .map(|lane| {
                    let init = (gv >> (u32::from(lane) * 8)) & 0xff;
                    format!(
                        "@property {} {{ syntax: \"<integer>\"; initial-value: {}; inherits: true; }}",
                        Self::global_lane_name(g, lane),
                        init
                    )
                })
                .collect::<String>();
            let _ = writeln!(out, "{}", g_props_line);
        }

        Self::emit_chunked_entries(
            out,
            8,
            self.mem_names.iter().enumerate().map(|(i, name)| {
                let init = self.mem_init.get(i).copied().unwrap_or(0);
                format!(
                    "@property {} {{ syntax: \"<integer>\"; initial-value: {}; inherits: true; }}",
                    name, init
                )
            }),
        );
        if self.max_mem_store_slots > 0 {
            Self::emit_chunked_entries(
                out,
                8,
                (0..self.page_count()).map(|page| {
                    format!(
                        "@property --mwdp{} {{ syntax: \"<integer>\"; initial-value: 0; inherits: true; }}",
                        page
                    )
                }),
            );
        }

        if self.uses_callstack {
            let _ = writeln!(
                out,
                "@property --cs_sp {{ syntax: \"<integer>\"; initial-value: 0; inherits: true; }}"
            );
            Self::emit_chunked_entries(
                out,
                8,
                self.cs_names.iter().map(|name| {
                    format!(
                        "@property {} {{ syntax: \"<integer>\"; initial-value: 0; inherits: true; }}",
                        name
                    )
                }),
            );
            if self.max_cs_store_slots > 0 {
                Self::emit_chunked_entries(
                    out,
                    8,
                    (0..self.cs_page_count()).map(|page| {
                        format!(
                            "@property --cswdp{} {{ syntax: \"<integer>\"; initial-value: 0; inherits: true; }}",
                            page
                        )
                    }),
                );
            }
        }
    }

    fn emit_chunked_entries(
        out: &mut String,
        chunk_size: usize,
        entries: impl IntoIterator<Item = String>,
    ) {
        let mut line = String::new();
        for (i, entry) in entries.into_iter().enumerate() {
            line.push_str(&entry);
            if (i + 1) % chunk_size == 0 {
                let _ = writeln!(out, "{}", line);
                line.clear();
            }
        }
        if !line.is_empty() {
            let _ = writeln!(out, "{}", line);
        }
    }
}
