use super::*;

impl<'a> Emitter<'a> {
    fn prop(name: &str, init: impl std::fmt::Display) -> String {
        format!(
            "@property {} {{ syntax: \"<integer>\"; initial-value: {}; inherits: true; }}",
            name, init
        )
    }

    pub(super) fn emit_properties(&self, out: &mut String) {
        let _ = writeln!(out, "{}", Self::prop("--pc", self.entry_pc));
        let _ = writeln!(out, "{}", Self::prop("--wait_input", 0));
        if self.js_coprocessor {
            let _ = writeln!(out, "{}", Self::prop("--cop_op", 0));
            for suffix in ["a", "b", "o"] {
                for lane in 0..4u8 {
                    let _ = writeln!(
                        out,
                        "{}",
                        Self::prop(&format!("--cop_{}{}", suffix, lane), 0)
                    );
                }
            }
        }

        Self::emit_chunked_entries(
            out,
            8,
            (0..self.program.num_vregs).map(|r| Self::prop(&format!("--r{}", r), 0)),
        );

        for g in 0..self.global_count {
            let gv = self.global_init[g as usize];
            let g_props_line = (0..4u8)
                .map(|lane| {
                    let init = (gv >> (u32::from(lane) * 8)) & 0xff;
                    Self::prop(&Self::global_lane_name(g, lane), init)
                })
                .collect::<String>();
            let _ = writeln!(out, "{}", g_props_line);
        }

        Self::emit_chunked_entries(
            out,
            8,
            self.mem_names
                .iter()
                .enumerate()
                .map(|(i, name)| Self::prop(name, self.mem_init.get(i).copied().unwrap_or(0))),
        );
        if self.max_mem_store_slots > 0 {
            Self::emit_chunked_entries(
                out,
                8,
                (0..self.page_count()).map(|page| Self::prop(&format!("--mwdp{}", page), 0)),
            );
        }

        if self.uses_callstack {
            let _ = writeln!(out, "{}", Self::prop("--cs_sp", 0));
            Self::emit_chunked_entries(
                out,
                8,
                self.cs_names.iter().map(|name| Self::prop(name, 0)),
            );
            if self.max_cs_store_slots > 0 {
                Self::emit_chunked_entries(
                    out,
                    8,
                    (0..self.cs_page_count())
                        .map(|page| Self::prop(&format!("--cswdp{}", page), 0)),
                );
            }
        }

        if self.uses_exceptions {
            let _ = writeln!(out, "{}", Self::prop("--exc_flag", 0));
            for lane in 0..4u8 {
                let _ = writeln!(out, "{}", Self::prop(&format!("--exc_tag_{}", lane), 0));
            }
        }
        if self.uses_exc_payload {
            for lane in 0..4u8 {
                let _ = writeln!(
                    out,
                    "{}",
                    Self::prop(&format!("--exc_payload_{}", lane), 0)
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
