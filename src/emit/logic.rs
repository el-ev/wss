use super::*;

impl<'a> Emitter<'a> {
    pub(super) fn emit_logic(&self, out: &mut String) {
        let nr = self.program.num_vregs as usize;
        let mut r_arms = vec![String::new(); nr];
        let mut g_arms: BTreeMap<(u32, u8), String> = BTreeMap::new();
        let mut pc_arms = String::new();
        let mut wait_input_arms = String::new();
        let mut fb_arms = String::new();
        let mut ra_arms = String::new();
        let mut cop_op_arms = String::new();
        let mut cop_a_arms = vec![String::new(); 4];
        let mut cop_b_arms = vec![String::new(); 4];
        let mut cs_sp_arms = String::new();
        let mut exc_flag_arms = String::new();
        let mut exc_tag_arms: [String; 4] = Default::default();
        let mut exc_payload_arms: [String; 4] = Default::default();
        let mut mw_active_pcs = Vec::new();
        let mut csw_active_pcs = Vec::new();

        let mut ms_cell_arms = vec![String::new(); self.max_mem_store_slots];
        let mut ms_par_arms = vec![String::new(); self.max_mem_store_slots];
        let mut ms_val_arms = vec![String::new(); self.max_mem_store_slots];
        let mut ms_ok_arms = vec![String::new(); self.max_mem_store_slots];
        let mut ms_cell_arms_b = vec![String::new(); self.max_mem_store_slots];
        let mut ms_par_arms_b = vec![String::new(); self.max_mem_store_slots];
        let mut ms_val_arms_b = vec![String::new(); self.max_mem_store_slots];
        let mut ms_ok_arms_b = vec![String::new(); self.max_mem_store_slots];
        let mut mr_idx_arms = vec![String::new(); self.max_mem_read_slots];
        let mut mr_ok_arms = vec![String::new(); self.max_mem_read_slots];

        let mut csw_idx_arms = vec![String::new(); self.max_cs_store_slots];
        let mut csw_par_arms = vec![String::new(); self.max_cs_store_slots];
        let mut csw_val_arms = vec![String::new(); self.max_cs_store_slots];
        let mut csw_ok_arms = vec![String::new(); self.max_cs_store_slots];
        let mut csr_idx_arms = vec![String::new(); self.max_cs_read_slots];
        let mut csr_par_arms = vec![String::new(); self.max_cs_read_slots];
        let mut csr_ok_arms = vec![String::new(); self.max_cs_read_slots];

        for cycle in &self.program.cycles {
            let pc = cycle.pc.index();
            let mut reg_now = HashMap::new();
            let mut global_sets = HashMap::new();
            let mut mem_stores_raw = Vec::new();
            let mut mem_reads = Vec::new();
            let mut cs_stores = Vec::new();
            let mut cs_reads = Vec::new();
            let mut cs_sp_expr: Option<String> = None;
            let mut exc_flag_set_expr: Option<String> = None;
            let mut exc_tag_set_exprs: [Option<String>; 4] = Default::default();
            let mut exc_payload_set_exprs: [Option<String>; 4] = Default::default();
            let mut loaded_pc_expr: Option<String> = None;
            let mut trap_mem_parts = Vec::new();
            let mut trap_cs_parts = Vec::new();
            let mut putchars = Vec::new();
            let mut has_getchar = false;
            let mut getchar_ready = String::from("0");

            for op in &cycle.ops {
                match &op.kind {
                    Inst8Kind::Copy(s) => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(dst.expect_vreg(), Self::val_expr(&reg_now, *s));
                        }
                    }
                    Inst8Kind::Add32Byte { lhs, rhs, lane } => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                Self::add32_byte_expr(&reg_now, *lhs, *rhs, *lane),
                            );
                        }
                    }
                    Inst8Kind::Sub32Byte { lhs, rhs, lane } => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                Self::sub32_byte_expr(&reg_now, *lhs, *rhs, *lane),
                            );
                        }
                    }
                    Inst8Kind::Sub32Borrow { lhs, rhs } => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                Self::sub32_borrow_expr(&reg_now, *lhs, *rhs),
                            );
                        }
                    }
                    Inst8Kind::Add(l, r) => {
                        if let Some(dst) = op.dst {
                            let e = format!(
                                "mod(calc(({}) + ({})), 256)",
                                Self::val_expr(&reg_now, *l),
                                Self::val_expr(&reg_now, *r)
                            );
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::Carry(l, r) => {
                        if let Some(dst) = op.dst {
                            let e = format!(
                                "--lt(255, calc(({}) + ({})))",
                                Self::val_expr(&reg_now, *l),
                                Self::val_expr(&reg_now, *r)
                            );
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::Sub(l, r) => {
                        if let Some(dst) = op.dst {
                            let e = format!(
                                "mod(calc(({}) - ({}) + 256), 256)",
                                Self::val_expr(&reg_now, *l),
                                Self::val_expr(&reg_now, *r)
                            );
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::MulLo(l, r) => {
                        if let Some(dst) = op.dst {
                            let e = format!(
                                "mod(calc(({}) * ({})), 256)",
                                Self::val_expr(&reg_now, *l),
                                Self::val_expr(&reg_now, *r)
                            );
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::MulHi(l, r) => {
                        if let Some(dst) = op.dst {
                            let e = format!(
                                "mod(round(down, calc((({}) * ({})) / 256)), 256)",
                                Self::val_expr(&reg_now, *l),
                                Self::val_expr(&reg_now, *r)
                            );
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::And8(l, r) => {
                        if let Some(dst) = op.dst {
                            let e = Self::bitwise_expr(
                                "and",
                                &Self::val_expr(&reg_now, *l),
                                &Self::val_expr(&reg_now, *r),
                            );
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::Or8(l, r) => {
                        if let Some(dst) = op.dst {
                            let e = Self::bitwise_expr(
                                "or",
                                &Self::val_expr(&reg_now, *l),
                                &Self::val_expr(&reg_now, *r),
                            );
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::Xor8(l, r) => {
                        if let Some(dst) = op.dst {
                            let e = Self::bitwise_expr(
                                "xor",
                                &Self::val_expr(&reg_now, *l),
                                &Self::val_expr(&reg_now, *r),
                            );
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::Eq(l, r) => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                format!(
                                    "--eq({}, {})",
                                    Self::val_expr(&reg_now, *l),
                                    Self::val_expr(&reg_now, *r)
                                ),
                            );
                        }
                    }
                    Inst8Kind::Ne(l, r) => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                format!(
                                    "--ne({}, {})",
                                    Self::val_expr(&reg_now, *l),
                                    Self::val_expr(&reg_now, *r)
                                ),
                            );
                        }
                    }
                    Inst8Kind::LtU(l, r) => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                format!(
                                    "--lt({}, {})",
                                    Self::val_expr(&reg_now, *l),
                                    Self::val_expr(&reg_now, *r)
                                ),
                            );
                        }
                    }
                    Inst8Kind::GeU(l, r) => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                format!(
                                    "calc(1 - (--lt({}, {})))",
                                    Self::val_expr(&reg_now, *l),
                                    Self::val_expr(&reg_now, *r)
                                ),
                            );
                        }
                    }
                    Inst8Kind::BoolAnd(bool_op) => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                Self::bool_nary_expr(&reg_now, bool_op, true),
                            );
                        }
                    }
                    Inst8Kind::BoolOr(bool_op) => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                Self::bool_nary_expr(&reg_now, bool_op, false),
                            );
                        }
                    }
                    Inst8Kind::BoolNot(s) => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                format!("calc(1 - min(1, ({})))", Self::val_expr(&reg_now, *s)),
                            );
                        }
                    }
                    Inst8Kind::Sel(c, l, r) => {
                        if let Some(dst) = op.dst {
                            let c_expr = Self::val_expr(&reg_now, *c);
                            let t_expr = Self::val_expr(&reg_now, *l);
                            let f_expr = Self::val_expr(&reg_now, *r);
                            reg_now.insert(
                                dst.expect_vreg(),
                                Self::sel_expr(&c_expr, &t_expr, &f_expr),
                            );
                        }
                    }
                    Inst8Kind::GlobalGetByte { global_idx, lane } => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                format!(
                                    "var({})",
                                    Self::staged_global_lane_name(1, *global_idx, *lane)
                                ),
                            );
                        }
                    }
                    Inst8Kind::GlobalSetByte {
                        global_idx,
                        lane,
                        val,
                    } => {
                        global_sets.insert((*global_idx, *lane), Self::val_expr(&reg_now, *val));
                    }
                    Inst8Kind::LoadMem { base, addr, lane } => {
                        if let Some(dst) = op.dst {
                            let (addr, in_bounds) = self.mem_addr_bounds(
                                &reg_now,
                                addr,
                                *base,
                                *lane,
                                &mut trap_mem_parts,
                            );
                            mem_reads.push(MemRead {
                                byte: addr.clone(),
                                ok: in_bounds.clone(),
                            });
                            let e = Self::sel_expr(
                                &in_bounds,
                                &format!("--mload({})", addr),
                                &format!("var(--_1r{})", dst.expect_vreg()),
                            );
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::StoreMem {
                        base,
                        addr,
                        lane,
                        val,
                    } => {
                        let (byte_addr, in_bounds) =
                            self.mem_addr_bounds(&reg_now, addr, *base, *lane, &mut trap_mem_parts);
                        mem_stores_raw.push(MemStoreByte {
                            cell: format!("--mhalf({})", byte_addr),
                            parity: format!("--mpar({})", byte_addr),
                            val: format!("mod({}, 256)", Self::val_expr(&reg_now, *val)),
                            ok: in_bounds,
                        });
                    }
                    Inst8Kind::Getchar => {
                        if let Some(dst) = op.dst {
                            has_getchar = true;
                            getchar_ready = "--ne(var(--kb, -1), -1)".to_string();
                            reg_now.insert(
                                dst.expect_vreg(),
                                Self::sel_expr(
                                    &getchar_ready,
                                    "mod(var(--kb, -1), 256)",
                                    &format!("var(--_1r{})", dst.expect_vreg()),
                                ),
                            );
                        }
                    }
                    Inst8Kind::Putchar(v) => {
                        putchars.push(Self::val_expr(&reg_now, *v));
                    }
                    Inst8Kind::CsStore { offset, val } => {
                        let slot_off = (*offset) / 2;
                        let parity = (*offset) % 2;
                        let idx = format!("calc(var(--_1cs_sp) + {})", slot_off);
                        let ok = self.cs_bounds_check(&idx, false, &mut trap_cs_parts);
                        cs_stores.push(CsStore {
                            idx,
                            parity: format!("{}", parity),
                            val: format!("mod({}, 256)", Self::val_expr(&reg_now, *val)),
                            ok,
                        });
                    }
                    Inst8Kind::CsLoad { offset } => {
                        if let Some(dst) = op.dst {
                            let slot_off = (*offset) / 2;
                            let parity = (*offset) % 2;
                            let idx = format!("calc(var(--_1cs_sp) + {})", slot_off);
                            let ok = self.cs_bounds_check(&idx, false, &mut trap_cs_parts);
                            cs_reads.push(CsRead {
                                idx: idx.clone(),
                                parity: format!("{}", parity),
                                ok: ok.clone(),
                            });
                            let word = format!("--read_cs({})", idx);
                            let load = if parity == 0 {
                                format!("--mlo({})", word)
                            } else {
                                format!("--mhi({})", word)
                            };
                            reg_now.insert(
                                dst.expect_vreg(),
                                Self::sel_expr(
                                    &ok,
                                    &load,
                                    &format!("var(--_1r{})", dst.expect_vreg()),
                                ),
                            );
                        }
                    }
                    Inst8Kind::CsStorePc { offset, val } => {
                        let idx = format!("calc(var(--_1cs_sp) + {})", offset);
                        let ok = self.cs_bounds_check(&idx, false, &mut trap_cs_parts);
                        cs_stores.push(CsStore {
                            idx,
                            parity: "2".to_string(),
                            val: format!("{}", val.index()),
                            ok,
                        });
                    }
                    Inst8Kind::CsLoadPc { offset } => {
                        let idx = format!("calc(var(--_1cs_sp) + {})", offset);
                        let ok = self.cs_bounds_check(&idx, false, &mut trap_cs_parts);
                        cs_reads.push(CsRead {
                            idx: idx.clone(),
                            parity: "2".to_string(),
                            ok: ok.clone(),
                        });
                        loaded_pc_expr = Some(Self::sel_expr(
                            &ok,
                            &format!("--read_cs({})", idx),
                            "var(--_1pc)",
                        ));
                    }
                    Inst8Kind::CsAlloc(size) => {
                        let next_sp = format!("calc(var(--_1cs_sp) + {})", size);
                        let ok = self.cs_bounds_check(&next_sp, true, &mut trap_cs_parts);
                        cs_sp_expr = Some(Self::sel_expr(&ok, &next_sp, "var(--_1cs_sp)"));
                    }
                    Inst8Kind::CsFree(size) => {
                        let next_sp = format!("calc(var(--_1cs_sp) - {})", size);
                        let ok = self.cs_bounds_check(&next_sp, true, &mut trap_cs_parts);
                        cs_sp_expr = Some(Self::sel_expr(&ok, &next_sp, "var(--_1cs_sp)"));
                    }
                    Inst8Kind::ExcFlagSet { val } => {
                        exc_flag_set_expr = Some(Self::val_expr(&reg_now, *val));
                    }
                    Inst8Kind::ExcFlagGet => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(dst.expect_vreg(), "var(--_1exc_flag)".to_string());
                        }
                    }
                    Inst8Kind::ExcTagSet { lane, val } => {
                        exc_tag_set_exprs[*lane as usize] = Some(Self::val_expr(&reg_now, *val));
                    }
                    Inst8Kind::ExcTagGet { lane } => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(dst.expect_vreg(), format!("var(--_1exc_tag_{})", lane));
                        }
                    }
                    Inst8Kind::ExcPayloadSet { lane, val } => {
                        exc_payload_set_exprs[*lane as usize] =
                            Some(Self::val_expr(&reg_now, *val));
                    }
                    Inst8Kind::ExcPayloadGet { lane } => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                format!("var(--_1exc_payload_{})", lane),
                            );
                        }
                    }
                }
            }

            let cop_setup = match &cycle.terminator {
                Terminator8::CallSetup {
                    callee_entry: CallTarget::Builtin(builtin),
                    args,
                    ..
                } => self.coprocessor_setup_for_builtin_call(*builtin, args, &reg_now),
                _ => None,
            };

            let mut term = self.emit_terminator(cycle, &mut reg_now, loaded_pc_expr);

            let trap_mem = if trap_mem_parts.is_empty() {
                "0".to_string()
            } else if trap_mem_parts.len() == 1 {
                trap_mem_parts[0].clone()
            } else {
                format!("min(1, calc({}))", trap_mem_parts.join(" + "))
            };

            if trap_mem != "0" {
                term.pc_expr = Self::sel_expr(
                    &trap_mem,
                    &format!("{}", TrapCode::InvalidMemoryAccess as i32),
                    &term.pc_expr,
                );
            }
            term.trap_expr = match (term.trap_expr.as_str(), trap_mem.as_str()) {
                ("0", "0") => "0".to_string(),
                ("0", _) => trap_mem.clone(),
                (_, "0") => term.trap_expr.clone(),
                _ => format!("min(1, calc(({}) + ({})))", term.trap_expr, trap_mem),
            };

            if self.uses_callstack {
                let trap_cs = if trap_cs_parts.is_empty() {
                    "0".to_string()
                } else if trap_cs_parts.len() == 1 {
                    trap_cs_parts[0].clone()
                } else {
                    format!("min(1, calc({}))", trap_cs_parts.join(" + "))
                };

                if trap_cs != "0" {
                    term.pc_expr = Self::sel_expr(
                        &trap_cs,
                        &format!("{}", TrapCode::CallstackOverflow as i32),
                        &term.pc_expr,
                    );
                }
                term.trap_expr = match (term.trap_expr.as_str(), trap_cs.as_str()) {
                    ("0", "0") => "0".to_string(),
                    ("0", _) => trap_cs.clone(),
                    (_, "0") => term.trap_expr.clone(),
                    _ => format!("min(1, calc(({}) + ({})))", term.trap_expr, trap_cs),
                };
            }

            if has_getchar {
                term.pc_expr = Self::sel_expr(&getchar_ready, &term.pc_expr, &format!("{}", pc));
                let _ = write!(
                    wait_input_arms,
                    "style(--_1pc: {}): calc(1 - ({})); ",
                    pc, getchar_ready
                );
            }

            let fallthrough_pc_expr = format!("{}", pc.saturating_add(1));
            let is_trivial_fallthrough =
                term.trap_expr == "0" && term.pc_expr == fallthrough_pc_expr;
            if !is_trivial_fallthrough {
                let _ = write!(pc_arms, "style(--_1pc: {}): {}; ", pc, term.pc_expr);
            }
            if let Some(exit_code) = &term.exit_code_expr {
                let _ = write!(ra_arms, "style(--_1pc: {}): {}; ", pc, exit_code);
            }

            if let Some(cop_setup) = cop_setup {
                let _ = write!(
                    cop_op_arms,
                    "style(--_1pc: {}): {}; ",
                    pc, cop_setup.op_code
                );
                for lane in 0..4usize {
                    let _ = write!(
                        cop_a_arms[lane],
                        "style(--_1pc: {}): {}; ",
                        pc, cop_setup.lhs[lane]
                    );
                    let _ = write!(
                        cop_b_arms[lane],
                        "style(--_1pc: {}): {}; ",
                        pc, cop_setup.rhs[lane]
                    );
                }
            }

            if !putchars.is_empty() {
                let append = putchars
                    .iter()
                    .map(|v| format!(" --i2char({})", v))
                    .collect::<String>();
                let fb_expr = format!("var(--_1fb){}", append);
                let _ = write!(fb_arms, "style(--_1pc: {}): {}; ", pc, fb_expr);
            }

            for (r, expr) in reg_now {
                let idx = r as usize;
                if idx < r_arms.len() {
                    let _ = write!(r_arms[idx], "style(--_1pc: {}): {}; ", pc, expr);
                }
            }

            for ((g, lane), expr) in global_sets {
                let arms = g_arms.entry((g, lane)).or_default();
                let _ = write!(arms, "style(--_1pc: {}): {}; ", pc, expr);
            }

            let mem_stores = Self::pair_mem_stores(mem_stores_raw);
            if !mem_stores.is_empty() {
                mw_active_pcs.push(pc);
            }
            for s in 0..self.max_mem_store_slots {
                if let Some(ms) = mem_stores.get(s) {
                    let _ = write!(
                        ms_cell_arms[s],
                        "style(--_1pc: {}): {}; ",
                        pc, &ms.first.cell
                    );
                    let _ = write!(
                        ms_par_arms[s],
                        "style(--_1pc: {}): {}; ",
                        pc, &ms.first.parity
                    );
                    let _ = write!(ms_val_arms[s], "style(--_1pc: {}): {}; ", pc, &ms.first.val);
                    let _ = write!(ms_ok_arms[s], "style(--_1pc: {}): {}; ", pc, &ms.first.ok);
                    if let Some(msb) = &ms.second {
                        let _ = write!(ms_cell_arms_b[s], "style(--_1pc: {}): {}; ", pc, &msb.cell);
                        let _ =
                            write!(ms_par_arms_b[s], "style(--_1pc: {}): {}; ", pc, &msb.parity);
                        let _ = write!(ms_val_arms_b[s], "style(--_1pc: {}): {}; ", pc, &msb.val);
                        let _ = write!(ms_ok_arms_b[s], "style(--_1pc: {}): {}; ", pc, &msb.ok);
                    }
                }
            }
            for s in 0..self.max_mem_read_slots {
                if let Some(mr) = mem_reads.get(s) {
                    let _ = write!(mr_idx_arms[s], "style(--_1pc: {}): {}; ", pc, mr.byte);
                    let _ = write!(mr_ok_arms[s], "style(--_1pc: {}): {}; ", pc, mr.ok);
                }
            }

            if self.uses_callstack {
                if !cs_stores.is_empty() {
                    csw_active_pcs.push(pc);
                }
                for s in 0..self.max_cs_store_slots {
                    if let Some(cs) = cs_stores.get(s) {
                        let _ = write!(csw_idx_arms[s], "style(--_1pc: {}): {}; ", pc, cs.idx);
                        let _ = write!(csw_par_arms[s], "style(--_1pc: {}): {}; ", pc, cs.parity);
                        let _ = write!(csw_val_arms[s], "style(--_1pc: {}): {}; ", pc, cs.val);
                        let _ = write!(csw_ok_arms[s], "style(--_1pc: {}): {}; ", pc, cs.ok);
                    }
                }
                for s in 0..self.max_cs_read_slots {
                    if let Some(cs) = cs_reads.get(s) {
                        let _ = write!(csr_idx_arms[s], "style(--_1pc: {}): {}; ", pc, cs.idx);
                        let _ = write!(csr_par_arms[s], "style(--_1pc: {}): {}; ", pc, cs.parity);
                        let _ = write!(csr_ok_arms[s], "style(--_1pc: {}): {}; ", pc, cs.ok);
                    }
                }
            }

            if self.uses_callstack
                && let Some(e) = cs_sp_expr
            {
                let _ = write!(cs_sp_arms, "style(--_1pc: {}): {}; ", pc, e);
            }

            if self.uses_exceptions {
                if let Some(e) = exc_flag_set_expr {
                    let _ = write!(exc_flag_arms, "style(--_1pc: {}): {}; ", pc, e);
                }
                for (lane, slot) in exc_tag_set_exprs.iter().enumerate() {
                    if let Some(e) = slot {
                        let _ = write!(exc_tag_arms[lane], "style(--_1pc: {}): {}; ", pc, e);
                    }
                }
            }
            if self.uses_exc_payload {
                for (lane, slot) in exc_payload_set_exprs.iter().enumerate() {
                    if let Some(e) = slot {
                        let _ = write!(exc_payload_arms[lane], "style(--_1pc: {}): {}; ", pc, e);
                    }
                }
            }
        }

        let _ = writeln!(out, " --_1pc: var(--_2pc, {});", self.entry_pc);
        let mut r_shadow_line = String::new();
        for r in 0..self.program.num_vregs {
            let _ = write!(r_shadow_line, " --_1r{}: var(--_2r{}, 0);", r, r);
            if (r as usize + 1).is_multiple_of(8) {
                let _ = writeln!(out, "{}", r_shadow_line);
                r_shadow_line.clear();
            }
        }
        if !r_shadow_line.is_empty() {
            let _ = writeln!(out, "{}", r_shadow_line);
        }
        for g in 0..self.global_count {
            let gv = self.global_init[g as usize];
            let mut g_shadow_line = String::new();
            for lane in 0..4u8 {
                let init = (gv >> (u32::from(lane) * 8)) & 0xff;
                let _ = write!(
                    g_shadow_line,
                    " {}: var({}, {});",
                    Self::staged_global_lane_name(1, g, lane),
                    Self::staged_global_lane_name(2, g, lane),
                    init
                );
            }
            let _ = writeln!(out, "{}", g_shadow_line);
        }
        let mut mem_shadow_line = String::new();
        for (i, name) in self.mem_names.iter().enumerate() {
            let init = self.mem_init.get(i).copied().unwrap_or(0);
            let s = Self::shadow_name(2, name);
            let _ = write!(
                mem_shadow_line,
                " {}: var({}, {});",
                Self::shadow_name(1, name),
                s,
                init
            );
            if (i + 1) % 8 == 0 {
                let _ = writeln!(out, "{}", mem_shadow_line);
                mem_shadow_line.clear();
            }
        }
        if !mem_shadow_line.is_empty() {
            let _ = writeln!(out, "{}", mem_shadow_line);
        }
        if self.uses_exc_payload {
            for lane in 0..4u8 {
                let _ = writeln!(
                    out,
                    " --_1exc_payload_{}: var(--_2exc_payload_{}, 0);",
                    lane, lane
                );
            }
        }
        if self.uses_exceptions {
            let _ = writeln!(out, " --_1exc_flag: var(--_2exc_flag, 0);");
            for lane in 0..4u8 {
                let _ = writeln!(out, " --_1exc_tag_{}: var(--_2exc_tag_{}, 0);", lane, lane);
            }
        }
        if self.uses_callstack {
            let _ = writeln!(out, " --_1cs_sp: var(--_2cs_sp, 0);");
            let mut cs_shadow_line = String::new();
            for (i, name) in self.cs_names.iter().enumerate() {
                let _ = write!(
                    cs_shadow_line,
                    " {}: var({}, 0);",
                    Self::shadow_name(1, name),
                    Self::shadow_name(2, name)
                );
                if (i + 1) % 8 == 0 {
                    let _ = writeln!(out, "{}", cs_shadow_line);
                    cs_shadow_line.clear();
                }
            }
            if !cs_shadow_line.is_empty() {
                let _ = writeln!(out, "{}", cs_shadow_line);
            }
        }
        let _ = writeln!(out, " --_1fb: var(--_2fb, \"\");");
        let _ = writeln!(out, " --_1ra: var(--_2ra, \"0x00000000\");");

        for s in 0..self.max_mem_store_slots {
            Self::emit_if_or_fallback(out, &format!("--msc{}", s), &ms_cell_arms[s], "-1");
            Self::emit_if_or_fallback(out, &format!("--msp{}", s), &ms_par_arms[s], "-1");
            Self::emit_if_or_fallback(out, &format!("--msv{}", s), &ms_val_arms[s], "0");
            Self::emit_if_or_fallback(out, &format!("--mso{}", s), &ms_ok_arms[s], "0");
            Self::emit_if_or_fallback(out, &format!("--msc{}b", s), &ms_cell_arms_b[s], "-1");
            Self::emit_if_or_fallback(out, &format!("--msp{}b", s), &ms_par_arms_b[s], "-1");
            Self::emit_if_or_fallback(out, &format!("--msv{}b", s), &ms_val_arms_b[s], "0");
            Self::emit_if_or_fallback(out, &format!("--mso{}b", s), &ms_ok_arms_b[s], "0");
        }
        Self::emit_partitioned_active_flag(out, "--mw_active", &mw_active_pcs);

        if self.max_mem_store_slots > 0 {
            for page in 0..self.page_count() {
                let expr = self.dirty_page_expr(page);
                let _ = writeln!(out, " --mwdp{}: {};", page, expr);
            }
        }

        for s in 0..self.max_mem_read_slots {
            Self::emit_if_or_fallback(out, &format!("--mri{}", s), &mr_idx_arms[s], "-1");
            Self::emit_if_or_fallback(out, &format!("--mro{}", s), &mr_ok_arms[s], "0");
        }

        if self.js_coprocessor {
            Self::emit_if_or_fallback(out, "--cop_op", &cop_op_arms, &format!("{}", COP_OP_NONE));
            for lane in 0..4usize {
                Self::emit_if_or_fallback(out, &format!("--cop_a{}", lane), &cop_a_arms[lane], "0");
                Self::emit_if_or_fallback(out, &format!("--cop_b{}", lane), &cop_b_arms[lane], "0");
            }
        }

        if self.uses_callstack {
            for s in 0..self.max_cs_store_slots {
                Self::emit_if_or_fallback(out, &format!("--csi{}", s), &csw_idx_arms[s], "-1");
                Self::emit_if_or_fallback(out, &format!("--csp{}", s), &csw_par_arms[s], "-1");
                Self::emit_if_or_fallback(out, &format!("--csv{}", s), &csw_val_arms[s], "0");
                Self::emit_if_or_fallback(out, &format!("--cso{}", s), &csw_ok_arms[s], "0");
            }
            Self::emit_partitioned_active_flag(out, "--csw_active", &csw_active_pcs);
            if self.max_cs_store_slots > 0 {
                for page in 0..self.cs_page_count() {
                    let expr = self.cs_dirty_page_expr(page);
                    let guarded = format!("if(style(--csw_active: 1): {}; else: 0)", expr);
                    let _ = writeln!(out, " --cswdp{}: {};", page, guarded);
                }
            }
            for s in 0..self.max_cs_read_slots {
                Self::emit_if_or_fallback(out, &format!("--cri{}", s), &csr_idx_arms[s], "-1");
                Self::emit_if_or_fallback(out, &format!("--crp{}", s), &csr_par_arms[s], "-1");
                Self::emit_if_or_fallback(out, &format!("--cro{}", s), &csr_ok_arms[s], "0");
            }
        }

        let pc_fallback = "--sel(--lt(var(--_1pc), 0), var(--_1pc), calc(var(--_1pc) + 1))";
        Self::emit_if_or_fallback(out, "--pc", &pc_arms, pc_fallback);
        Self::emit_if_or_fallback(out, "--wait_input", &wait_input_arms, "0");
        if self.uses_callstack {
            Self::emit_if_or_fallback(out, "--cs_sp", &cs_sp_arms, "var(--_1cs_sp)");
        }
        if self.uses_exceptions {
            Self::emit_if_or_fallback(out, "--exc_flag", &exc_flag_arms, "var(--_1exc_flag)");
            for lane in 0..4u8 {
                let fallback = format!("var(--_1exc_tag_{})", lane);
                Self::emit_if_or_fallback(
                    out,
                    &format!("--exc_tag_{}", lane),
                    &exc_tag_arms[lane as usize],
                    &fallback,
                );
            }
        }
        if self.uses_exc_payload {
            for lane in 0..4u8 {
                let fallback = format!("var(--_1exc_payload_{})", lane);
                Self::emit_if_or_fallback(
                    out,
                    &format!("--exc_payload_{}", lane),
                    &exc_payload_arms[lane as usize],
                    &fallback,
                );
            }
        }

        for (r, arms) in r_arms.iter().enumerate() {
            let fallback = format!("var(--_1r{})", r);
            Self::emit_if_or_fallback(out, &format!("--r{}", r), arms, &fallback);
        }

        for g in 0..self.global_count {
            let mut g_line = String::new();
            for lane in 0..4u8 {
                let arms = g_arms.get(&(g, lane)).cloned().unwrap_or_default();
                let global_lane = Self::global_lane_name(g, lane);
                let fallback = format!("var({})", Self::staged_global_lane_name(1, g, lane));
                g_line.push_str(&Self::if_or_fallback_decl(&global_lane, &arms, &fallback));
            }
            let _ = writeln!(out, "{}", g_line);
        }

        let mut mem_line = String::new();
        for (i, name) in self.mem_names.iter().enumerate() {
            let prev = format!("var({})", Self::shadow_name(1, name));
            let merged = if self.max_mem_store_slots == 0 {
                prev.clone()
            } else {
                format!("--mmerge16({}, {})", i, prev)
            };
            let page = i / MEM_DIRTY_PAGE_CELLS;
            let expr = if self.max_mem_store_slots == 0 {
                prev.clone()
            } else {
                let page_merge =
                    format!("if(style(--mwdp{}: 1): {}; else: {})", page, merged, prev);
                format!("if(style(--mw_active: 1): {}; else: {})", page_merge, prev)
            };
            let _ = write!(mem_line, " {}: {};", name, expr);
            if (i + 1) % 8 == 0 {
                let _ = writeln!(out, "{}", mem_line);
                mem_line.clear();
            }
        }
        if !mem_line.is_empty() {
            let _ = writeln!(out, "{}", mem_line);
        }

        if self.uses_callstack {
            let mut cs_line = String::new();
            for (i, name) in self.cs_names.iter().enumerate() {
                let prev = format!("var({})", Self::shadow_name(1, name));
                let expr = if self.max_cs_store_slots == 0 {
                    prev.clone()
                } else {
                    let merged = format!("--csmerge({}, {})", i, prev);
                    let page = i / CALLSTACK_DIRTY_PAGE_CELLS;
                    let page_merge =
                        format!("if(style(--cswdp{}: 1): {}; else: {})", page, merged, prev);
                    format!("if(style(--csw_active: 1): {}; else: {})", page_merge, prev)
                };
                let _ = write!(cs_line, " {}: {};", name, expr);
                if (i + 1) % 8 == 0 {
                    let _ = writeln!(out, "{}", cs_line);
                    cs_line.clear();
                }
            }
            if !cs_line.is_empty() {
                let _ = writeln!(out, "{}", cs_line);
            }
        }

        Self::emit_if_or_fallback(out, "--fb", &fb_arms, "var(--_1fb)");
        Self::emit_if_or_fallback(out, "--ra", &ra_arms, "var(--_1ra)");
    }

    pub(super) fn emit_terminator(
        &self,
        cycle: &crate::ir8::Cycle,
        reg_now: &mut HashMap<u16, String>,
        loaded_pc_expr: Option<String>,
    ) -> TermResult {
        match &cycle.terminator {
            Terminator8::Goto(pc) => TermResult {
                pc_expr: format!("{}", pc.index()),
                trap_expr: "0".to_string(),
                exit_code_expr: None,
            },
            Terminator8::Branch {
                cond,
                if_true,
                if_false,
            } => {
                let c = Self::val_expr(reg_now, *cond);
                TermResult {
                    pc_expr: Self::sel_expr(
                        &c,
                        &format!("{}", if_true.index()),
                        &format!("{}", if_false.index()),
                    ),
                    trap_expr: "0".to_string(),
                    exit_code_expr: None,
                }
            }
            Terminator8::Switch {
                index,
                targets,
                default,
            } => {
                let idx = Self::val_expr(reg_now, *index);
                let mut expr = format!("{}", default.index());
                for (i, target) in targets.iter().enumerate().rev() {
                    expr = Self::sel_expr(
                        &format!("--eq({}, {})", idx, i),
                        &format!("{}", target.index()),
                        &expr,
                    );
                }
                TermResult {
                    pc_expr: expr,
                    trap_expr: "0".to_string(),
                    exit_code_expr: None,
                }
            }
            Terminator8::CallSetup {
                callee_entry,
                cont,
                args,
                callee_arg_vregs,
            } => match callee_entry {
                CallTarget::Builtin(builtin) => {
                    let (ret, trap) = self.eval_builtin(*builtin, args, reg_now);
                    for (i, expr) in ret.into_iter().enumerate() {
                        let gated = Self::sel_expr(&trap, &format!("var(--_1r{})", i), &expr);
                        reg_now.insert(i as u16, gated);
                    }
                    TermResult {
                        pc_expr: Self::sel_expr(
                            &trap,
                            &TrapCode::DivisionByZero.pc().to_string(),
                            &format!("{}", cont.index()),
                        ),
                        trap_expr: trap,
                        exit_code_expr: None,
                    }
                }
                CallTarget::Pc(callee_pc) => {
                    for (src, dst) in args.iter().zip(callee_arg_vregs.iter()) {
                        reg_now.insert(dst.b0.expect_vreg(), Self::val_expr(reg_now, src.b0));
                        reg_now.insert(dst.b1.expect_vreg(), Self::val_expr(reg_now, src.b1));
                        reg_now.insert(dst.b2.expect_vreg(), Self::val_expr(reg_now, src.b2));
                        reg_now.insert(dst.b3.expect_vreg(), Self::val_expr(reg_now, src.b3));
                    }
                    TermResult {
                        pc_expr: format!("{}", callee_pc.index()),
                        trap_expr: "0".to_string(),
                        exit_code_expr: None,
                    }
                }
            },
            Terminator8::Return { val } => {
                let exit_code_expr = Self::emit_exit_value(reg_now, *val);
                let fallback_pc_expr = if self.uses_callstack {
                    // Return may be in a later packed cycle than CsLoadPc; in that case,
                    // re-read RA from the current call-stack top.
                    let idx = "calc(var(--_1cs_sp) + 0)";
                    let ok = format!(
                        "calc(--lt(-1, {}) * --lt({}, {}))",
                        idx,
                        idx,
                        self.cs_names.len()
                    );
                    Self::sel_expr(&ok, &format!("--read_cs({})", idx), "var(--_1pc)")
                } else {
                    "var(--_1pc)".to_string()
                };
                TermResult {
                    pc_expr: loaded_pc_expr.unwrap_or(fallback_pc_expr),
                    trap_expr: "0".to_string(),
                    exit_code_expr,
                }
            }
            Terminator8::Exit { val } => {
                let exit_code_expr = Self::emit_exit_value(reg_now, *val);
                TermResult {
                    pc_expr: TrapCode::Exited.pc().to_string(),
                    trap_expr: "0".to_string(),
                    exit_code_expr,
                }
            }
            Terminator8::Trap(code) => TermResult {
                pc_expr: format!("{}", *code as i32),
                trap_expr: "0".to_string(),
                exit_code_expr: None,
            },
        }
    }

    pub(super) fn val_expr(now: &HashMap<u16, String>, r: Val8) -> String {
        if let Some(v) = r.imm_value() {
            return format!("{}", v);
        }
        now.get(&r.expect_vreg())
            .cloned()
            .unwrap_or_else(|| format!("var(--_1r{})", r.expect_vreg()))
    }

    pub(super) fn bool_nary_expr(
        now: &HashMap<u16, String>,
        op: &crate::ir8::BoolNary8,
        and: bool,
    ) -> String {
        let terms: Vec<String> = op
            .as_slice()
            .iter()
            .map(|r| Self::val_expr(now, *r))
            .collect();
        if terms.is_empty() {
            return if and {
                "1".to_string()
            } else {
                "0".to_string()
            };
        }
        if terms.len() == 1 {
            return terms[0].clone();
        }
        let joined = terms
            .iter()
            .map(|term| format!("({term})"))
            .collect::<Vec<_>>()
            .join(if and { " * " } else { " + " });
        format!("calc(min(1, {joined}))")
    }

    fn mem_addr_bounds(
        &self,
        reg_now: &HashMap<u16, String>,
        addr: &crate::ir8::Addr,
        base: u16,
        lane: u8,
        trap_parts: &mut Vec<String>,
    ) -> (String, String) {
        let byte_addr = format!(
            "--addr16({}, {}, {})",
            Self::val_expr(reg_now, addr.lo),
            Self::val_expr(reg_now, addr.hi),
            (base as u32) + (lane as u32)
        );
        let in_bounds = format!("--lt({}, {})", byte_addr, self.memory_end);
        trap_parts.push(format!("calc(1 - ({}))", in_bounds));
        (byte_addr, in_bounds)
    }

    fn cs_bounds_check(&self, idx: &str, extend: bool, trap_parts: &mut Vec<String>) -> String {
        let limit = self.cs_names.len() + usize::from(extend);
        let ok = format!("calc(--lt(-1, {}) * --lt({}, {}))", idx, idx, limit);
        trap_parts.push(format!("calc(1 - ({}))", ok));
        ok
    }

    pub(super) fn sel_expr(cond: &str, if_true: &str, if_false: &str) -> String {
        if !cond.contains("--sel(") && !if_true.contains("--sel(") && !if_false.contains("--sel(") {
            return format!("--sel({}, {}, {})", cond, if_true, if_false);
        }
        format!(
            "calc(({}) * ({}) + (1 - ({})) * ({}))",
            cond, if_true, cond, if_false
        )
    }

    pub(super) fn word_bytes_expr(now: &HashMap<u16, String>, w: Word) -> [String; 4] {
        [
            Self::val_expr(now, w.b0),
            Self::val_expr(now, w.b1),
            Self::val_expr(now, w.b2),
            Self::val_expr(now, w.b3),
        ]
    }

    fn byte_add_total_expr(lhs: &str, rhs: &str, carry_in: &str) -> String {
        if carry_in == "0" {
            format!("calc(({lhs}) + ({rhs}))")
        } else {
            format!("calc(({lhs}) + ({rhs}) + ({carry_in}))")
        }
    }

    fn byte_sub_total_expr(lhs: &str, rhs: &str, borrow_in: &str) -> String {
        if borrow_in == "0" {
            format!("calc(({lhs}) - ({rhs}))")
        } else {
            format!("calc(({lhs}) - ({rhs}) - ({borrow_in}))")
        }
    }

    fn add32_carry_in_expr(now: &HashMap<u16, String>, lhs: Word, rhs: Word, lane: u8) -> String {
        let mut carry = "0".to_string();
        for idx in 0..lane {
            let lhs_byte = Self::val_expr(now, lhs.byte(idx));
            let rhs_byte = Self::val_expr(now, rhs.byte(idx));
            let total = Self::byte_add_total_expr(&lhs_byte, &rhs_byte, &carry);
            carry = format!("round(down, calc(({total}) / 256))");
        }
        carry
    }

    fn sub32_borrow_in_expr(now: &HashMap<u16, String>, lhs: Word, rhs: Word, lane: u8) -> String {
        let mut borrow = "0".to_string();
        for idx in 0..lane {
            let lhs_byte = Self::val_expr(now, lhs.byte(idx));
            let rhs_byte = Self::val_expr(now, rhs.byte(idx));
            borrow = format!(
                "round(down, calc((255 - ({lhs_byte}) + ({rhs_byte}) + ({borrow})) / 256))"
            );
        }
        borrow
    }

    pub(super) fn add32_byte_expr(
        now: &HashMap<u16, String>,
        lhs: Word,
        rhs: Word,
        lane: u8,
    ) -> String {
        let lhs_byte = Self::val_expr(now, lhs.byte(lane));
        let rhs_byte = Self::val_expr(now, rhs.byte(lane));
        let carry_in = Self::add32_carry_in_expr(now, lhs, rhs, lane);
        let total = Self::byte_add_total_expr(&lhs_byte, &rhs_byte, &carry_in);
        format!("mod({total}, 256)")
    }

    pub(super) fn sub32_byte_expr(
        now: &HashMap<u16, String>,
        lhs: Word,
        rhs: Word,
        lane: u8,
    ) -> String {
        let lhs_byte = Self::val_expr(now, lhs.byte(lane));
        let rhs_byte = Self::val_expr(now, rhs.byte(lane));
        let borrow_in = Self::sub32_borrow_in_expr(now, lhs, rhs, lane);
        let total = Self::byte_sub_total_expr(&lhs_byte, &rhs_byte, &borrow_in);
        format!("mod(calc(({total}) + 256), 256)")
    }

    pub(super) fn sub32_borrow_expr(now: &HashMap<u16, String>, lhs: Word, rhs: Word) -> String {
        let mut borrow = "0".to_string();
        for idx in 0..4u8 {
            let lhs_byte = Self::val_expr(now, lhs.byte(idx));
            let rhs_byte = Self::val_expr(now, rhs.byte(idx));
            borrow = format!(
                "round(down, calc((255 - ({lhs_byte}) + ({rhs_byte}) + ({borrow})) / 256))"
            );
        }
        borrow
    }

    pub(super) fn zero_word_expr() -> [String; 4] {
        [
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
            "0".to_string(),
        ]
    }

    pub(super) fn word_sel_expr(
        cond: &str,
        if_true: [String; 4],
        if_false: [String; 4],
    ) -> [String; 4] {
        [
            Self::sel_expr(cond, &if_true[0], &if_false[0]),
            Self::sel_expr(cond, &if_true[1], &if_false[1]),
            Self::sel_expr(cond, &if_true[2], &if_false[2]),
            Self::sel_expr(cond, &if_true[3], &if_false[3]),
        ]
    }

    pub(super) fn byte_bit_expr(byte: &str, bit: u8) -> String {
        let p2 = 1u32 << bit;
        format!("mod(round(down, calc(({}) / {})), 2)", byte, p2)
    }

    pub(super) fn byte_popcnt_expr(byte: &str) -> String {
        let mut bits = Vec::with_capacity(8);
        for k in 0..8u8 {
            bits.push(Self::byte_bit_expr(byte, k));
        }
        format!("calc({})", bits.join(" + "))
    }

    pub(super) fn byte_ctz_expr(byte: &str) -> String {
        format!("--byte_ctz(mod(calc(({}) + 256), 256))", byte)
    }

    pub(super) fn byte_clz_expr(byte: &str) -> String {
        format!("--byte_clz(mod(calc(({}) + 256), 256))", byte)
    }

    pub(super) fn shl_stage_expr(word: &[String; 4], amount: u8) -> [String; 4] {
        match amount {
            0 => word.clone(),
            1 | 2 | 4 => {
                let p = 1u32 << amount;
                let q = 1u32 << (8 - amount);
                let lo = |x: &str| format!("mod(calc(({}) * {}), 256)", x, p);
                let carry = |x: &str| format!("round(down, calc(({}) / {}))", x, q);
                [
                    lo(&word[0]),
                    format!("mod(calc(({}) + ({})), 256)", lo(&word[1]), carry(&word[0])),
                    format!("mod(calc(({}) + ({})), 256)", lo(&word[2]), carry(&word[1])),
                    format!("mod(calc(({}) + ({})), 256)", lo(&word[3]), carry(&word[2])),
                ]
            }
            8 => [
                "0".to_string(),
                word[0].clone(),
                word[1].clone(),
                word[2].clone(),
            ],
            16 => [
                "0".to_string(),
                "0".to_string(),
                word[0].clone(),
                word[1].clone(),
            ],
            _ => Self::zero_word_expr(),
        }
    }

    fn shr_byte_expr(byte: &str, carry: &str, p: u32) -> String {
        format!(
            "mod(round(down, calc((({}) + 256 * ({})) / {})), 256)",
            byte, carry, p
        )
    }

    pub(super) fn shr_u_stage_expr(word: &[String; 4], amount: u8) -> [String; 4] {
        match amount {
            0 => word.clone(),
            1 | 2 | 4 => {
                let p = 1u32 << amount;
                let carry = |x: &str| format!("mod(({}), {})", x, p);
                [
                    Self::shr_byte_expr(&word[0], &carry(&word[1]), p),
                    Self::shr_byte_expr(&word[1], &carry(&word[2]), p),
                    Self::shr_byte_expr(&word[2], &carry(&word[3]), p),
                    format!("round(down, calc(({}) / {}))", word[3], p),
                ]
            }
            8 => [
                word[1].clone(),
                word[2].clone(),
                word[3].clone(),
                "0".to_string(),
            ],
            16 => [
                word[2].clone(),
                word[3].clone(),
                "0".to_string(),
                "0".to_string(),
            ],
            _ => Self::zero_word_expr(),
        }
    }

    pub(super) fn shr_s_stage_expr(word: &[String; 4], amount: u8) -> [String; 4] {
        let sign = Self::byte_bit_expr(&word[3], 7);
        match amount {
            0 => word.clone(),
            1 | 2 | 4 => {
                let p = 1u32 << amount;
                let fill = format!("calc(({}) * {})", sign, p - 1);
                let carry = |x: &str| format!("mod(({}), {})", x, p);
                [
                    Self::shr_byte_expr(&word[0], &carry(&word[1]), p),
                    Self::shr_byte_expr(&word[1], &carry(&word[2]), p),
                    Self::shr_byte_expr(&word[2], &carry(&word[3]), p),
                    Self::shr_byte_expr(&word[3], &fill, p),
                ]
            }
            8 => {
                let fill = Self::sel_expr(&sign, "255", "0");
                [
                    word[1].clone(),
                    word[2].clone(),
                    word[3].clone(),
                    fill.clone(),
                ]
            }
            16 => {
                let fill = Self::sel_expr(&sign, "255", "0");
                [word[2].clone(), word[3].clone(), fill.clone(), fill]
            }
            _ => Self::zero_word_expr(),
        }
    }

    pub(super) fn rotl_stage_expr(word: &[String; 4], amount: u8) -> [String; 4] {
        match amount {
            0 => word.clone(),
            1 | 2 | 4 => {
                let p = 1u32 << amount;
                let q = 1u32 << (8 - amount);
                let lo = |x: &str| format!("mod(calc(({}) * {}), 256)", x, p);
                let carry = |x: &str| format!("round(down, calc(({}) / {}))", x, q);
                [
                    format!("mod(calc(({}) + ({})), 256)", lo(&word[0]), carry(&word[3])),
                    format!("mod(calc(({}) + ({})), 256)", lo(&word[1]), carry(&word[0])),
                    format!("mod(calc(({}) + ({})), 256)", lo(&word[2]), carry(&word[1])),
                    format!("mod(calc(({}) + ({})), 256)", lo(&word[3]), carry(&word[2])),
                ]
            }
            8 => [
                word[3].clone(),
                word[0].clone(),
                word[1].clone(),
                word[2].clone(),
            ],
            16 => [
                word[2].clone(),
                word[3].clone(),
                word[0].clone(),
                word[1].clone(),
            ],
            _ => Self::zero_word_expr(),
        }
    }

    pub(super) fn rotr_stage_expr(word: &[String; 4], amount: u8) -> [String; 4] {
        match amount {
            0 => word.clone(),
            1 | 2 | 4 => {
                let p = 1u32 << amount;
                let q = 1u32 << (8 - amount);
                let hi = |x: &str| format!("round(down, calc(({}) / {}))", x, p);
                let carry = |x: &str| format!("calc(mod(({}), {}) * {})", x, p, q);
                [
                    format!("mod(calc(({}) + ({})), 256)", hi(&word[0]), carry(&word[1])),
                    format!("mod(calc(({}) + ({})), 256)", hi(&word[1]), carry(&word[2])),
                    format!("mod(calc(({}) + ({})), 256)", hi(&word[2]), carry(&word[3])),
                    format!("mod(calc(({}) + ({})), 256)", hi(&word[3]), carry(&word[0])),
                ]
            }
            8 => [
                word[1].clone(),
                word[2].clone(),
                word[3].clone(),
                word[0].clone(),
            ],
            16 => [
                word[2].clone(),
                word[3].clone(),
                word[0].clone(),
                word[1].clone(),
            ],
            _ => Self::zero_word_expr(),
        }
    }

    pub(super) fn byte_hex_expr(expr: &str) -> String {
        let b = format!("mod(calc(({}) + 256), 256)", expr);
        let hi = format!("round(down, calc(({}) / 16))", b);
        let lo = format!("mod(({}), 16)", b);
        format!("--hex({}) --hex({})", hi, lo)
    }

    fn word_hex_body_expr(now: &HashMap<u16, String>, w: Word) -> String {
        let b3 = Self::byte_hex_expr(&Self::val_expr(now, w.b3));
        let b2 = Self::byte_hex_expr(&Self::val_expr(now, w.b2));
        let b1 = Self::byte_hex_expr(&Self::val_expr(now, w.b1));
        let b0 = Self::byte_hex_expr(&Self::val_expr(now, w.b0));
        format!("{} {} {} {}", b3, b2, b1, b0)
    }

    pub(super) fn word_hex_expr(now: &HashMap<u16, String>, w: Word) -> String {
        format!("\"0x\" {}", Self::word_hex_body_expr(now, w))
    }

    fn emit_exit_value(
        reg_now: &mut HashMap<u16, String>,
        val: Option<crate::ir8::ValueWords>,
    ) -> Option<String> {
        let exit_code_expr = val.map(|w| Self::value_hex_expr(reg_now, w));
        if let Some(w) = val {
            for (i, expr) in Self::word_bytes_expr(reg_now, w.lo).into_iter().enumerate() {
                reg_now.insert(i as u16, expr);
            }
            if let Some(hi) = w.hi {
                for (i, expr) in Self::word_bytes_expr(reg_now, hi).into_iter().enumerate() {
                    reg_now.insert(4 + i as u16, expr);
                }
            }
        }
        exit_code_expr
    }

    pub(super) fn value_hex_expr(
        now: &HashMap<u16, String>,
        value: crate::ir8::ValueWords,
    ) -> String {
        if let Some(hi) = value.hi {
            format!(
                "\"0x\" {} {}",
                Self::word_hex_body_expr(now, hi),
                Self::word_hex_body_expr(now, value.lo)
            )
        } else {
            Self::word_hex_expr(now, value.lo)
        }
    }

    pub(super) fn coprocessor_setup_for_builtin_call(
        &self,
        builtin: crate::ir8::BuiltinId,
        args: &[Word],
        now: &HashMap<u16, String>,
    ) -> Option<JsCoprocessorSetup> {
        if !self.js_coprocessor {
            return None;
        }
        let op_code = builtin.coprocessor_opcode();
        let lhs = args
            .first()
            .map(|w| Self::word_bytes_expr(now, *w))
            .unwrap_or_else(Self::zero_word_expr);
        let rhs = args
            .get(1)
            .map(|w| Self::word_bytes_expr(now, *w))
            .unwrap_or_else(Self::zero_word_expr);
        Some(JsCoprocessorSetup { op_code, lhs, rhs })
    }

    pub(super) fn coprocessor_output_word_expr() -> [String; 4] {
        [
            "var(--cop_o0)".to_string(),
            "var(--cop_o1)".to_string(),
            "var(--cop_o2)".to_string(),
            "var(--cop_o3)".to_string(),
        ]
    }

    pub(super) fn coprocessor_trap_expr(builtin: crate::ir8::BuiltinId) -> String {
        match builtin {
            crate::ir8::BuiltinId::DivU32
            | crate::ir8::BuiltinId::RemU32
            | crate::ir8::BuiltinId::DivS32
            | crate::ir8::BuiltinId::RemS32 => "--lt(var(--cop_o0), 0)".to_string(),
            _ => "0".to_string(),
        }
    }

    pub(super) fn eval_builtin(
        &self,
        builtin: crate::ir8::BuiltinId,
        args: &[Word],
        now: &HashMap<u16, String>,
    ) -> ([String; 4], String) {
        if self.js_coprocessor {
            return (
                Self::coprocessor_output_word_expr(),
                Self::coprocessor_trap_expr(builtin),
            );
        }

        let lhs = args
            .first()
            .map(|w| Self::word_bytes_expr(now, *w))
            .unwrap_or_else(Self::zero_word_expr);
        let rhs = args
            .get(1)
            .map(|w| Self::word_bytes_expr(now, *w))
            .unwrap_or_else(Self::zero_word_expr);

        let s_bits = (0..5u8)
            .map(|k| Self::byte_bit_expr(&rhs[0], k))
            .collect::<Vec<_>>();

        let (ret, trap) = match builtin {
            crate::ir8::BuiltinId::DivU32
            | crate::ir8::BuiltinId::RemU32
            | crate::ir8::BuiltinId::DivS32
            | crate::ir8::BuiltinId::RemS32 => {
                // Div/rem builtins are lowered to explicit IR8 microcode now.
                // If one reaches emit-time builtin dispatch, trap explicitly.
                (Self::zero_word_expr(), "1".to_string())
            }
            crate::ir8::BuiltinId::Shl32 => {
                let mut out = lhs.clone();
                for (idx, amount) in [1u8, 2, 4, 8, 16].into_iter().enumerate() {
                    let sh = Self::shl_stage_expr(&out, amount);
                    out = Self::word_sel_expr(&s_bits[idx], sh, out);
                }
                (out, "0".to_string())
            }
            crate::ir8::BuiltinId::ShrU32 => {
                let mut out = lhs.clone();
                for (idx, amount) in [1u8, 2, 4, 8, 16].into_iter().enumerate() {
                    let sh = Self::shr_u_stage_expr(&out, amount);
                    out = Self::word_sel_expr(&s_bits[idx], sh, out);
                }
                (out, "0".to_string())
            }
            crate::ir8::BuiltinId::ShrS32 => {
                let mut out = lhs.clone();
                for (idx, amount) in [1u8, 2, 4, 8, 16].into_iter().enumerate() {
                    let sh = Self::shr_s_stage_expr(&out, amount);
                    out = Self::word_sel_expr(&s_bits[idx], sh, out);
                }
                (out, "0".to_string())
            }
            crate::ir8::BuiltinId::Rotl32 => {
                let mut out = lhs.clone();
                for (idx, amount) in [1u8, 2, 4, 8, 16].into_iter().enumerate() {
                    let sh = Self::rotl_stage_expr(&out, amount);
                    out = Self::word_sel_expr(&s_bits[idx], sh, out);
                }
                (out, "0".to_string())
            }
            crate::ir8::BuiltinId::Rotr32 => {
                let mut out = lhs.clone();
                for (idx, amount) in [1u8, 2, 4, 8, 16].into_iter().enumerate() {
                    let sh = Self::rotr_stage_expr(&out, amount);
                    out = Self::word_sel_expr(&s_bits[idx], sh, out);
                }
                (out, "0".to_string())
            }
            crate::ir8::BuiltinId::Clz32 => {
                let c3 = Self::byte_clz_expr(&lhs[3]);
                let c2 = format!("calc(8 + ({}))", Self::byte_clz_expr(&lhs[2]));
                let c1 = format!("calc(16 + ({}))", Self::byte_clz_expr(&lhs[1]));
                let c0 = format!("calc(24 + ({}))", Self::byte_clz_expr(&lhs[0]));
                let nz3 = format!("--ne({}, 0)", lhs[3]);
                let nz2 = format!("--ne({}, 0)", lhs[2]);
                let nz1 = format!("--ne({}, 0)", lhs[1]);
                let z3 = format!("calc(1 - ({}))", nz3);
                let z2 = format!("calc(1 - ({}))", nz2);
                let z1 = format!("calc(1 - ({}))", nz1);
                let out0 = format!(
                    "calc(({}) * ({}) + (({}) * ({})) * ({}) + (({}) * ({}) * ({})) * ({}) + (({}) * ({}) * ({})) * ({}))",
                    nz3, c3, z3, nz2, c2, z3, z2, nz1, c1, z3, z2, z1, c0
                );
                (
                    [out0, "0".to_string(), "0".to_string(), "0".to_string()],
                    "0".to_string(),
                )
            }
            crate::ir8::BuiltinId::Ctz32 => {
                let c0 = Self::byte_ctz_expr(&lhs[0]);
                let c1 = format!("calc(8 + ({}))", Self::byte_ctz_expr(&lhs[1]));
                let c2 = format!("calc(16 + ({}))", Self::byte_ctz_expr(&lhs[2]));
                let c3 = format!("calc(24 + ({}))", Self::byte_ctz_expr(&lhs[3]));
                let nz0 = format!("--ne({}, 0)", lhs[0]);
                let nz1 = format!("--ne({}, 0)", lhs[1]);
                let nz2 = format!("--ne({}, 0)", lhs[2]);
                let z0 = format!("calc(1 - ({}))", nz0);
                let z1 = format!("calc(1 - ({}))", nz1);
                let z2 = format!("calc(1 - ({}))", nz2);
                let out0 = format!(
                    "calc(({}) * ({}) + (({}) * ({})) * ({}) + (({}) * ({}) * ({})) * ({}) + (({}) * ({}) * ({})) * ({}))",
                    nz0, c0, z0, nz1, c1, z0, z1, nz2, c2, z0, z1, z2, c3
                );
                (
                    [out0, "0".to_string(), "0".to_string(), "0".to_string()],
                    "0".to_string(),
                )
            }
            crate::ir8::BuiltinId::Popcnt32 => {
                let out0 = format!(
                    "calc(({}) + ({}) + ({}) + ({}))",
                    Self::byte_popcnt_expr(&lhs[0]),
                    Self::byte_popcnt_expr(&lhs[1]),
                    Self::byte_popcnt_expr(&lhs[2]),
                    Self::byte_popcnt_expr(&lhs[3])
                );
                (
                    [out0, "0".to_string(), "0".to_string(), "0".to_string()],
                    "0".to_string(),
                )
            }
        };

        (ret, trap)
    }

    pub(super) fn bitwise_expr(op: &str, l: &str, r: &str) -> String {
        let mut terms = Vec::with_capacity(8);
        for k in 0..8u32 {
            let p2 = 1u32 << k;
            let lb = format!("mod(round(down, calc(({}) / {})), 2)", l, p2);
            let rb = format!("mod(round(down, calc(({}) / {})), 2)", r, p2);
            let bit = match op {
                "and" => format!("calc(({}) * ({}))", lb, rb),
                "or" => format!("min(1, calc(({}) + ({})))", lb, rb),
                "xor" => format!("mod(calc(({}) + ({})), 2)", lb, rb),
                _ => "0".to_string(),
            };
            terms.push(format!("calc(({}) * {})", bit, p2));
        }
        format!("calc({})", terms.join(" + "))
    }
}
