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

        Self::emit_chunked_prefixed(
            out,
            8,
            "",
            (0..self.program.num_vregs).map(|r| Self::prop(&format!("--r{}", r), 0)),
        );

        for g in 0..self.program.global_init.len() as u32 {
            for lane in 0..4u8 {
                let init = self.program.global_init_byte(g, lane);
                let _ = write!(
                    out,
                    "{}",
                    Self::prop(&Self::global_lane_name(g, lane), init)
                );
            }
            let _ = writeln!(out);
        }

        // Memory cells stay unregistered: every value they take is already
        // an integer token (the store snapshot, its initial literal, or an
        // `--mmerge16` result typed by the function), and an unregistered
        // property skips the per-recalc typed parse, ~1/3 of a cell's cost.
        if self.max_mem_store_slots > 0 {
            Self::emit_chunked_prefixed(
                out,
                8,
                "",
                (0..self.page_count()).map(|page| Self::prop(&format!("--mwdp{}", page), 0)),
            );
            if self.uses_mem_inc {
                let _ = writeln!(out, "{}", Self::prop("--mik", -1));
                let _ = writeln!(out, "{}", Self::prop("--mio", 0));
                let _ = writeln!(out, "{}", Self::prop("--mid", 0));
                for j in 0..3 {
                    let _ = writeln!(out, "{}", Self::prop(&format!("--mic{}", j), -1));
                    let _ = writeln!(out, "{}", Self::prop(&format!("--mir{}", j), 0));
                    let _ = writeln!(out, "{}", Self::prop(&format!("--miv{}", j), 0));
                }
                let _ = writeln!(out, "{}", Self::prop("--mtpi0", -1));
                let _ = writeln!(out, "{}", Self::prop("--mtpi2", -1));
            }
            {
                let _ = writeln!(out, "{}", Self::prop("--mwdp_any", 0));
                Self::emit_chunked_prefixed(
                    out,
                    8,
                    "",
                    (0..self.max_mem_store_slots).flat_map(|s| {
                        ["", "b"].into_iter().flat_map(move |b| {
                            [
                                Self::prop(&format!("--mta{}{}", s, b), -1),
                                Self::prop(&format!("--mtp{}{}", s, b), -1),
                            ]
                        })
                    }),
                );
            }
        }

        if self.uses_callstack() {
            // Callstack cells stay unregistered, like memory cells.
            let _ = writeln!(out, "{}", Self::prop("--cs_sp", 0));
            if self.max_cs_store_slots > 0 {
                Self::emit_chunked_prefixed(
                    out,
                    8,
                    "",
                    (0..self.cs_page_count())
                        .map(|page| Self::prop(&format!("--cswdp{}", page), 0)),
                );
                {
                    let _ = writeln!(out, "{}", Self::prop("--cswdp_any", 0));
                    Self::emit_chunked_prefixed(
                        out,
                        8,
                        "",
                        (0..self.max_cs_store_slots).flat_map(|s| {
                            [
                                Self::prop(&format!("--ctl{}", s), -1),
                                Self::prop(&format!("--cth{}", s), -1),
                                Self::prop(&format!("--cvl{}", s), 0),
                                Self::prop(&format!("--cvh{}", s), 0),
                                Self::prop(&format!("--ctp{}", s), -1),
                            ]
                        }),
                    );
                }
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
                let _ = writeln!(out, "{}", Self::prop(&format!("--exc_payload_{}", lane), 0));
            }
        }
    }
}
