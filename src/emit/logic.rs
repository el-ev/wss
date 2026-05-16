use super::*;

#[derive(Clone, Copy)]
pub(super) enum BitwiseOp {
    And,
    Or,
    Xor,
}

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
        let mut mb_arms = vec![String::new(); self.max_mem_addr_slots];

        let global_bool_regs = Self::collect_bool_regs(self.program);

        for cycle in &self.program.cycles {
            let pc = cycle.pc.index();
            let mut reg_now = HashMap::new();
            let mut bool_regs: std::collections::HashSet<u16> = global_bool_regs.clone();
            let mut addr_counts: HashMap<crate::ir8::Addr, usize> = HashMap::new();
            for op in &cycle.ops {
                match op.kind {
                    Inst8Kind::LoadMem { addr, .. } | Inst8Kind::StoreMem { addr, .. } => {
                        *addr_counts.entry(addr).or_insert(0) += 1;
                    }
                    _ => {}
                }
            }
            let mut addr_slot: HashMap<crate::ir8::Addr, usize> = HashMap::new();
            let mut next_addr_slot: usize = 0;
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
                            if Self::val_is_bool(&bool_regs, *s) {
                                bool_regs.insert(dst.expect_vreg());
                            }
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
                            bool_regs.insert(dst.expect_vreg());
                        }
                    }
                    Inst8Kind::Add(l, r) => {
                        if let Some(dst) = op.dst {
                            let lhs = Self::val_expr(&reg_now, *l);
                            let rhs = Self::val_expr(&reg_now, *r);
                            let total = Self::fold_add(vec![lhs, rhs]);
                            let e = Self::fold_mod(&total, 256);
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::Carry(l, r) => {
                        if let Some(dst) = op.dst {
                            let lhs = Self::paren_if_needed(&Self::val_expr(&reg_now, *l));
                            let rhs = Self::paren_if_needed(&Self::val_expr(&reg_now, *r));
                            let e = format!("--lt(255, calc({lhs} + {rhs}))");
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::Sub(l, r) => {
                        if let Some(dst) = op.dst {
                            let lhs = Self::val_expr(&reg_now, *l);
                            let rhs = Self::val_expr(&reg_now, *r);
                            // lhs - rhs + 256 with literal folding.
                            let e = if let (Ok(ln), Ok(rn)) =
                                (lhs.parse::<i64>(), rhs.parse::<i64>())
                            {
                                (ln - rn).rem_euclid(256).to_string()
                            } else {
                                let lp = Self::paren_if_needed(&lhs);
                                let rp = Self::paren_if_needed(&rhs);
                                let total = format!("calc({lp} - {rp} + 256)");
                                Self::fold_mod(&total, 256)
                            };
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::MulLo(l, r) => {
                        if let Some(dst) = op.dst {
                            let lhs = Self::val_expr(&reg_now, *l);
                            let rhs = Self::val_expr(&reg_now, *r);
                            let e = Self::fold_mod(&Self::fold_mul(&lhs, &rhs), 256);
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::MulHi(l, r) => {
                        if let Some(dst) = op.dst {
                            let lhs = Self::paren_if_needed(&Self::val_expr(&reg_now, *l));
                            let rhs = Self::paren_if_needed(&Self::val_expr(&reg_now, *r));
                            // Two byte operands give a product <= 65025, so
                            // round-down by 256 already lies in 0..=254 and
                            // the outer `mod(_, 256)` is redundant.
                            let e = format!("round(down, calc({lhs} * {rhs} / 256))");
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::And8(l, r) => {
                        if let Some(dst) = op.dst {
                            let e = Self::bitwise_expr(
                                BitwiseOp::And,
                                &Self::val_expr(&reg_now, *l),
                                &Self::val_expr(&reg_now, *r),
                            );
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::Or8(l, r) => {
                        if let Some(dst) = op.dst {
                            let e = Self::bitwise_expr(
                                BitwiseOp::Or,
                                &Self::val_expr(&reg_now, *l),
                                &Self::val_expr(&reg_now, *r),
                            );
                            reg_now.insert(dst.expect_vreg(), e);
                        }
                    }
                    Inst8Kind::Xor8(l, r) => {
                        if let Some(dst) = op.dst {
                            let e = Self::bitwise_expr(
                                BitwiseOp::Xor,
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
                                Self::eq_expr(
                                    &Self::val_expr(&reg_now, *l),
                                    &Self::val_expr(&reg_now, *r),
                                ),
                            );
                            bool_regs.insert(dst.expect_vreg());
                        }
                    }
                    Inst8Kind::Ne(l, r) => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                Self::ne_expr(
                                    &Self::val_expr(&reg_now, *l),
                                    &Self::val_expr(&reg_now, *r),
                                ),
                            );
                            bool_regs.insert(dst.expect_vreg());
                        }
                    }
                    Inst8Kind::LtU(l, r) => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                Self::lt_expr(
                                    &Self::val_expr(&reg_now, *l),
                                    &Self::val_expr(&reg_now, *r),
                                ),
                            );
                            bool_regs.insert(dst.expect_vreg());
                        }
                    }
                    Inst8Kind::GeU(l, r) => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                format!(
                                    "--ge({}, {})",
                                    Self::val_expr(&reg_now, *l),
                                    Self::val_expr(&reg_now, *r)
                                ),
                            );
                            bool_regs.insert(dst.expect_vreg());
                        }
                    }
                    Inst8Kind::BoolAnd(bool_op) => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                Self::bool_nary_expr(&reg_now, bool_op, true),
                            );
                            bool_regs.insert(dst.expect_vreg());
                        }
                    }
                    Inst8Kind::BoolOr(bool_op) => {
                        if let Some(dst) = op.dst {
                            reg_now.insert(
                                dst.expect_vreg(),
                                Self::bool_nary_expr(&reg_now, bool_op, false),
                            );
                            bool_regs.insert(dst.expect_vreg());
                        }
                    }
                    Inst8Kind::BoolNot(s) => {
                        if let Some(dst) = op.dst {
                            let inner = Self::val_expr(&reg_now, *s);
                            let body = if Self::val_is_bool(&bool_regs, *s) {
                                inner
                            } else {
                                format!("min(1, {})", inner)
                            };
                            reg_now.insert(dst.expect_vreg(), format!("calc(1 - {})", body));
                            bool_regs.insert(dst.expect_vreg());
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
                            let slot = self.assign_addr_slot(
                                addr,
                                pc,
                                &reg_now,
                                &addr_counts,
                                &mut addr_slot,
                                &mut next_addr_slot,
                                &mut mb_arms,
                            );
                            let (addr, in_bounds) = self.mem_addr_bounds(
                                &reg_now,
                                addr,
                                *base,
                                *lane,
                                slot,
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
                        let slot = self.assign_addr_slot(
                            addr,
                            pc,
                            &reg_now,
                            &addr_counts,
                            &mut addr_slot,
                            &mut next_addr_slot,
                            &mut mb_arms,
                        );
                        let (byte_addr, in_bounds) = self.mem_addr_bounds(
                            &reg_now,
                            addr,
                            *base,
                            *lane,
                            slot,
                            &mut trap_mem_parts,
                        );
                        mem_stores_raw.push(MemStoreByte {
                            cell: format!("--mhalf({})", byte_addr),
                            parity: format!("--mpar({})", byte_addr),
                            val: Self::fold_mod(&Self::val_expr(&reg_now, *val), 256),
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
                    Inst8Kind::RandomByte { lane } => {
                        if let Some(dst) = op.dst {
                            let a = lane;
                            let b = (lane + 1) & 3;
                            let c = (lane + 2) & 3;
                            reg_now.insert(
                                dst.expect_vreg(),
                                format!(
                                    "mod(calc(var(--rng{a}, 0) * var(--rng{b}, 0) + var(--rng{c}, 0) + 1), 256)"
                                ),
                            );
                        }
                    }
                    Inst8Kind::CsStore { offset, val } => {
                        let slot_off = (*offset) / 2;
                        let parity = (*offset) % 2;
                        let idx = Self::var_with_offset("var(--_1cs_sp)", slot_off as i64);
                        let ok = self.cs_bounds_check(&idx, false, &mut trap_cs_parts);
                        cs_stores.push(CsStore {
                            idx,
                            parity: format!("{}", parity),
                            val: Self::fold_mod(&Self::val_expr(&reg_now, *val), 256),
                            ok,
                        });
                    }
                    Inst8Kind::CsLoad { offset } => {
                        if let Some(dst) = op.dst {
                            let slot_off = (*offset) / 2;
                            let parity = (*offset) % 2;
                            let idx = Self::var_with_offset("var(--_1cs_sp)", slot_off as i64);
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
                        let idx = Self::var_with_offset("var(--_1cs_sp)", *offset as i64);
                        let ok = self.cs_bounds_check(&idx, false, &mut trap_cs_parts);
                        cs_stores.push(CsStore {
                            idx,
                            parity: "2".to_string(),
                            val: format!("{}", val.index()),
                            ok,
                        });
                    }
                    Inst8Kind::CsLoadPc { offset } => {
                        let idx = Self::var_with_offset("var(--_1cs_sp)", *offset as i64);
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
                        let next_sp = Self::var_with_offset("var(--_1cs_sp)", *size as i64);
                        let ok = self.cs_bounds_check(&next_sp, true, &mut trap_cs_parts);
                        cs_sp_expr = Some(Self::sel_expr(&ok, &next_sp, "var(--_1cs_sp)"));
                    }
                    Inst8Kind::CsFree(size) => {
                        let next_sp = Self::var_with_offset("var(--_1cs_sp)", -(*size as i64));
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

            Self::dedupe_bounds_checks(&mut trap_mem_parts);
            let trap_mem = Self::clamp_sum_or_zero(&trap_mem_parts);

            if trap_mem != "0" {
                term.pc_expr = Self::sel_expr(
                    &trap_mem,
                    &TrapCode::InvalidMemoryAccess.pc().to_string(),
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
                Self::dedupe_bounds_checks(&mut trap_cs_parts);
                let trap_cs = Self::clamp_sum_or_zero(&trap_cs_parts);

                if trap_cs != "0" {
                    term.pc_expr = Self::sel_expr(
                        &trap_cs,
                        &TrapCode::CallstackOverflow.pc().to_string(),
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
                    "style(--_1pc: {}): --eq(var(--kb, -1), -1); ",
                    pc
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

        let mut slot_consts = MemSlotConsts::default();
        slot_consts.ms_val.resize(self.max_mem_store_slots, None);
        slot_consts.ms_val_b.resize(self.max_mem_store_slots, None);
        for s in 0..self.max_mem_store_slots {
            // All slot indicators default to 0: their consumers gate on the
            // companion `--msoN` active bit, which is also 0 when the slot
            // is not driven this cycle.
            Self::emit_if_or_fallback(out, &format!("--msc{}", s), &ms_cell_arms[s], "0");
            Self::emit_if_or_fallback(out, &format!("--msp{}", s), &ms_par_arms[s], "0");
            Self::emit_if_or_fallback(out, &format!("--msv{}", s), &ms_val_arms[s], "0");
            Self::emit_if_or_fallback(out, &format!("--mso{}", s), &ms_ok_arms[s], "0");
            Self::emit_if_or_fallback(out, &format!("--msc{}b", s), &ms_cell_arms_b[s], "0");
            Self::emit_if_or_fallback(out, &format!("--msp{}b", s), &ms_par_arms_b[s], "0");
            Self::emit_if_or_fallback(out, &format!("--msv{}b", s), &ms_val_arms_b[s], "0");
            Self::emit_if_or_fallback(out, &format!("--mso{}b", s), &ms_ok_arms_b[s], "0");
            slot_consts.ms_val[s] = Self::arms_active_constant(&ms_val_arms[s], "0");
            slot_consts.ms_val_b[s] = Self::arms_active_constant(&ms_val_arms_b[s], "0");
        }
        let _ = self.mem_slot_consts.set(slot_consts);

        if self.max_mem_store_slots > 0 {
            for page in 0..self.page_count() {
                let expr = self.dirty_page_expr(page);
                let _ = writeln!(out, " --mwdp{}: {};", page, expr);
            }
        }

        Self::emit_slot_with_cse(out, "mri", &mr_idx_arms, "0");
        Self::emit_slot_with_cse(out, "mro", &mr_ok_arms, "0");

        for (s, arms) in mb_arms.iter().enumerate() {
            Self::emit_if_or_fallback(out, &format!("--mb{}", s), arms, "0");
        }

        if self.js_coprocessor {
            Self::emit_if_or_fallback(out, "--cop_op", &cop_op_arms, &format!("{}", COP_OP_NONE));
            for lane in 0..4usize {
                Self::emit_if_or_fallback(out, &format!("--cop_a{}", lane), &cop_a_arms[lane], "0");
                Self::emit_if_or_fallback(out, &format!("--cop_b{}", lane), &cop_b_arms[lane], "0");
            }
        }

        if self.uses_callstack {
            Self::emit_slot_with_cse(out, "csi", &csw_idx_arms, "0");
            for s in 0..self.max_cs_store_slots {
                Self::emit_if_or_fallback(out, &format!("--csp{}", s), &csw_par_arms[s], "0");
                Self::emit_if_or_fallback(out, &format!("--csv{}", s), &csw_val_arms[s], "0");
            }
            Self::emit_slot_with_cse(out, "cso", &csw_ok_arms, "0");
            if self.max_cs_store_slots > 0 {
                for page in 0..self.cs_page_count() {
                    // cs_dirty_page_expr evaluates to 0 whenever no csoN is set,
                    // so the explicit `if(csw_active: 1: ...; else: 0)` guard
                    // is redundant.
                    let expr = self.cs_dirty_page_expr(page);
                    let _ = writeln!(out, " --cswdp{}: {};", page, expr);
                }
            }
            Self::emit_slot_with_cse(out, "cri", &csr_idx_arms, "0");
            for s in 0..self.max_cs_read_slots {
                Self::emit_if_or_fallback(out, &format!("--crp{}", s), &csr_par_arms[s], "0");
            }
            Self::emit_slot_with_cse(out, "cro", &csr_ok_arms, "0");
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
                // mwdpN = 1 implies some mso* is 1 at this PC, which implies
                // mw_active = 1; so the outer `mw_active` check is redundant.
                format!("if(style(--mwdp{}: 1): {}; else: {})", page, merged, prev)
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
                    // cswdpN = 1 implies some csoN is 1 at this PC, which
                    // implies csw_active = 1; so the outer check is redundant.
                    format!("if(style(--cswdp{}: 1): {}; else: {})", page, merged, prev)
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
                        &Self::eq_expr(&idx, &i.to_string()),
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
                    let idx = "var(--_1cs_sp)";
                    let ok = Self::inrange_expr(idx, self.cs_names.len() as i64);
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
                pc_expr: code.pc().to_string(),
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

    /// Peels redundant `calc(…)` and `min(1, …)` wrappers from a truthy-only
    /// context (e.g. a `--sel` condition): `min(1, X) == 0` iff `X == 0` for
    /// non-negative X, and `--sel` only cares about the zero / nonzero
    /// distinction. We only strip a wrapper when the remaining inner
    /// expression is itself a valid CSS value at the outermost level (a
    /// number, `var(…)`, or function call) — otherwise we'd be left with a
    /// bare arithmetic expression that isn't a legal `--sel` argument.
    fn strip_truthy_wrappers(s: &str) -> String {
        let mut cur = s.trim().to_string();
        loop {
            let next = if let Some(inner) =
                cur.strip_prefix("calc(").and_then(|t| t.strip_suffix(')'))
                && Self::balanced_parens(inner)
                && Self::is_atomic_css_value(inner.trim())
            {
                inner.trim().to_string()
            } else if let Some(inner) = cur
                .strip_prefix("min(1, ")
                .and_then(|t| t.strip_suffix(')'))
                && Self::balanced_parens(inner)
                && Self::is_atomic_css_value(inner.trim())
            {
                inner.trim().to_string()
            } else {
                return cur;
            };
            if next == cur {
                return cur;
            }
            cur = next;
        }
    }

    /// True when `s` is a valid stand-alone CSS value at the outer level —
    /// i.e. a numeric literal, a `var(…)`, or a `name(…)` function call.
    fn is_atomic_css_value(s: &str) -> bool {
        let t = s.trim();
        if t.is_empty() {
            return false;
        }
        // Numeric literal (optionally signed, may include decimal).
        if t.bytes()
            .all(|b| b.is_ascii_digit() || b == b'.' || b == b'-' || b == b'+')
        {
            return true;
        }
        // Function-call form: `<ident>(<balanced>)` covering the whole string.
        let bytes = t.as_bytes();
        let mut i = 0;
        if i < bytes.len() && bytes[i] == b'-' && i + 1 < bytes.len() && bytes[i + 1] == b'-' {
            i += 2;
        }
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_' || bytes[i] == b'-')
        {
            i += 1;
        }
        if i == 0 || i >= bytes.len() || bytes[i] != b'(' {
            return false;
        }
        let mut depth = 0i32;
        for (j, b) in bytes.iter().enumerate().skip(i) {
            match b {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        return j == bytes.len() - 1;
                    }
                }
                _ => {}
            }
        }
        false
    }

    fn balanced_parens(s: &str) -> bool {
        let mut depth = 0i32;
        for b in s.bytes() {
            match b {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth < 0 {
                        return false;
                    }
                }
                _ => {}
            }
        }
        depth == 0
    }

    pub(super) fn val_is_bool(bool_regs: &std::collections::HashSet<u16>, v: Val8) -> bool {
        match v.imm_value() {
            Some(x) => x <= 1,
            None => bool_regs.contains(&v.expect_vreg()),
        }
    }

    fn collect_bool_regs(program: &crate::ir8::Ir8Program) -> std::collections::HashSet<u16> {
        let mut bool_regs: std::collections::HashSet<u16> = std::collections::HashSet::new();
        // Iterate to a fixed point so derived rules (And8 with bool, Copy-of-bool,
        // Sel between two bools, etc.) propagate.
        loop {
            let mut changed = false;
            let imm_bool = |v: Val8, set: &std::collections::HashSet<u16>| -> bool {
                match v.imm_value() {
                    Some(x) => x <= 1,
                    None => set.contains(&v.expect_vreg()),
                }
            };
            for cycle in &program.cycles {
                for op in &cycle.ops {
                    let Some(dst) = op.dst else { continue };
                    let dst_v = dst.expect_vreg();
                    if bool_regs.contains(&dst_v) {
                        continue;
                    }
                    let is_bool = match op.kind {
                        Inst8Kind::Eq(_, _)
                        | Inst8Kind::Ne(_, _)
                        | Inst8Kind::LtU(_, _)
                        | Inst8Kind::GeU(_, _)
                        | Inst8Kind::BoolAnd(_)
                        | Inst8Kind::BoolOr(_)
                        | Inst8Kind::BoolNot(_)
                        | Inst8Kind::Sub32Borrow { .. }
                        | Inst8Kind::Carry(_, _) => true,
                        Inst8Kind::Copy(s) => imm_bool(s, &bool_regs),
                        Inst8Kind::And8(a, b) => imm_bool(a, &bool_regs) || imm_bool(b, &bool_regs),
                        Inst8Kind::Or8(a, b) | Inst8Kind::Xor8(a, b) => {
                            imm_bool(a, &bool_regs) && imm_bool(b, &bool_regs)
                        }
                        Inst8Kind::Sel(_, t, f) => {
                            imm_bool(t, &bool_regs) && imm_bool(f, &bool_regs)
                        }
                        _ => false,
                    };
                    if is_bool && bool_regs.insert(dst_v) {
                        changed = true;
                    }
                }
            }
            if !changed {
                break;
            }
        }
        bool_regs
    }

    pub(super) fn eq_expr(l: &str, r: &str) -> String {
        if let (Ok(a), Ok(b)) = (l.trim().parse::<i64>(), r.trim().parse::<i64>()) {
            return format!("{}", (a == b) as i32);
        }
        match (l, r) {
            (a, "0") | ("0", a) => format!("--eqz({})", a),
            (a, "1") | ("1", a) => format!("--eq1({})", a),
            _ => format!("--eq({}, {})", l, r),
        }
    }

    pub(super) fn ne_expr(l: &str, r: &str) -> String {
        if let (Ok(a), Ok(b)) = (l.trim().parse::<i64>(), r.trim().parse::<i64>()) {
            return format!("{}", (a != b) as i32);
        }
        match (l, r) {
            (a, "0") | ("0", a) => format!("--nez({})", a),
            _ => format!("--ne({}, {})", l, r),
        }
    }

    /// Formats `var(name) + imm`, dropping the `calc()` wrapper and `+ 0`
    /// when the offset is zero. Negative offsets use `- |imm|`.
    pub(super) fn var_with_offset(var: &str, imm: i64) -> String {
        if imm == 0 {
            var.to_string()
        } else if imm > 0 {
            format!("calc({} + {})", var, imm)
        } else {
            format!("calc({} - {})", var, -imm)
        }
    }

    /// Parses an expression of the form `BASE`, `calc(BASE + N)`, or
    /// `calc(BASE - N)` into its base and integer offset. Returns `None` if
    /// the expression is not a calc-with-literal-offset shape. Bare
    /// expressions return offset 0.
    pub(super) fn try_split_calc_offset(s: &str) -> Option<(String, i64)> {
        let t = s.trim();
        let Some(inner) = t.strip_prefix("calc(").and_then(|x| x.strip_suffix(')')) else {
            return Some((t.to_string(), 0));
        };
        let bytes = inner.as_bytes();
        let mut depth: i32 = 0;
        let mut split: Option<(usize, bool)> = None;
        let mut i = 0;
        while i + 2 < bytes.len() {
            match bytes[i] {
                b'(' => depth += 1,
                b')' => depth -= 1,
                b' ' if depth == 0
                    && (bytes[i + 1] == b'+' || bytes[i + 1] == b'-')
                    && bytes[i + 2] == b' ' =>
                {
                    split = Some((i, bytes[i + 1] == b'-'));
                }
                _ => {}
            }
            i += 1;
        }
        let (pos, is_neg) = split?;
        let lhs = inner[..pos].trim();
        let rhs = inner[pos + 3..].trim();
        let n: i64 = rhs.parse().ok()?;
        Some((lhs.to_string(), if is_neg { -n } else { n }))
    }

    /// Emits `--lt(l, r)` after folding constant offsets so a literal sits
    /// on the side opposite the variable: `--lt(calc(B + N), K)` becomes
    /// `--lt(B, K - N)`, symmetric for the swapped form.
    pub(super) fn lt_expr(l: &str, r: &str) -> String {
        let (l, r) = (l.trim(), r.trim());
        if let (Ok(a), Ok(b)) = (l.parse::<i64>(), r.parse::<i64>()) {
            return if a < b {
                "1".to_string()
            } else {
                "0".to_string()
            };
        }
        // Fold a byte-valued var against an out-of-range bound to a constant.
        if let Ok(k) = r.parse::<i64>() {
            if Self::is_byte_valued_var(l) {
                if k <= 0 {
                    return "0".to_string();
                }
                if k > 255 {
                    return "1".to_string();
                }
            }
            if let Some((base, n)) = Self::try_split_calc_offset(l)
                && n != 0
            {
                return format!("--lt({}, {})", base, k - n);
            }
        }
        if let Ok(k) = l.parse::<i64>() {
            if Self::is_byte_valued_var(r) {
                if k < 0 {
                    return "1".to_string();
                }
                if k >= 255 {
                    return "0".to_string();
                }
            }
            if let Some((base, n)) = Self::try_split_calc_offset(r)
                && n != 0
            {
                return format!("--lt({}, {})", k - n, base);
            }
        }
        format!("--lt({}, {})", l, r)
    }

    /// Like `lt_expr` but for `--ge`.
    pub(super) fn ge_expr(l: &str, r: &str) -> String {
        let (l, r) = (l.trim(), r.trim());
        if let (Ok(a), Ok(b)) = (l.parse::<i64>(), r.parse::<i64>()) {
            return if a >= b {
                "1".to_string()
            } else {
                "0".to_string()
            };
        }
        if let Ok(k) = r.parse::<i64>() {
            if Self::is_byte_valued_var(l) {
                if k <= 0 {
                    return "1".to_string();
                }
                if k > 255 {
                    return "0".to_string();
                }
            }
            if let Some((base, n)) = Self::try_split_calc_offset(l)
                && n != 0
            {
                return format!("--ge({}, {})", base, k - n);
            }
        }
        if let Ok(k) = l.parse::<i64>() {
            if Self::is_byte_valued_var(r) {
                if k < 0 {
                    return "0".to_string();
                }
                if k >= 255 {
                    return "1".to_string();
                }
            }
            if let Some((base, n)) = Self::try_split_calc_offset(r)
                && n != 0
            {
                return format!("--ge({}, {})", k - n, base);
            }
        }
        format!("--ge({}, {})", l, r)
    }

    /// Emits `--inrange(idx, k)`. When `idx` has a literal offset we inline
    /// the definition so the offset folds into both bounds and the calc()
    /// wrapper disappears from the per-argument evaluation.
    pub(super) fn inrange_expr(idx: &str, k: i64) -> String {
        // Strip a trivial `+ 0` wrapper but keep the `--inrange` function
        // call; CSS evaluates the calc argument once into a local, while
        // inlining the definition would re-read the base in every --lt.
        if let Some((base, 0)) = Self::try_split_calc_offset(idx) {
            return format!("--inrange({}, {})", base, k);
        }
        format!("--inrange({}, {})", idx, k)
    }

    pub(super) fn paren_if_needed(expr: &str) -> String {
        if Self::is_atomic_calc_term(expr) {
            expr.to_string()
        } else {
            format!("({})", expr)
        }
    }

    /// Prepares `expr` for inclusion inside a surrounding `calc(...)`. When the
    /// expression itself is already `calc(BODY)`, rewrites it as `(BODY)` so
    /// we avoid emitting `calc(calc(BODY) ...)` redundancies.
    /// Returns the boolean negation `1 - b`, folding when `b` is a literal
    /// `"0"` or `"1"`. Avoids emitting `calc(1 - (0))`-style trivia.
    pub(super) fn flip_bool(b: &str) -> String {
        match b.trim() {
            "0" => "1".to_string(),
            "1" => "0".to_string(),
            other => format!("calc(1 - {})", Self::calc_inner(other)),
        }
    }

    pub(super) fn calc_inner(expr: &str) -> String {
        let t = expr.trim();
        if let Some(inner) = Self::peel_calc(t) {
            return format!("({})", inner);
        }
        Self::paren_if_needed(t)
    }

    pub(super) fn peel_calc(s: &str) -> Option<&str> {
        let inner = s.strip_prefix("calc(")?.strip_suffix(')')?;
        let mut depth: i32 = 0;
        for b in inner.bytes() {
            match b {
                b'(' => depth += 1,
                b')' => {
                    if depth == 0 {
                        return None;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
        if depth == 0 { Some(inner) } else { None }
    }

    pub(super) fn is_atomic_calc_term(expr: &str) -> bool {
        let t = expr.trim();
        let bytes = t.as_bytes();
        if bytes.is_empty() {
            return true;
        }
        {
            let mut i = 0;
            if bytes[0] == b'-' || bytes[0] == b'+' {
                i += 1;
            }
            let digit_start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            if i < bytes.len() && bytes[i] == b'.' {
                i += 1;
                while i < bytes.len() && bytes[i].is_ascii_digit() {
                    i += 1;
                }
            }
            if i == bytes.len() && i > digit_start {
                return true;
            }
        }
        let mut i = 0;
        while i < bytes.len()
            && (bytes[i].is_ascii_alphabetic()
                || bytes[i] == b'-'
                || bytes[i] == b'_'
                || (i > 0 && bytes[i].is_ascii_digit()))
        {
            i += 1;
        }
        if i == 0 || i >= bytes.len() || bytes[i] != b'(' || *bytes.last().unwrap() != b')' {
            return false;
        }
        let mut depth: i32 = 0;
        for (idx, b) in bytes.iter().enumerate().skip(i) {
            if *b == b'(' {
                depth += 1;
            } else if *b == b')' {
                depth -= 1;
                if depth == 0 && idx != bytes.len() - 1 {
                    return false;
                }
            }
        }
        depth == 0
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
            .map(|term| Self::paren_if_needed(term))
            .collect::<Vec<_>>()
            .join(if and { " * " } else { " + " });
        if and {
            // Product of bools stays in {0, 1}; no clamp needed.
            format!("calc({joined})")
        } else {
            format!("calc(min(1, {joined}))")
        }
    }

    fn mem_addr_bounds(
        &self,
        reg_now: &HashMap<u16, String>,
        addr: &crate::ir8::Addr,
        base: u16,
        lane: u8,
        slot: Option<usize>,
        trap_parts: &mut Vec<String>,
    ) -> (String, String) {
        let imm = (base as u32) + (lane as u32);
        let byte_addr = match slot {
            Some(s) if imm == 0 => format!("var(--mb{})", s),
            Some(s) => format!("calc(var(--mb{}) + {})", s, imm),
            None => format!(
                "--addr16({}, {}, {})",
                Self::val_expr(reg_now, addr.lo),
                Self::val_expr(reg_now, addr.hi),
                imm
            ),
        };
        let mem_end = self.memory_end.to_string();
        let in_bounds = Self::lt_expr(&byte_addr, &mem_end);
        trap_parts.push(Self::ge_expr(&byte_addr, &mem_end));
        (byte_addr, in_bounds)
    }

    #[allow(clippy::too_many_arguments)]
    fn assign_addr_slot(
        &self,
        addr: &crate::ir8::Addr,
        pc: u16,
        reg_now: &HashMap<u16, String>,
        addr_counts: &HashMap<crate::ir8::Addr, usize>,
        addr_slot: &mut HashMap<crate::ir8::Addr, usize>,
        next_addr_slot: &mut usize,
        mb_arms: &mut [String],
    ) -> Option<usize> {
        if addr_counts.get(addr).copied().unwrap_or(0) < 2 {
            return None;
        }
        if let Some(&s) = addr_slot.get(addr) {
            return Some(s);
        }
        let s = *next_addr_slot;
        *next_addr_slot += 1;
        addr_slot.insert(*addr, s);
        let lo = Self::val_expr(reg_now, addr.lo);
        let hi = Self::val_expr(reg_now, addr.hi);
        let _ = write!(mb_arms[s], "style(--_1pc: {}): --m16({}, {}); ", pc, lo, hi);
        Some(s)
    }

    fn cs_bounds_check(&self, idx: &str, extend: bool, trap_parts: &mut Vec<String>) -> String {
        let limit = (self.cs_names.len() + usize::from(extend)) as i64;
        let ok = Self::inrange_expr(idx, limit);
        // out-of-range = (idx < 0) + (idx >= limit); both bools, sum stays in {0, 1}.
        trap_parts.push(Self::lt_expr(idx, "0"));
        trap_parts.push(Self::ge_expr(idx, &limit.to_string()));
        ok
    }

    pub(super) fn sel_expr(cond: &str, if_true: &str, if_false: &str) -> String {
        // --sel uses style(--c: 0)/else, so it only cares whether `cond` is
        // zero — any nonzero value selects the true branch. Strip a redundant
        // `min(1, …)` clamp around the condition (and surrounding calc()) when
        // we can identify it; the inner expression has the same truthiness.
        let cond_str = Self::strip_truthy_wrappers(cond.trim());
        let cond = cond_str.as_str();
        match cond {
            "0" => return if_false.to_string(),
            "1" => return if_true.to_string(),
            _ => {}
        }
        let t = if_true.trim();
        let f = if_false.trim();
        if t == f {
            return if_true.to_string();
        }
        // `sel(calc(1 - bool), t, f)` flips its branches into `sel(bool, f, t)`,
        // letting downstream branch folds catch the bool form.
        if let Some(rest) = cond.strip_prefix("calc(1 - ")
            && let Some(inner) = rest.strip_suffix(')')
        {
            let inner_trim = inner.trim();
            let unwrapped = inner_trim
                .strip_prefix('(')
                .and_then(|s| s.strip_suffix(')'))
                .map(str::trim)
                .unwrap_or(inner_trim);
            if Self::is_bool_expr(unwrapped) {
                return Self::sel_expr(unwrapped, if_false, if_true);
            }
        }
        // `sel(bool, 1, 0)` is the bool itself; `sel(bool, 0, 1)` is its
        // logical negation. Only safe when `cond` already returns 0/1.
        if Self::is_bool_expr(cond) {
            if t == "1" && f == "0" {
                return cond.to_string();
            }
            if t == "0" && f == "1" {
                return Self::flip_bool(cond);
            }
        }
        if !cond.contains("--sel(") && !if_true.contains("--sel(") && !if_false.contains("--sel(") {
            return format!("--sel({}, {}, {})", cond, if_true, if_false);
        }
        // Drop dead multiplicands when either branch is literal `0`:
        //   sel(c, 0, f) → (1 - c) * f
        //   sel(c, t, 0) → c * t
        if t == "0" {
            return format!(
                "calc((1 - {}) * {})",
                Self::calc_inner(cond),
                Self::calc_inner(if_false),
            );
        }
        if f == "0" {
            return format!(
                "calc({} * {})",
                Self::calc_inner(cond),
                Self::calc_inner(if_true),
            );
        }
        format!(
            "calc({} * {} + (1 - {}) * {})",
            Self::calc_inner(cond),
            Self::calc_inner(if_true),
            Self::calc_inner(cond),
            Self::calc_inner(if_false),
        )
    }

    /// Conservatively detect expressions that already evaluate to 0 or 1.
    pub(super) fn is_bool_expr(s: &str) -> bool {
        let t = s.trim();
        if matches!(t, "0" | "1")
            || t.starts_with("--eq(")
            || t.starts_with("--eq1(")
            || t.starts_with("--eqz(")
            || t.starts_with("--ne(")
            || t.starts_with("--nez(")
            || t.starts_with("--lt(")
            || t.starts_with("--ge(")
            || t.starts_with("--inrange(")
            || t.starts_with("--mpar(")
        {
            return true;
        }
        // `mod(<inner>, 2)` is always 0 or 1.
        if let Some(rest) = t.strip_prefix("mod(")
            && let Some(inner) = rest.strip_suffix(", 2)")
        {
            // Ensure the trailing ", 2)" closes the mod call and isn't
            // nested inside a deeper expression.
            let mut depth = 0i32;
            for b in inner.bytes() {
                match b {
                    b'(' => depth += 1,
                    b')' => depth -= 1,
                    _ => {}
                }
                if depth < 0 {
                    return false;
                }
            }
            return depth == 0;
        }
        false
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
        let mut const_sum: i64 = 0;
        let mut var_terms: Vec<String> = Vec::new();
        for t in [lhs.trim(), rhs.trim(), carry_in.trim()] {
            if t == "0" {
                continue;
            }
            if let Ok(n) = t.parse::<i64>() {
                const_sum += n;
            } else {
                var_terms.push(t.to_string());
            }
        }
        if var_terms.is_empty() {
            return const_sum.to_string();
        }
        if const_sum == 0 && var_terms.len() == 1 {
            return var_terms.into_iter().next().unwrap();
        }
        let mut parts: Vec<String> = var_terms.iter().map(|t| Self::calc_inner(t)).collect();
        if const_sum != 0 {
            parts.push(const_sum.to_string());
        }
        format!("calc({})", parts.join(" + "))
    }

    fn byte_sub_total_expr(lhs: &str, rhs: &str, borrow_in: &str) -> String {
        // Accumulate `lhs - rhs - borrow_in` while folding any literal terms.
        let mut const_sum: i64 = 0;
        let mut pos_terms: Vec<String> = Vec::new();
        let mut neg_terms: Vec<String> = Vec::new();
        if let Ok(n) = lhs.trim().parse::<i64>() {
            const_sum += n;
        } else if lhs.trim() != "0" {
            pos_terms.push(lhs.trim().to_string());
        }
        for t in [rhs.trim(), borrow_in.trim()] {
            if t == "0" {
                continue;
            }
            if let Ok(n) = t.parse::<i64>() {
                const_sum -= n;
            } else {
                neg_terms.push(t.to_string());
            }
        }
        if pos_terms.is_empty() && neg_terms.is_empty() {
            return const_sum.to_string();
        }
        if const_sum == 0 && neg_terms.is_empty() && pos_terms.len() == 1 {
            return pos_terms.into_iter().next().unwrap();
        }
        // CSS calc() rejects leading unary minus; emit `0 - x` when only
        // negative terms remain.
        let mut out = String::from("calc(");
        if !pos_terms.is_empty() {
            out.push_str(&Self::calc_inner(&pos_terms[0]));
            for t in &pos_terms[1..] {
                out.push_str(" + ");
                out.push_str(&Self::calc_inner(t));
            }
            if const_sum > 0 {
                out.push_str(" + ");
                out.push_str(&const_sum.to_string());
            } else if const_sum < 0 {
                out.push_str(" - ");
                out.push_str(&(-const_sum).to_string());
            }
        } else {
            out.push_str(&const_sum.to_string());
        }
        for t in &neg_terms {
            out.push_str(" - ");
            out.push_str(&Self::calc_inner(t));
        }
        out.push(')');
        out
    }

    fn add32_carry_in_expr(now: &HashMap<u16, String>, lhs: Word, rhs: Word, lane: u8) -> String {
        let mut carry = "0".to_string();
        for idx in 0..lane {
            let lhs_byte = Self::val_expr(now, lhs.byte(idx));
            let rhs_byte = Self::val_expr(now, rhs.byte(idx));
            let total = Self::byte_add_total_expr(&lhs_byte, &rhs_byte, &carry);
            carry = if let Ok(n) = total.parse::<i64>() {
                (n.div_euclid(256)).to_string()
            } else if Self::is_byte_valued_var(&total) {
                // A bare byte var is in 0..=255, so floor(_ / 256) is always 0.
                "0".to_string()
            } else {
                format!("round(down, calc({} / 256))", Self::calc_inner(&total))
            };
        }
        carry
    }

    /// Computes the byte-level borrow-out for one subtraction step:
    ///   round(down, (255 - lhs + rhs + borrow_in) / 256)
    /// Folds literal operands into the leading 255 constant so e.g.
    /// `255 - var + 32` emits as `287 - var` rather than `255 - var + 32`.
    fn byte_borrow_step(lhs: &str, rhs: &str, borrow_in: &str) -> String {
        let mut const_sum: i64 = 255;
        let mut neg_terms: Vec<String> = Vec::new();
        let mut pos_terms: Vec<String> = Vec::new();
        if let Ok(l) = lhs.trim().parse::<i64>() {
            const_sum -= l;
        } else {
            neg_terms.push(lhs.trim().to_string());
        }
        for t in [rhs.trim(), borrow_in.trim()] {
            if t == "0" {
                continue;
            }
            if let Ok(n) = t.parse::<i64>() {
                const_sum += n;
            } else {
                pos_terms.push(t.to_string());
            }
        }
        if neg_terms.is_empty() && pos_terms.is_empty() {
            return const_sum.div_euclid(256).to_string();
        }
        let mut numerator = const_sum.to_string();
        for t in &pos_terms {
            numerator.push_str(" + ");
            numerator.push_str(&Self::calc_inner(t));
        }
        for t in &neg_terms {
            numerator.push_str(" - ");
            numerator.push_str(&Self::calc_inner(t));
        }
        format!("round(down, calc(({}) / 256))", numerator)
    }

    fn sub32_borrow_in_expr(now: &HashMap<u16, String>, lhs: Word, rhs: Word, lane: u8) -> String {
        let mut borrow = "0".to_string();
        for idx in 0..lane {
            let lhs_byte = Self::val_expr(now, lhs.byte(idx));
            let rhs_byte = Self::val_expr(now, rhs.byte(idx));
            borrow = Self::byte_borrow_step(&lhs_byte, &rhs_byte, &borrow);
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
        Self::fold_mod(&total, 256)
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
        // CSS `mod(A, B)` for positive B returns a result in [0, B), regardless
        // of A's sign (Euclidean modulus), so we don't need a `+ 256` shift.
        Self::fold_mod(&total, 256)
    }

    pub(super) fn sub32_borrow_expr(now: &HashMap<u16, String>, lhs: Word, rhs: Word) -> String {
        let mut borrow = "0".to_string();
        for idx in 0..4u8 {
            let lhs_byte = Self::val_expr(now, lhs.byte(idx));
            let rhs_byte = Self::val_expr(now, rhs.byte(idx));
            borrow = Self::byte_borrow_step(&lhs_byte, &rhs_byte, &borrow);
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
        let trimmed = byte.trim();
        let p2 = 1u32 << bit;
        // Constant-fold when the byte is a numeric literal — common when the
        // builder is selecting bits of an immediate shift/rotate count.
        if let Ok(v) = trimmed.parse::<i64>() {
            return ((v >> bit) & 1).to_string();
        }
        // When the input is already known to be in {0, 1}, every bit above
        // bit 0 is zero and bit 0 is the value itself.
        if Self::is_bool_expr(trimmed) {
            return if bit == 0 {
                trimmed.to_string()
            } else {
                "0".to_string()
            };
        }
        let b = Self::paren_if_needed(byte);
        if p2 == 1 {
            format!("mod({b}, 2)")
        } else {
            format!("mod(round(down, calc({b} / {p2})), 2)")
        }
    }

    pub(super) fn byte_popcnt_expr(byte: &str) -> String {
        let mut bits = Vec::with_capacity(8);
        for k in 0..8u8 {
            bits.push(Self::byte_bit_expr(byte, k));
        }
        format!("calc({})", bits.join(" + "))
    }

    pub(super) fn byte_ctz_expr(byte: &str) -> String {
        if let Ok(v) = byte.trim().parse::<i64>()
            && (0..=255).contains(&v)
        {
            let b = v as u8;
            let r = if b == 0 { 8 } else { b.trailing_zeros() };
            return format!("{}", r);
        }
        format!("--byte_ctz({})", Self::sanitize_byte_arg(byte))
    }

    pub(super) fn byte_clz_expr(byte: &str) -> String {
        if let Ok(v) = byte.trim().parse::<i64>()
            && (0..=255).contains(&v)
        {
            let b = v as u8;
            let r = if b == 0 { 8 } else { b.leading_zeros() };
            return format!("{}", r);
        }
        format!("--byte_clz({})", Self::sanitize_byte_arg(byte))
    }

    /// Normalises a byte expression for table-lookup helpers: drop the
    /// defensive `+ 256, mod 256` wrap when the input is already in 0..=255.
    fn sanitize_byte_arg(byte: &str) -> String {
        let trimmed = byte.trim();
        if Self::is_byte_valued_var(trimmed)
            || trimmed.starts_with("mod(")
            || trimmed.parse::<i64>().is_ok_and(|n| (0..=255).contains(&n))
        {
            return trimmed.to_string();
        }
        format!("mod(calc({} + 256), 256)", Self::paren_if_needed(byte))
    }

    pub(super) fn shl_stage_expr(word: &[String; 4], amount: u8) -> [String; 4] {
        match amount {
            0 => word.clone(),
            1 | 2 | 4 => {
                let p = 1u32 << amount;
                let q = 1u32 << (8 - amount);
                let lo = |x: &str| format!("mod(calc({} * {p}), 256)", Self::paren_if_needed(x));
                let carry =
                    |x: &str| format!("round(down, calc({} / {q}))", Self::paren_if_needed(x));
                let combine = |a: String, b: String| {
                    format!(
                        "mod(calc({} + {}), 256)",
                        Self::paren_if_needed(&a),
                        Self::paren_if_needed(&b)
                    )
                };
                [
                    lo(&word[0]),
                    combine(lo(&word[1]), carry(&word[0])),
                    combine(lo(&word[2]), carry(&word[1])),
                    combine(lo(&word[3]), carry(&word[2])),
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
        let b = Self::paren_if_needed(byte);
        let c = Self::paren_if_needed(carry);
        format!("mod(round(down, calc(({b} + 256 * {c}) / {p})), 256)")
    }

    pub(super) fn shr_u_stage_expr(word: &[String; 4], amount: u8) -> [String; 4] {
        match amount {
            0 => word.clone(),
            1 | 2 | 4 => {
                let p = 1u32 << amount;
                let carry = |x: &str| format!("mod({}, {p})", Self::paren_if_needed(x));
                [
                    Self::shr_byte_expr(&word[0], &carry(&word[1]), p),
                    Self::shr_byte_expr(&word[1], &carry(&word[2]), p),
                    Self::shr_byte_expr(&word[2], &carry(&word[3]), p),
                    format!(
                        "round(down, calc({} / {p}))",
                        Self::paren_if_needed(&word[3])
                    ),
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
        // The hex helper expects a byte in 0..=255. Byte-valued vars and
        // already-mod-256'd expressions skip the defensive `+ 256` wrap.
        let trimmed = expr.trim();
        let b = if Self::is_byte_valued_var(trimmed)
            || trimmed.starts_with("mod(")
            || trimmed.parse::<i64>().is_ok_and(|n| (0..=255).contains(&n))
        {
            trimmed.to_string()
        } else {
            format!("mod(calc(({}) + 256), 256)", expr)
        };
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
                let nz3 = Self::ne_expr(&lhs[3], "0");
                let nz2 = Self::ne_expr(&lhs[2], "0");
                let nz1 = Self::ne_expr(&lhs[1], "0");
                let z3 = Self::flip_bool(&nz3);
                let z2 = Self::flip_bool(&nz2);
                let z1 = Self::flip_bool(&nz1);
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
                let nz0 = Self::ne_expr(&lhs[0], "0");
                let nz1 = Self::ne_expr(&lhs[1], "0");
                let nz2 = Self::ne_expr(&lhs[2], "0");
                let z0 = Self::flip_bool(&nz0);
                let z1 = Self::flip_bool(&nz1);
                let z2 = Self::flip_bool(&nz2);
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

    pub(super) fn bitwise_expr(op: BitwiseOp, l: &str, r: &str) -> String {
        let mut terms = Vec::with_capacity(8);
        for k in 0..8u32 {
            let p2 = 1u32 << k;
            let lb = Self::byte_bit_expr(l, k as u8);
            let rb = Self::byte_bit_expr(r, k as u8);
            let bit = match op {
                BitwiseOp::And => Self::fold_mul(&lb, &rb),
                BitwiseOp::Or => Self::fold_or_bit(&lb, &rb),
                BitwiseOp::Xor => Self::fold_xor_bit(&lb, &rb),
            };
            terms.push(Self::fold_mul_by_int(&bit, p2));
        }
        Self::fold_add(terms)
    }

    fn fold_mul(a: &str, b: &str) -> String {
        let at = a.trim();
        let bt = b.trim();
        if at == "0" || bt == "0" {
            return "0".to_string();
        }
        if at == "1" {
            return b.to_string();
        }
        if bt == "1" {
            return a.to_string();
        }
        if let (Ok(an), Ok(bn)) = (at.parse::<i64>(), bt.parse::<i64>()) {
            return (an * bn).to_string();
        }
        format!(
            "calc({} * {})",
            Self::paren_if_needed(a),
            Self::paren_if_needed(b)
        )
    }

    fn fold_or_bit(a: &str, b: &str) -> String {
        match (a.trim(), b.trim()) {
            ("0", _) => b.to_string(),
            (_, "0") => a.to_string(),
            ("1", _) | (_, "1") => "1".to_string(),
            _ => format!(
                "min(1, calc({} + {}))",
                Self::paren_if_needed(a),
                Self::paren_if_needed(b)
            ),
        }
    }

    /// Computes `a ^ b` on bit-valued expressions using `mod(a + b, 2)`.
    fn fold_xor_bit(a: &str, b: &str) -> String {
        match (a.trim(), b.trim()) {
            ("0", _) => b.to_string(),
            (_, "0") => a.to_string(),
            ("1", "1") => "0".to_string(),
            _ => format!(
                "mod(calc({} + {}), 2)",
                Self::paren_if_needed(a),
                Self::paren_if_needed(b)
            ),
        }
    }

    /// Multiplies an expression by an integer constant, folding the trivial
    /// `0 * x` and `1 * x` cases.
    fn fold_mul_by_int(expr: &str, c: u32) -> String {
        if c == 0 || expr.trim() == "0" {
            return "0".to_string();
        }
        if c == 1 {
            return expr.to_string();
        }
        if expr.trim() == "1" {
            return c.to_string();
        }
        format!("calc({} * {c})", Self::paren_if_needed(expr))
    }

    /// Sums a list of expressions, dropping zero terms. If every term is
    /// zero, returns `"0"`. A single non-zero term is returned without the
    /// surrounding `calc(...)`.
    fn fold_add(terms: Vec<String>) -> String {
        let nz: Vec<String> = terms.into_iter().filter(|t| t.trim() != "0").collect();
        match nz.len() {
            0 => "0".to_string(),
            1 => nz.into_iter().next().unwrap(),
            _ => format!("calc({})", nz.join(" + ")),
        }
    }

    /// Returns Some(K) when every dispatch arm in `arms` has the same integer
    /// literal value K. Returns Some(fallback) if `arms` is empty (the slot
    /// is never written) and the fallback parses as an integer. Otherwise
    /// returns None. Only considers literal values — arms with calc/var
    /// expressions disqualify the slot.
    pub(super) fn arms_active_constant(arms: &str, fallback: &str) -> Option<String> {
        let mut common: Option<String> = None;
        let mut saw_arm = false;
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
                        saw_arm = true;
                        let value_start = arm.rfind("): ").map(|idx| idx + 3);
                        let val = match value_start {
                            Some(p) => arm[p..].trim(),
                            None => return None,
                        };
                        if val.parse::<i64>().is_err() {
                            return None;
                        }
                        match &common {
                            None => common = Some(val.to_string()),
                            Some(c) if c == val => {}
                            _ => return None,
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
        if !saw_arm {
            let fb = fallback.trim();
            return fb.parse::<i64>().ok().map(|_| fb.to_string());
        }
        common
    }

    /// Folds `mod(value, modulus)` when both arguments are integer literals,
    /// and short-circuits `mod(0, _) -> 0`.
    pub(super) fn fold_mod(value: &str, modulus: i64) -> String {
        let v = value.trim();
        if v == "0" {
            return "0".to_string();
        }
        if let Ok(n) = v.parse::<i64>()
            && modulus > 0
        {
            return n.rem_euclid(modulus).to_string();
        }
        if modulus == 256 && Self::is_byte_valued_var(v) {
            return v.to_string();
        }
        format!("mod({}, {})", value, modulus)
    }

    /// True if `v` is a `var(--_NrM)` shadow reference to a byte-typed
    /// register — IR8 vregs always hold 0..=255, so a `mod(…, 256)` wrapper
    /// around such a reference is redundant.
    fn is_byte_valued_var(v: &str) -> bool {
        let Some(inner) = v.strip_prefix("var(").and_then(|t| t.strip_suffix(')')) else {
            return false;
        };
        let inner = inner.trim();
        let after_dash = match inner.strip_prefix("--_") {
            Some(t) => t,
            None => return false,
        };
        // Stage digit, then `r`, then digits.
        let mut chars = after_dash.bytes();
        let Some(stage) = chars.next() else {
            return false;
        };
        if !stage.is_ascii_digit() {
            return false;
        }
        let Some(b'r') = chars.next() else {
            return false;
        };
        let rest = &after_dash[2..];
        !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_digit())
    }
}
