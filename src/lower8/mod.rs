use std::collections::HashMap;

use anyhow::Context;

use crate::ast::{BinOp, RelOp, UnOp};
use crate::constants::MAX_ADDRESSABLE_MEMORY_BYTES;
use crate::ir::{BasicBlock, BlockId, Inst, IrNode, Terminator};
use crate::ir8::{
    BasicBlock8, BoolNary8, BuiltinId, CallTarget, Inst8, Inst8Kind, Ir8Program, PC_STRIDE, Pc,
    Terminator8, TrapCode, VREG_START, Val8, ValueWords, Word,
};
use crate::module::{ConstInit, IrFuncBody, IrModule};

mod analysis;
mod builder;
mod calls;
mod ops;

use analysis::{
    collect_spill_words, compute_live_after_by_block, compute_local_live_after_by_block,
};
use builder::{FuncAlloc, FuncBuilder, alloc_builtin_div_params, prealloc_locals};
use ops::lower_load_fill_byte;

#[derive(Debug, Clone, Copy, Default)]
pub struct Lower8Config {
    pub js_coprocessor: bool,
}

fn build_memory_layout(
    module: &IrModule,
    runtime_memory_limit_bytes: u32,
    stack_pointer: Option<u32>,
) -> anyhow::Result<(u32, Vec<u8>)> {
    let initial_memory_bytes = (module.num_pages() as usize) * 65536;
    anyhow::ensure!(
        initial_memory_bytes <= MAX_ADDRESSABLE_MEMORY_BYTES as usize,
        "linear memory too large for 16-bit addressing ({} bytes)",
        initial_memory_bytes
    );
    let mut init_bytes = vec![0u8; initial_memory_bytes];
    for (off, bytes) in module.preloaded_data() {
        let end = off + bytes.len();
        anyhow::ensure!(end <= initial_memory_bytes, "data segment out of bounds");
        init_bytes[*off..end].copy_from_slice(bytes);
    }
    let runtime_memory_end = if runtime_memory_limit_bytes == 0 {
        stack_pointer
            .context("runtime memory limit is 0, but global 0 (stack pointer) is missing")?
    } else {
        runtime_memory_limit_bytes
    };
    anyhow::ensure!(
        runtime_memory_limit_bytes <= MAX_ADDRESSABLE_MEMORY_BYTES,
        "runtime memory limit {} exceeds 16-bit address space limit {}",
        runtime_memory_limit_bytes,
        MAX_ADDRESSABLE_MEMORY_BYTES
    );
    if let Some(sp) = stack_pointer
        && sp > runtime_memory_end
    {
        eprintln!(
            "warning: global 0 stack pointer ({}) exceeds runtime memory limit ({} bytes)",
            sp, runtime_memory_end
        );
    }
    Ok((runtime_memory_end, init_bytes))
}

fn build_global_init(module: &IrModule) -> anyhow::Result<(Vec<Vec<u32>>, Vec<u32>)> {
    let mut map = Vec::with_capacity(module.globals().len());
    let mut out = Vec::new();
    for (i, g) in module.globals().iter().enumerate() {
        let base = out.len() as u32;
        match (g.content_type(), g.init()) {
            (wasmparser::ValType::I32, ConstInit::I32(v)) => {
                map.push(vec![base]);
                out.push(v as u32);
            }
            (wasmparser::ValType::I64, ConstInit::I64(v)) => {
                map.push(vec![base, base + 1]);
                out.push(v as u64 as u32);
                out.push(((v as u64) >> 32) as u32);
            }
            (ty, init) => {
                anyhow::bail!(
                    "global {} type/init mismatch in lower8: {:?} initialized with {:?}",
                    i,
                    ty,
                    init
                );
            }
        }
    }
    Ok((map, out))
}

struct Lower8Context<'a> {
    module: &'a IrModule,
    allocs: &'a [FuncAlloc],
    global_words: &'a [Vec<u32>],
    div_builtins: Option<DivBuiltinFuncs>,
}

#[derive(Clone, Copy)]
struct DivBuiltinFuncs {
    div_u: u32,
    rem_u: u32,
    div_s: u32,
    rem_s: u32,
}

impl DivBuiltinFuncs {
    fn for_op(self, op: BinOp) -> Option<u32> {
        match op {
            BinOp::DivU => Some(self.div_u),
            BinOp::RemU => Some(self.rem_u),
            BinOp::DivS => Some(self.div_s),
            BinOp::RemS => Some(self.rem_s),
            _ => None,
        }
    }
}

fn lower_word_lane32_op(
    b: &mut FuncBuilder,
    lhs: Word,
    rhs: Word,
    make_kind: impl Fn(Word, Word, u8) -> Inst8Kind,
) -> Word {
    let dst = b.alloc_word();
    for (lane, dst_lane) in dst.bytes().into_iter().enumerate() {
        b.emit(Inst8::with_dst(dst_lane, make_kind(lhs, rhs, lane as u8)));
    }
    dst
}

fn lower_word_bytewise_op(
    b: &mut FuncBuilder,
    lhs: Word,
    rhs: Word,
    make_kind: impl Fn(Val8, Val8) -> Inst8Kind,
) -> Word {
    let dst = b.alloc_word();
    for (dst_lane, (lhs_lane, rhs_lane)) in dst
        .bytes()
        .into_iter()
        .zip(lhs.bytes().into_iter().zip(rhs.bytes()))
    {
        b.emit(Inst8::with_dst(dst_lane, make_kind(lhs_lane, rhs_lane)));
    }
    dst
}

fn lower_value_bytewise_op(
    b: &mut FuncBuilder,
    lhs: ValueWords,
    rhs: ValueWords,
    make_kind: impl Fn(Val8, Val8) -> Inst8Kind + Copy,
) -> ValueWords {
    let lo = lower_word_bytewise_op(b, lhs.lo, rhs.lo, make_kind);
    let hi = lower_word_bytewise_op(
        b,
        lhs.hi.expect("i64 lhs must have hi"),
        rhs.hi.expect("i64 rhs must have hi"),
        make_kind,
    );
    ValueWords::two(lo, hi)
}

fn lower_add32(b: &mut FuncBuilder, lhs: Word, rhs: Word) -> Word {
    lower_word_lane32_op(b, lhs, rhs, |lhs, rhs, lane| Inst8Kind::Add32Byte {
        lhs,
        rhs,
        lane,
    })
}

fn lower_sub32(b: &mut FuncBuilder, lhs: Word, rhs: Word) -> Word {
    lower_word_lane32_op(b, lhs, rhs, |lhs, rhs, lane| Inst8Kind::Sub32Byte {
        lhs,
        rhs,
        lane,
    })
}

fn lower_sub32_with_borrow(b: &mut FuncBuilder, lhs: Word, rhs: Word) -> (Word, Val8) {
    let dst = lower_sub32(b, lhs, rhs);
    let borrow = b.alloc_reg();
    b.emit(Inst8::with_dst(borrow, Inst8Kind::Sub32Borrow { lhs, rhs }));
    (dst, borrow)
}

fn select_word(b: &mut FuncBuilder, cond: Val8, if_true: Word, if_false: Word) -> Word {
    let dst = b.alloc_word();
    for (dst_lane, (true_lane, false_lane)) in dst
        .bytes()
        .into_iter()
        .zip(if_true.bytes().into_iter().zip(if_false.bytes()))
    {
        b.emit(Inst8::with_dst(
            dst_lane,
            Inst8Kind::Sel(cond, true_lane, false_lane),
        ));
    }
    dst
}

fn select_value(
    b: &mut FuncBuilder,
    cond: Val8,
    if_true: ValueWords,
    if_false: ValueWords,
) -> ValueWords {
    let lo = select_word(b, cond, if_true.lo, if_false.lo);
    match (if_true.hi, if_false.hi) {
        (Some(true_hi), Some(false_hi)) => {
            ValueWords::two(lo, select_word(b, cond, true_hi, false_hi))
        }
        (None, None) => ValueWords::one(lo),
        _ => unreachable!("mismatched value widths in select_value"),
    }
}

fn emit_bool_chain(
    b: &mut FuncBuilder,
    vals: &[Val8],
    make_kind: impl Fn(BoolNary8) -> Inst8Kind,
) -> Val8 {
    let dst = b.alloc_reg();
    let op = BoolNary8::from_vals(vals).expect("bool op inputs should fit IR8 nary limit");
    b.emit(Inst8::with_dst(dst, make_kind(op)));
    dst
}

fn compare_word_lanes(
    b: &mut FuncBuilder,
    lhs: Word,
    rhs: Word,
    op: fn(Val8, Val8) -> Inst8Kind,
) -> [Val8; 4] {
    let mut out = [Val8::imm(0); 4];
    for (i, (&l, &r)) in lhs.bytes().iter().zip(rhs.bytes().iter()).enumerate() {
        let dst = b.alloc_reg();
        b.emit(Inst8::with_dst(dst, op(l, r)));
        out[i] = dst;
    }
    out
}

fn lower_eq32_bit(b: &mut FuncBuilder, lhs: Word, rhs: Word) -> Val8 {
    let eqs = compare_word_lanes(b, lhs, rhs, Inst8Kind::Eq);
    emit_bool_chain(b, &eqs, Inst8Kind::BoolAnd)
}

fn lower_ltu32_bit(b: &mut FuncBuilder, lhs: Word, rhs: Word) -> Val8 {
    let lt3 = b.alloc_reg();
    b.emit(Inst8::with_dst(lt3, Inst8Kind::LtU(lhs.b3, rhs.b3)));
    let eq3 = b.alloc_reg();
    b.emit(Inst8::with_dst(eq3, Inst8Kind::Eq(lhs.b3, rhs.b3)));
    let lt2 = b.alloc_reg();
    b.emit(Inst8::with_dst(lt2, Inst8Kind::LtU(lhs.b2, rhs.b2)));
    let eq2 = b.alloc_reg();
    b.emit(Inst8::with_dst(eq2, Inst8Kind::Eq(lhs.b2, rhs.b2)));
    let lt1 = b.alloc_reg();
    b.emit(Inst8::with_dst(lt1, Inst8Kind::LtU(lhs.b1, rhs.b1)));
    let eq1 = b.alloc_reg();
    b.emit(Inst8::with_dst(eq1, Inst8Kind::Eq(lhs.b1, rhs.b1)));
    let lt0 = b.alloc_reg();
    b.emit(Inst8::with_dst(lt0, Inst8Kind::LtU(lhs.b0, rhs.b0)));

    // lt3 || (eq3 && lt2) || (eq3 && eq2 && lt1) || (eq3 && eq2 && eq1 && lt0)
    let eq3_and_lt2 = emit_bool_chain(b, &[eq3, lt2], Inst8Kind::BoolAnd);
    let eq3_and_eq2 = emit_bool_chain(b, &[eq3, eq2], Inst8Kind::BoolAnd);
    let eq3_eq2_and_lt1 = emit_bool_chain(b, &[eq3_and_eq2, lt1], Inst8Kind::BoolAnd);
    let eq3_eq2_and_eq1 = emit_bool_chain(b, &[eq3_and_eq2, eq1], Inst8Kind::BoolAnd);
    let eq3_eq2_eq1_and_lt0 = emit_bool_chain(b, &[eq3_eq2_and_eq1, lt0], Inst8Kind::BoolAnd);
    emit_bool_chain(
        b,
        &[lt3, eq3_and_lt2, eq3_eq2_and_lt1, eq3_eq2_eq1_and_lt0],
        Inst8Kind::BoolOr,
    )
}

fn shl1_word(b: &mut FuncBuilder, w: Word) -> Word {
    lower_add32(b, w, w)
}

fn add_lsb_bit(b: &mut FuncBuilder, w: Word, bit: Val8, zero: Val8) -> Word {
    lower_add32(b, w, Word::new(bit, zero, zero, zero))
}

fn word_bit(b: &mut FuncBuilder, w: Word, bit_index: u8, zero: Val8) -> Val8 {
    let byte = w.byte(bit_index / 8);
    let mask = Val8::imm(1u8 << (bit_index % 8));
    let masked = b.alloc_reg();
    b.emit(Inst8::with_dst(masked, Inst8Kind::And8(byte, mask)));
    let bit = b.alloc_reg();
    b.emit(Inst8::with_dst(bit, Inst8Kind::Ne(masked, zero)));
    bit
}

fn negate_word(b: &mut FuncBuilder, w: Word) -> Word {
    let zero = Word::from_u32_imm(0);
    lower_sub32(b, zero, w)
}

fn lower_eq64_bit(b: &mut FuncBuilder, lhs: ValueWords, rhs: ValueWords) -> Val8 {
    let lo_eq = lower_eq32_bit(b, lhs.lo, rhs.lo);
    let hi_eq = lower_eq32_bit(
        b,
        lhs.hi.expect("i64 lhs must have hi"),
        rhs.hi.expect("i64 rhs must have hi"),
    );
    emit_bool_chain(b, &[lo_eq, hi_eq], Inst8Kind::BoolAnd)
}

fn lower_ltu64_bit(b: &mut FuncBuilder, lhs: ValueWords, rhs: ValueWords) -> Val8 {
    let lhs_hi = lhs.hi.expect("i64 lhs must have hi");
    let rhs_hi = rhs.hi.expect("i64 rhs must have hi");
    let hi_lt = lower_ltu32_bit(b, lhs_hi, rhs_hi);
    let hi_eq = lower_eq32_bit(b, lhs_hi, rhs_hi);
    let lo_lt = lower_ltu32_bit(b, lhs.lo, rhs.lo);
    let hi_eq_and_lo_lt = emit_bool_chain(b, &[hi_eq, lo_lt], Inst8Kind::BoolAnd);
    emit_bool_chain(b, &[hi_lt, hi_eq_and_lo_lt], Inst8Kind::BoolOr)
}

fn lower_add64(b: &mut FuncBuilder, lhs: ValueWords, rhs: ValueWords) -> ValueWords {
    let lo = lower_add32(b, lhs.lo, rhs.lo);
    let carry = lower_ltu32_bit(b, lo, lhs.lo);
    let hi_sum = lower_add32(
        b,
        lhs.hi.expect("i64 lhs must have hi"),
        rhs.hi.expect("i64 rhs must have hi"),
    );
    let hi = lower_add32(
        b,
        hi_sum,
        Word::new(carry, Val8::imm(0), Val8::imm(0), Val8::imm(0)),
    );
    ValueWords::two(lo, hi)
}

fn lower_sub64_with_borrow(
    b: &mut FuncBuilder,
    lhs: ValueWords,
    rhs: ValueWords,
) -> (ValueWords, Val8) {
    let (lo, borrow0) = lower_sub32_with_borrow(b, lhs.lo, rhs.lo);
    let (hi_tmp, borrow1a) = lower_sub32_with_borrow(
        b,
        lhs.hi.expect("i64 lhs must have hi"),
        rhs.hi.expect("i64 rhs must have hi"),
    );
    let borrow_word = Word::new(borrow0, Val8::imm(0), Val8::imm(0), Val8::imm(0));
    let (hi, borrow1b) = lower_sub32_with_borrow(b, hi_tmp, borrow_word);
    let borrow = emit_bool_chain(b, &[borrow1a, borrow1b], Inst8Kind::BoolOr);
    (ValueWords::two(lo, hi), borrow)
}

fn lower_sub64(b: &mut FuncBuilder, lhs: ValueWords, rhs: ValueWords) -> ValueWords {
    lower_sub64_with_borrow(b, lhs, rhs).0
}

fn lower_eqz64(b: &mut FuncBuilder, value: ValueWords) -> Word {
    let zero = ValueWords::two(Word::from_u32_imm(0), Word::from_u32_imm(0));
    let bit = lower_eq64_bit(b, value, zero);
    bool_to_word(b, bit)
}

fn lower_extend_i32_to_i64(
    b: &mut FuncBuilder,
    value: Word,
    signed: bool,
) -> ValueWords {
    if signed {
        let fill = lower_load_fill_byte(b, value.bytes()[3], true);
        let hi = Word::new(fill, fill, fill, fill);
        ValueWords::two(value, hi)
    } else {
        ValueWords::two(value, Word::from_u32_imm(0))
    }
}

fn lower_sign_extend64_from_byte(
    b: &mut FuncBuilder,
    value: ValueWords,
    check_word_hi: bool,
    check_byte: usize,
) -> ValueWords {
    let sign_src = if check_word_hi {
        value.hi.expect("i64 value must have hi").bytes()[check_byte]
    } else {
        value.lo.bytes()[check_byte]
    };
    let fill = lower_load_fill_byte(b, sign_src, true);
    let mut lo = value.lo;
    let mut hi = value.hi.expect("i64 value must have hi");
    if check_word_hi {
        for lane in (check_byte + 1)..4 {
            hi = Word::new(
                if lane == 0 { fill } else { hi.b0 },
                if lane <= 1 { fill } else { hi.b1 },
                if lane <= 2 { fill } else { hi.b2 },
                if lane <= 3 { fill } else { hi.b3 },
            );
        }
    } else {
        let lo_bytes = lo.bytes();
        lo = Word::new(
            lo_bytes[0],
            if check_byte < 1 { fill } else { lo_bytes[1] },
            if check_byte < 2 { fill } else { lo_bytes[2] },
            if check_byte < 3 { fill } else { lo_bytes[3] },
        );
        hi = Word::new(fill, fill, fill, fill);
    }
    ValueWords::two(lo, hi)
}

fn emit_divrem_u32_iteration(
    b: &mut FuncBuilder,
    numer: Word,
    denom: Word,
    bit_idx: u8,
    quotient_state: Word,
    remainder_state: Word,
    zero: Val8,
) {
    let remainder_shifted = shl1_word(b, remainder_state);
    let in_bit = word_bit(b, numer, bit_idx, zero);
    let remainder_with_bit = add_lsb_bit(b, remainder_shifted, in_bit, zero);

    let (sub, borrow_out) = lower_sub32_with_borrow(b, remainder_with_bit, denom);
    let ge = b.alloc_reg();
    b.emit(Inst8::with_dst(ge, Inst8Kind::BoolNot(borrow_out)));

    let remainder_next = select_word(b, ge, sub, remainder_with_bit);

    let quotient_shifted = shl1_word(b, quotient_state);
    let quotient_next = add_lsb_bit(b, quotient_shifted, ge, zero);

    b.copy_word(remainder_state, remainder_next);
    b.copy_word(quotient_state, quotient_next);
}

fn divrem_u32_core(b: &mut FuncBuilder, numer: Word, denom: Word) -> (Word, Word) {
    let zero_word = Word::from_u32_imm(0);
    let zero = Val8::imm(0);
    let quotient_state = b.alloc_word();
    let remainder_state = b.alloc_word();
    b.copy_word(quotient_state, zero_word);
    b.copy_word(remainder_state, zero_word);

    // TODO(i64): restoring division is currently unrolled for a fixed 32-bit width.
    let iter_blocks: Vec<Pc> = (0..32).map(|_| b.alloc_block()).collect();
    let done_pc = b.alloc_block();
    let check_b2_pc = b.alloc_block();
    let check_b1_pc = b.alloc_block();

    let b3_zero = b.alloc_reg();
    b.emit(Inst8::with_dst(b3_zero, Inst8Kind::Eq(numer.b3, zero)));
    b.finish(Terminator8::Branch {
        cond: b3_zero,
        if_true: check_b2_pc,
        if_false: iter_blocks[31],
    });

    b.switch_to(check_b2_pc);
    let b2_zero = b.alloc_reg();
    b.emit(Inst8::with_dst(b2_zero, Inst8Kind::Eq(numer.b2, zero)));
    b.finish(Terminator8::Branch {
        cond: b2_zero,
        if_true: check_b1_pc,
        if_false: iter_blocks[23],
    });

    b.switch_to(check_b1_pc);
    let b1_zero = b.alloc_reg();
    b.emit(Inst8::with_dst(b1_zero, Inst8Kind::Eq(numer.b1, zero)));
    b.finish(Terminator8::Branch {
        cond: b1_zero,
        if_true: iter_blocks[7],
        if_false: iter_blocks[15],
    });

    for bit_idx in (0..32u8).rev() {
        b.switch_to(iter_blocks[bit_idx as usize]);
        emit_divrem_u32_iteration(
            b,
            numer,
            denom,
            bit_idx,
            quotient_state,
            remainder_state,
            zero,
        );
        let next_pc = if bit_idx == 0 {
            done_pc
        } else {
            iter_blocks[bit_idx as usize - 1]
        };
        b.finish(Terminator8::Goto(next_pc));
    }

    b.switch_to(done_pc);
    (quotient_state, remainder_state)
}

fn emit_zero_check_trap(b: &mut FuncBuilder, denom: Word) {
    let denom_zero = lower_eq32_bit(b, denom, Word::from_u32_imm(0));
    let trap_pc = b.alloc_block();
    let cont_pc = b.alloc_block();
    b.finish(Terminator8::Branch {
        cond: denom_zero,
        if_true: trap_pc,
        if_false: cont_pc,
    });
    b.switch_to(trap_pc);
    b.finish(Terminator8::Trap(TrapCode::DivisionByZero));
    b.switch_to(cont_pc);
}

fn lower_divrem_u32(b: &mut FuncBuilder, numer: Word, denom: Word, want_rem: bool) -> Word {
    emit_zero_check_trap(b, denom);
    let (q, r) = divrem_u32_core(b, numer, denom);
    if want_rem { r } else { q }
}

fn lower_divrem_s32(b: &mut FuncBuilder, numer: Word, denom: Word, want_rem: bool) -> Word {
    emit_zero_check_trap(b, denom);
    let zero = Val8::imm(0);
    let numer_sign = word_bit(b, numer, 31, zero);
    let denom_sign = word_bit(b, denom, 31, zero);
    let numer_neg = negate_word(b, numer);
    let denom_neg = negate_word(b, denom);
    let numer_abs = select_word(b, numer_sign, numer_neg, numer);
    let denom_abs = select_word(b, denom_sign, denom_neg, denom);

    let (mut q, mut r) = divrem_u32_core(b, numer_abs, denom_abs);
    if want_rem {
        let r_neg = negate_word(b, r);
        r = select_word(b, numer_sign, r_neg, r);
        r
    } else {
        let sign = b.alloc_reg();
        b.emit(Inst8::with_dst(
            sign,
            Inst8Kind::Xor8(numer_sign, denom_sign),
        ));
        let q_neg = negate_word(b, q);
        q = select_word(b, sign, q_neg, q);
        q
    }
}

fn word_const_u32(w: Word) -> Option<u32> {
    let b0 = w.b0.imm_value()? as u32;
    let b1 = w.b1.imm_value()? as u32;
    let b2 = w.b2.imm_value()? as u32;
    let b3 = w.b3.imm_value()? as u32;
    Some(b0 | (b1 << 8) | (b2 << 16) | (b3 << 24))
}

fn shift_left_byte_const(b: &mut FuncBuilder, src: Val8, shift: u8) -> Val8 {
    debug_assert!(shift <= 7);
    if shift == 0 {
        return src;
    }
    if let Some(imm) = src.imm_value() {
        return Val8::imm(imm.wrapping_shl(u32::from(shift)));
    }

    let dst = b.alloc_reg();
    b.emit(Inst8::with_dst(
        dst,
        Inst8Kind::MulLo(src, Val8::imm(1u8 << shift)),
    ));
    dst
}

fn shift_right_u_byte_const(b: &mut FuncBuilder, src: Val8, shift: u8) -> Val8 {
    debug_assert!(shift <= 7);
    if shift == 0 {
        return src;
    }
    if let Some(imm) = src.imm_value() {
        return Val8::imm(imm >> shift);
    }

    let dst = b.alloc_reg();
    b.emit(Inst8::with_dst(
        dst,
        Inst8Kind::MulHi(src, Val8::imm(1u8 << (8 - shift))),
    ));
    dst
}

fn or_bytes_const_fold(b: &mut FuncBuilder, lhs: Val8, rhs: Val8) -> Val8 {
    if let (Some(l), Some(r)) = (lhs.imm_value(), rhs.imm_value()) {
        return Val8::imm(l | r);
    }
    if let Val8::Imm(0) = lhs {
        return rhs;
    }
    if let Val8::Imm(0) = rhs {
        return lhs;
    }

    let dst = b.alloc_reg();
    b.emit(Inst8::with_dst(dst, Inst8Kind::Or8(lhs, rhs)));
    dst
}

fn word_byte_or_fill(src: [Val8; 4], idx: usize, fill: Val8) -> Val8 {
    src.get(idx).copied().unwrap_or(fill)
}

fn lower_shl_const_u32(b: &mut FuncBuilder, lhs: Word, rhs_const: u32) -> Word {
    let shift = (rhs_const & 31) as u8;
    if shift == 0 {
        return lhs;
    }

    let src = lhs.bytes();
    let dst = b.alloc_word();
    let byte_shift = usize::from(shift / 8);
    let bit_shift = shift % 8;
    for (lane, dst_lane) in dst.bytes().into_iter().enumerate() {
        let low_src = lane
            .checked_sub(byte_shift)
            .map_or(Val8::imm(0), |idx| src[idx]);
        let out = if bit_shift == 0 {
            low_src
        } else {
            let shifted = shift_left_byte_const(b, low_src, bit_shift);
            let carry_src = lane
                .checked_sub(byte_shift + 1)
                .map_or(Val8::imm(0), |idx| src[idx]);
            let carry = shift_right_u_byte_const(b, carry_src, 8 - bit_shift);
            or_bytes_const_fold(b, shifted, carry)
        };
        b.emit(Inst8::with_dst(dst_lane, Inst8Kind::Copy(out)));
    }
    dst
}

fn lower_shr_const_u32_with_fill(
    b: &mut FuncBuilder,
    lhs: Word,
    shift: u8,
    high_fill: Val8,
) -> Word {
    debug_assert!(shift <= 31);
    if shift == 0 {
        return lhs;
    }

    let src = lhs.bytes();
    let dst = b.alloc_word();
    let byte_shift = usize::from(shift / 8);
    let bit_shift = shift % 8;
    for (lane, dst_lane) in dst.bytes().into_iter().enumerate() {
        let low_src = word_byte_or_fill(src, lane + byte_shift, high_fill);
        let out = if bit_shift == 0 {
            low_src
        } else {
            let shifted = shift_right_u_byte_const(b, low_src, bit_shift);
            let carry_src = word_byte_or_fill(src, lane + byte_shift + 1, high_fill);
            let carry = shift_left_byte_const(b, carry_src, 8 - bit_shift);
            or_bytes_const_fold(b, shifted, carry)
        };
        b.emit(Inst8::with_dst(dst_lane, Inst8Kind::Copy(out)));
    }
    dst
}

fn lower_shr_u_const_u32(b: &mut FuncBuilder, lhs: Word, rhs_const: u32) -> Word {
    let shift = (rhs_const & 31) as u8;
    lower_shr_const_u32_with_fill(b, lhs, shift, Val8::imm(0))
}

fn lower_shr_s_const_u32(b: &mut FuncBuilder, lhs: Word, rhs_const: u32) -> Word {
    let shift = (rhs_const & 31) as u8;
    if shift == 0 {
        return lhs;
    }

    let is_neg = b.alloc_reg();
    b.emit(Inst8::with_dst(
        is_neg,
        Inst8Kind::GeU(lhs.b3, Val8::imm(0x80)),
    ));
    let high_fill = b.alloc_reg();
    b.emit(Inst8::with_dst(
        high_fill,
        Inst8Kind::Sub(Val8::imm(0), is_neg),
    ));
    lower_shr_const_u32_with_fill(b, lhs, shift, high_fill)
}

fn lower_rotl_const_u32(b: &mut FuncBuilder, lhs: Word, rhs_const: u32) -> Word {
    let shift = (rhs_const & 31) as u8;
    if shift == 0 {
        return lhs;
    }
    let left = lower_shl_const_u32(b, lhs, u32::from(shift));
    let right = lower_shr_u_const_u32(b, lhs, u32::from(32 - shift));
    lower_word_bytewise_op(b, left, right, Inst8Kind::Or8)
}

fn lower_rotr_const_u32(b: &mut FuncBuilder, lhs: Word, rhs_const: u32) -> Word {
    let shift = (rhs_const & 31) as u8;
    if shift == 0 {
        return lhs;
    }
    let right = lower_shr_u_const_u32(b, lhs, u32::from(shift));
    let left = lower_shl_const_u32(b, lhs, u32::from(32 - shift));
    lower_word_bytewise_op(b, left, right, Inst8Kind::Or8)
}

fn lower_const_shift_or_rotate(b: &mut FuncBuilder, op: BinOp, lhs: Word, rhs_const: u32) -> Word {
    match op {
        BinOp::Shl => lower_shl_const_u32(b, lhs, rhs_const),
        BinOp::ShrU => lower_shr_u_const_u32(b, lhs, rhs_const),
        BinOp::ShrS => lower_shr_s_const_u32(b, lhs, rhs_const),
        BinOp::Rotl => lower_rotl_const_u32(b, lhs, rhs_const),
        BinOp::Rotr => lower_rotr_const_u32(b, lhs, rhs_const),
        _ => unreachable!("expected shift/rotate op, got {op:?}"),
    }
}

fn lower_divrem_const_u32(
    b: &mut FuncBuilder,
    op: BinOp,
    numer: Word,
    denom_u32: u32,
) -> Option<Word> {
    let is_rem = matches!(op, BinOp::RemU);
    if !matches!(op, BinOp::DivU | BinOp::RemU) {
        return None;
    }

    if denom_u32 == 0 {
        // Let runtime-trapping path handle divide-by-zero semantics.
        return None;
    }

    if denom_u32 == 1 {
        return Some(if is_rem { Word::from_u32_imm(0) } else { numer });
    }

    if denom_u32.is_power_of_two() {
        if is_rem {
            let mask = Word::from_u32_imm(denom_u32 - 1);
            return Some(lower_word_bytewise_op(b, numer, mask, Inst8Kind::And8));
        }
        return Some(lower_shr_u_const_u32(b, numer, denom_u32.trailing_zeros()));
    }

    // For non-trivial constants we prefer the shared helper to avoid
    // large inlined unrolled division sequences.
    None
}

fn lower_divrem_const_s32(
    b: &mut FuncBuilder,
    op: BinOp,
    numer: Word,
    denom_u32: u32,
) -> Option<Word> {
    let is_rem = matches!(op, BinOp::RemS);
    if !matches!(op, BinOp::DivS | BinOp::RemS) {
        return None;
    }
    if denom_u32 == 0 {
        return None;
    }

    let denom_i32 = denom_u32 as i32;
    if denom_i32 == 1 {
        return Some(if is_rem { Word::from_u32_imm(0) } else { numer });
    }

    if denom_i32 == -1 {
        return Some(if is_rem {
            Word::from_u32_imm(0)
        } else {
            negate_word(b, numer)
        });
    }

    None
}

#[allow(clippy::too_many_arguments)]
fn lower_divrem_call_via_function(
    b: &mut FuncBuilder,
    op: BinOp,
    lhs: Word,
    rhs: Word,
    live_after: &[IrNode],
    live_locals_after: &[u32],
    allocs: &[FuncAlloc],
    div_builtins: Option<DivBuiltinFuncs>,
) -> anyhow::Result<Word> {
    let div_builtins = div_builtins.context("div/rem helper table is unavailable")?;
    let callee_id = div_builtins
        .for_op(op)
        .with_context(|| format!("expected div/rem op, got {:?}", op))?;
    let callee_alloc = &allocs[callee_id as usize];
    let callee_arg_vregs = divrem_param_vregs(&callee_alloc.local_vregs, callee_id)?.to_vec();
    let callee_entry = Pc::new(callee_id as u16 * PC_STRIDE);
    let cont = b.alloc_block();
    let spill_words = collect_spill_words(live_after, &b.inst_map, &b.local_vregs);
    b.emit_cs_save(cont, live_locals_after, &spill_words);

    b.finish(Terminator8::CallSetup {
        callee_entry: CallTarget::Pc(callee_entry),
        cont,
        args: vec![lhs, rhs],
        callee_arg_vregs,
    });
    b.switch_to(cont);
    b.emit_cs_restore(live_locals_after, &spill_words);

    let dst = b.alloc_word();
    b.copy_ret_to_word(dst);
    Ok(dst)
}

/// Sign-extend from `check_byte` (0 for i8, 1 for i16): copies bytes 0..=check_byte,
/// fills the rest with the sign bit.
fn lower_sign_extend(b: &mut FuncBuilder, operand: Word, check_byte: usize) -> Word {
    let dst = b.alloc_word();
    let is_neg = b.alloc_reg();
    let op_bytes = operand.bytes();
    b.emit(Inst8::with_dst(
        is_neg,
        Inst8Kind::GeU(op_bytes[check_byte], Val8::imm(0x80)),
    ));
    let fill = b.alloc_reg();
    b.emit(Inst8::with_dst(fill, Inst8Kind::Sub(Val8::imm(0), is_neg)));
    let dst_bytes = dst.bytes();
    for i in 0..4 {
        let src = if i <= check_byte { op_bytes[i] } else { fill };
        b.emit(Inst8::with_dst(dst_bytes[i], Inst8Kind::Copy(src)));
    }
    dst
}

fn lower_builtin_call(b: &mut FuncBuilder, builtin: BuiltinId, args: Vec<Word>) -> Word {
    let callee_arg_vregs: Vec<Word> = args.iter().map(|_| b.alloc_word()).collect();
    let cont = b.alloc_block();
    b.finish(Terminator8::CallSetup {
        callee_entry: CallTarget::Builtin(builtin),
        cont,
        args,
        callee_arg_vregs,
    });
    b.switch_to(cont);
    let dst = b.alloc_word();
    b.copy_ret_to_word(dst);
    dst
}

fn lower_binary(b: &mut FuncBuilder, op: BinOp, lhs: Word, rhs: Word) -> anyhow::Result<Word> {
    Ok(match op {
        BinOp::Add => lower_add32(b, lhs, rhs),
        BinOp::Sub => lower_sub32(b, lhs, rhs),
        BinOp::And => lower_word_bytewise_op(b, lhs, rhs, Inst8Kind::And8),
        BinOp::Or => lower_word_bytewise_op(b, lhs, rhs, Inst8Kind::Or8),
        BinOp::Xor => lower_word_bytewise_op(b, lhs, rhs, Inst8Kind::Xor8),
        BinOp::Mul => lower_mul32(b, lhs, rhs),
        BinOp::DivU => lower_divrem_u32(b, lhs, rhs, false),
        BinOp::RemU => lower_divrem_u32(b, lhs, rhs, true),
        BinOp::DivS => lower_divrem_s32(b, lhs, rhs, false),
        BinOp::RemS => lower_divrem_s32(b, lhs, rhs, true),
        BinOp::Shl | BinOp::ShrU | BinOp::ShrS | BinOp::Rotl | BinOp::Rotr => {
            if let Some(rhs_const) = word_const_u32(rhs) {
                lower_const_shift_or_rotate(b, op, lhs, rhs_const)
            } else {
                let builtin = match op {
                    BinOp::Shl => BuiltinId::Shl32,
                    BinOp::ShrU => BuiltinId::ShrU32,
                    BinOp::ShrS => BuiltinId::ShrS32,
                    BinOp::Rotl => BuiltinId::Rotl32,
                    BinOp::Rotr => BuiltinId::Rotr32,
                    _ => unreachable!("expected shift/rotate op, got {op:?}"),
                };
                lower_builtin_call(b, builtin, vec![lhs, rhs])
            }
        }
    })
}

fn add_bytes(b: &mut FuncBuilder, terms: &[Val8]) -> (Val8, Val8) {
    debug_assert!(!terms.is_empty());
    if terms.len() == 1 {
        return (terms[0], Val8::imm(0));
    }
    let mut acc = terms[0];
    let mut carry = Val8::imm(0);
    for &t in &terms[1..] {
        let new_acc = b.alloc_reg();
        b.emit(Inst8::with_dst(new_acc, Inst8Kind::Add(acc, t)));
        let this_carry = b.alloc_reg();
        b.emit(Inst8::with_dst(this_carry, Inst8Kind::Carry(acc, t)));
        let new_carry = b.alloc_reg();
        b.emit(Inst8::with_dst(
            new_carry,
            Inst8Kind::Add(carry, this_carry),
        ));
        acc = new_acc;
        carry = new_carry;
    }
    (acc, carry)
}

fn lower_mul32(b: &mut FuncBuilder, lhs: Word, rhs: Word) -> Word {
    macro_rules! mul_lo {
        ($x:expr, $y:expr) => {{
            let v = b.alloc_reg();
            b.emit(Inst8::with_dst(v, Inst8Kind::MulLo($x, $y)));
            v
        }};
    }
    macro_rules! mul_hi {
        ($x:expr, $y:expr) => {{
            let v = b.alloc_reg();
            b.emit(Inst8::with_dst(v, Inst8Kind::MulHi($x, $y)));
            v
        }};
    }

    let p00_lo = mul_lo!(lhs.b0, rhs.b0);
    let p00_hi = mul_hi!(lhs.b0, rhs.b0);
    let p01_lo = mul_lo!(lhs.b0, rhs.b1);
    let p01_hi = mul_hi!(lhs.b0, rhs.b1);
    let p10_lo = mul_lo!(lhs.b1, rhs.b0);
    let p10_hi = mul_hi!(lhs.b1, rhs.b0);
    let p02_lo = mul_lo!(lhs.b0, rhs.b2);
    let p02_hi = mul_hi!(lhs.b0, rhs.b2);
    let p11_lo = mul_lo!(lhs.b1, rhs.b1);
    let p11_hi = mul_hi!(lhs.b1, rhs.b1);
    let p20_lo = mul_lo!(lhs.b2, rhs.b0);
    let p20_hi = mul_hi!(lhs.b2, rhs.b0);
    let p03_lo = mul_lo!(lhs.b0, rhs.b3);
    let p12_lo = mul_lo!(lhs.b1, rhs.b2);
    let p21_lo = mul_lo!(lhs.b2, rhs.b1);
    let p30_lo = mul_lo!(lhs.b3, rhs.b0);

    let b0 = p00_lo;
    let (b1, c1) = add_bytes(b, &[p00_hi, p01_lo, p10_lo]);
    let (b2, c2) = add_bytes(b, &[p01_hi, p10_hi, p02_lo, p11_lo, p20_lo, c1]);
    let (b3, _) = add_bytes(
        b,
        &[p02_hi, p11_hi, p20_hi, p03_lo, p12_lo, p21_lo, p30_lo, c2],
    );

    Word::new(b0, b1, b2, b3)
}

fn bool32(b: &mut FuncBuilder, val: Word) -> Val8 {
    let zero = Word::from_u32_imm(0);
    let nes = compare_word_lanes(b, val, zero, Inst8Kind::Ne);
    emit_bool_chain(b, &nes, Inst8Kind::BoolOr)
}

fn bool_not(b: &mut FuncBuilder, bit: Val8) -> Val8 {
    let dst = b.alloc_reg();
    b.emit(Inst8::with_dst(dst, Inst8Kind::BoolNot(bit)));
    dst
}

fn bool_to_word(b: &mut FuncBuilder, bit: Val8) -> Word {
    let dst = b.alloc_word();
    b.set_word_from_byte(dst, bit);
    dst
}

fn lower_eq32(b: &mut FuncBuilder, lhs: Word, rhs: Word) -> Word {
    let bit = lower_eq32_bit(b, lhs, rhs);
    bool_to_word(b, bit)
}

fn lower_ne32(b: &mut FuncBuilder, lhs: Word, rhs: Word) -> Word {
    let nes = compare_word_lanes(b, lhs, rhs, Inst8Kind::Ne);
    let bit = emit_bool_chain(b, &nes, Inst8Kind::BoolOr);
    bool_to_word(b, bit)
}

fn lower_ltu32(b: &mut FuncBuilder, lhs: Word, rhs: Word) -> Word {
    let bit = lower_ltu32_bit(b, lhs, rhs);
    bool_to_word(b, bit)
}

fn lower_gtu32(b: &mut FuncBuilder, lhs: Word, rhs: Word) -> Word {
    lower_ltu32(b, rhs, lhs)
}

fn negate_cmp(b: &mut FuncBuilder, result: Word) -> Word {
    let bit = b.alloc_reg();
    b.emit(Inst8::with_dst(bit, Inst8Kind::BoolNot(result.b0)));
    bool_to_word(b, bit)
}

fn lower_leu32(b: &mut FuncBuilder, lhs: Word, rhs: Word) -> Word {
    let gt = lower_gtu32(b, lhs, rhs);
    negate_cmp(b, gt)
}

fn lower_geu32(b: &mut FuncBuilder, lhs: Word, rhs: Word) -> Word {
    let lt = lower_ltu32(b, lhs, rhs);
    negate_cmp(b, lt)
}

fn flip_sign(b: &mut FuncBuilder, value: Word) -> Word {
    let mask = Val8::imm(0x80);
    let flipped = b.alloc_reg();
    b.emit(Inst8::with_dst(flipped, Inst8Kind::Xor8(value.b3, mask)));
    Word::new(value.b0, value.b1, value.b2, flipped)
}

fn lower_compare(b: &mut FuncBuilder, op: RelOp, lhs: Word, rhs: Word) -> anyhow::Result<Word> {
    Ok(match op {
        RelOp::Eq => lower_eq32(b, lhs, rhs),
        RelOp::Ne => lower_ne32(b, lhs, rhs),
        RelOp::LtU => lower_ltu32(b, lhs, rhs),
        RelOp::GtU => lower_gtu32(b, lhs, rhs),
        RelOp::LeU => lower_leu32(b, lhs, rhs),
        RelOp::GeU => lower_geu32(b, lhs, rhs),
        // Signed: XOR sign bits to convert to unsigned order.
        RelOp::LtS => {
            let (lhs2, rhs2) = (flip_sign(b, lhs), flip_sign(b, rhs));
            lower_ltu32(b, lhs2, rhs2)
        }
        RelOp::GtS => {
            let (lhs2, rhs2) = (flip_sign(b, lhs), flip_sign(b, rhs));
            lower_gtu32(b, lhs2, rhs2)
        }
        RelOp::LeS => {
            let (lhs2, rhs2) = (flip_sign(b, lhs), flip_sign(b, rhs));
            lower_leu32(b, lhs2, rhs2)
        }
        RelOp::GeS => {
            let (lhs2, rhs2) = (flip_sign(b, lhs), flip_sign(b, rhs));
            lower_geu32(b, lhs2, rhs2)
        }
    })
}

fn lower_unary(b: &mut FuncBuilder, op: UnOp, operand: Word) -> anyhow::Result<Word> {
    Ok(match op {
        UnOp::Eqz => {
            // eqz(a) = (a == 0)
            let zero_const = Word::from_u32_imm(0);
            lower_eq32(b, operand, zero_const)
        }
        UnOp::Extend8S => lower_sign_extend(b, operand, 0),
        UnOp::Extend16S => lower_sign_extend(b, operand, 1),
        UnOp::Clz => lower_builtin_call(b, BuiltinId::Clz32, vec![operand]),
        UnOp::Ctz => lower_builtin_call(b, BuiltinId::Ctz32, vec![operand]),
        UnOp::Popcnt => lower_builtin_call(b, BuiltinId::Popcnt32, vec![operand]),
        UnOp::Extend32S | UnOp::WrapI64 | UnOp::ExtendI32S | UnOp::ExtendI32U => {
            unreachable!("i64 unary ops are handled before lower_unary dispatch");
        }
    })
}

fn lower_terminator(
    b: &mut FuncBuilder,
    module: &IrModule,
    term: &Terminator,
    allocs: &[FuncAlloc],
) -> anyhow::Result<()> {
    match term {
        Terminator::Goto(target) => {
            b.finish(Terminator8::Goto(b.pc_of(*target)));
        }
        Terminator::Branch {
            cond,
            if_true,
            if_false,
        } => {
            let cv = b.get_word(*cond);
            let cond_bit = bool32(b, cv);
            b.finish(Terminator8::Branch {
                cond: cond_bit,
                if_true: b.pc_of(*if_true),
                if_false: b.pc_of(*if_false),
            });
        }
        Terminator::Switch {
            index,
            targets,
            default,
        } => {
            let iv = b.get_word(*index);
            b.finish(Terminator8::Switch {
                index: iv.b0,
                targets: targets.iter().map(|t| b.pc_of(*t)).collect(),
                default: b.pc_of(*default),
            });
        }
        Terminator::TailCall { func, args } => {
            if b.is_entry {
                anyhow::bail!("entry function '_start' must not contain tail_call terminators");
            }
            let t8 = calls::lower_tail_call(b, *func, args, allocs)?;
            b.finish(t8);
        }
        Terminator::TailCallIndirect {
            type_index,
            table_index,
            index,
            args,
        } => {
            if b.is_entry {
                anyhow::bail!(
                    "entry function '_start' must not contain tail_call_indirect terminators"
                );
            }
            calls::lower_tail_call_indirect(
                b,
                module,
                *type_index,
                *table_index,
                *index,
                args,
                allocs,
            )?;
        }
        Terminator::Return(val_ref) => {
            let val = val_ref.map(|r| b.get_value(r));
            if b.is_entry {
                b.finish(Terminator8::Exit { val });
            } else {
                calls::emit_non_main_return_sequence(b, val);
            }
        }
        Terminator::Unreachable => {
            b.finish(Terminator8::Trap(TrapCode::Unreachable));
        }
        Terminator::UncaughtExit => {
            if b.is_entry {
                b.finish(Terminator8::Trap(TrapCode::UncaughtException));
            } else {
                calls::emit_non_main_return_sequence(b, None);
            }
        }
    };
    Ok(())
}

#[derive(Clone, Copy)]
enum DivBuiltinKind {
    DivU,
    RemU,
    DivS,
    RemS,
}

fn divrem_param_vregs(local_vregs: &[ValueWords], func_id: u32) -> anyhow::Result<[Word; 2]> {
    anyhow::ensure!(
        local_vregs.len() >= 2,
        "builtin div/rem function {} expects two parameter words",
        func_id
    );
    Ok([local_vregs[0].lo, local_vregs[1].lo])
}

fn lower_divrem_builtin_function(
    func_id: u32,
    local_vregs: Vec<ValueWords>,
    vreg_start: u16,
    kind: DivBuiltinKind,
) -> anyhow::Result<(Vec<BasicBlock8>, Pc, u32, u16)> {
    let [numer, denom] = divrem_param_vregs(&local_vregs, func_id)?;
    let mut b = FuncBuilder::new(func_id, false, vreg_start, local_vregs);
    let entry = b.alloc_block();
    b.switch_to(entry);

    let ret = match kind {
        // TODO(i64): synthetic div/rem helper functions exist only for 32-bit integer ops.
        DivBuiltinKind::DivU => lower_divrem_u32(&mut b, numer, denom, false),
        DivBuiltinKind::RemU => lower_divrem_u32(&mut b, numer, denom, true),
        DivBuiltinKind::DivS => lower_divrem_s32(&mut b, numer, denom, false),
        DivBuiltinKind::RemS => lower_divrem_s32(&mut b, numer, denom, true),
    };

    calls::emit_non_main_return_sequence(&mut b, Some(ValueWords::one(ret)));

    Ok((
        b.blocks,
        Pc::new(func_id as u16 * PC_STRIDE),
        2,
        b.vreg_counter,
    ))
}

pub fn lower8_module(module: &IrModule, memory_bytes_cap: u32) -> anyhow::Result<Ir8Program> {
    lower8_module_with_config(module, memory_bytes_cap, Lower8Config::default())
}

pub fn lower8_module_with_config(
    module: &IrModule,
    memory_bytes_cap: u32,
    config: Lower8Config,
) -> anyhow::Result<Ir8Program> {
    let builtin_slots = if config.js_coprocessor {
        0usize
    } else {
        4usize
    };
    let total_func_slots = module.bodies().len() + builtin_slots;
    let max_func_slots = usize::from(u16::MAX / PC_STRIDE) + 1;
    anyhow::ensure!(
        total_func_slots <= max_func_slots,
        "module needs {} function PC slot(s), but only {} fit in the u16 PC space with PC_STRIDE {}",
        total_func_slots,
        max_func_slots,
        PC_STRIDE
    );

    let entry_func_id = module.entry_export().context("no '_start' export")? as usize;
    let (global_words, global_init) = build_global_init(module)?;
    let stack_pointer = global_init.first().copied();
    let (memory_end, init_bytes) = build_memory_layout(module, memory_bytes_cap, stack_pointer)?;
    let (mut allocs, mut vreg_counter) = prealloc_locals(module);

    let div_builtins = if config.js_coprocessor {
        None
    } else {
        let user_func_count = module.bodies().len() as u32;
        let builtins = DivBuiltinFuncs {
            div_u: user_func_count,
            rem_u: user_func_count + 1,
            div_s: user_func_count + 2,
            rem_s: user_func_count + 3,
        };
        allocs.push(alloc_builtin_div_params(&mut vreg_counter));
        allocs.push(alloc_builtin_div_params(&mut vreg_counter));
        allocs.push(alloc_builtin_div_params(&mut vreg_counter));
        allocs.push(alloc_builtin_div_params(&mut vreg_counter));
        Some(builtins)
    };

    let mut func_blocks: Vec<Vec<BasicBlock8>> = Vec::with_capacity(module.bodies().len() + 4);
    let mut func_entries: Vec<Pc> = Vec::with_capacity(module.bodies().len() + 4);
    let mut func_num_locals: Vec<u32> = Vec::with_capacity(module.bodies().len() + 4);
    let lower_ctx = Lower8Context {
        module,
        allocs: &allocs,
        global_words: &global_words,
        div_builtins,
    };

    for (func_id, body) in module.bodies().iter().enumerate() {
        let is_entry = func_id == entry_func_id;
        let local_vregs = allocs[func_id].local_vregs.clone();

        let (blocks, entry_pc, num_locals, new_counter) = lower_function(
            &lower_ctx,
            body.as_ref(),
            func_id as u32,
            is_entry,
            local_vregs,
            vreg_counter,
        )?;

        vreg_counter = new_counter;
        func_blocks.push(blocks);
        func_entries.push(entry_pc);
        func_num_locals.push(num_locals);
    }

    if let Some(div_builtins) = div_builtins {
        let div_u_entry = Pc::new(div_builtins.div_u as u16 * PC_STRIDE);
        let rem_u_entry = Pc::new(div_builtins.rem_u as u16 * PC_STRIDE);
        let div_s_entry = Pc::new(div_builtins.div_s as u16 * PC_STRIDE);
        let rem_s_entry = Pc::new(div_builtins.rem_s as u16 * PC_STRIDE);

        let mut need_div_u = false;
        let mut need_rem_u = false;
        let mut need_div_s = false;
        let mut need_rem_s = false;
        for blocks in &func_blocks {
            for bb in blocks {
                if let Terminator8::CallSetup { callee_entry, .. } = bb.terminator {
                    if callee_entry == CallTarget::Pc(div_u_entry) {
                        need_div_u = true;
                    } else if callee_entry == CallTarget::Pc(rem_u_entry) {
                        need_rem_u = true;
                    } else if callee_entry == CallTarget::Pc(div_s_entry) {
                        need_div_s = true;
                    } else if callee_entry == CallTarget::Pc(rem_s_entry) {
                        need_rem_s = true;
                    }
                }
            }
        }

        for (func_id, kind, needed) in [
            (div_builtins.div_u, DivBuiltinKind::DivU, need_div_u),
            (div_builtins.rem_u, DivBuiltinKind::RemU, need_rem_u),
            (div_builtins.div_s, DivBuiltinKind::DivS, need_div_s),
            (div_builtins.rem_s, DivBuiltinKind::RemS, need_rem_s),
        ] {
            if needed {
                let local_vregs = allocs[func_id as usize].local_vregs.clone();
                let (blocks, entry_pc, num_locals, new_counter) =
                    lower_divrem_builtin_function(func_id, local_vregs, vreg_counter, kind)?;
                vreg_counter = new_counter;
                func_blocks.push(blocks);
                func_entries.push(entry_pc);
                func_num_locals.push(num_locals);
            } else {
                func_blocks.push(Vec::new());
                func_entries.push(Pc::new(func_id as u16 * PC_STRIDE));
                func_num_locals.push(2);
            }
        }
    }

    Ok(Ir8Program {
        entry_func: entry_func_id as u32,
        num_vregs: vreg_counter,
        func_blocks,
        func_entries,
        func_num_locals,
        cycles: Vec::new(),
        memory_end,
        init_bytes,
        global_init,
    })
}

fn lower_function(
    ctx: &Lower8Context<'_>,
    body: Option<&IrFuncBody>,
    func_id: u32,
    is_entry: bool,
    local_vregs: Vec<ValueWords>,
    vreg_start: u16,
) -> anyhow::Result<(Vec<BasicBlock8>, Pc, u32, u16)> {
    let body = match body {
        Some(b) => b,
        None => {
            return Ok((Vec::new(), Pc::new(0), 0, vreg_start));
        }
    };

    let num_locals = local_vregs
        .iter()
        .map(|value| u32::from(value.word_count()))
        .sum();
    let mut b = FuncBuilder::new(func_id, is_entry, vreg_start, local_vregs);

    let live_after_by_block = compute_live_after_by_block(body);
    let local_live_after_by_block = compute_local_live_after_by_block(body);

    for ir_block in body.blocks() {
        let pc = b.alloc_block();
        b.block_pc_map.insert(ir_block.id, pc);
    }

    for (block_idx, ir_block) in body.blocks().iter().enumerate() {
        let blk_pc = b.block_pc_map[&ir_block.id];
        b.switch_to(blk_pc);

        let ref_base = BasicBlock::ref_base(body.blocks(), block_idx);
        for (i, inst) in ir_block.insts.iter().enumerate() {
            ops::lower_inst(
                &mut b,
                ctx,
                inst,
                ref_base + i,
                &live_after_by_block[block_idx][i],
                &local_live_after_by_block[block_idx][i],
            )?;
        }

        lower_terminator(&mut b, ctx.module, &ir_block.terminator, ctx.allocs)?;
    }

    let vreg_end = b.vreg_counter;
    Ok((
        b.blocks,
        Pc::new(func_id as u16 * PC_STRIDE),
        num_locals,
        vreg_end,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasmparser::{RefType, ValType};

    const fn bid(value: usize) -> BlockId {
        BlockId(value)
    }

    const fn r(value: usize) -> IrNode {
        IrNode(value)
    }

    fn mk_sig(params: &[ValType], results: &[ValType]) -> crate::module::FuncType {
        crate::module::FuncType::new(
            params.to_vec().into_boxed_slice(),
            results.to_vec().into_boxed_slice(),
        )
    }

    fn set_ir_functions(
        module: &mut IrModule,
        funcs: Vec<(crate::module::FuncType, Option<crate::module::IrFuncBody>)>,
    ) {
        *module.functions_mut() = funcs.iter().map(|(sig, _)| sig.clone()).collect();
        *module.bodies_mut() = funcs.into_iter().map(|(_, body)| body).collect();
    }

    fn mk_single_main_module(main_block: BasicBlock) -> IrModule {
        let mut module = IrModule::new(crate::module::ModuleInfo::default(), vec![]);
        module.set_num_pages(1);
        module.set_entry_export(Some(0));
        set_ir_functions(
            &mut module,
            vec![(
                mk_sig(&[], &[ValType::I32]),
                Some(crate::module::IrFuncBody::new(
                    vec![],
                    bid(0),
                    vec![main_block],
                )),
            )],
        );
        module
    }

    fn mk_module_with(f: impl FnOnce(&mut IrModule)) -> IrModule {
        let mut module = IrModule::new(crate::module::ModuleInfo::default(), vec![]);
        f(&mut module);
        module
    }

    fn find_main_return_word(blocks: &[BasicBlock8]) -> Word {
        blocks
            .iter()
            .find_map(|bb| match bb.terminator {
                Terminator8::Exit { val: Some(v) } => Some(v.lo),
                _ => None,
            })
            .expect("expected main function exit value")
    }

    fn find_inst_def_kind(blocks: &[BasicBlock8], reg: Val8) -> &Inst8Kind {
        blocks
            .iter()
            .flat_map(|bb| bb.insts.iter())
            .find_map(|inst| (inst.dst == Some(reg)).then_some(&inst.kind))
            .unwrap_or_else(|| panic!("missing definition for v{}", reg.expect_vreg()))
    }

    fn count_builtin_calls(blocks: &[BasicBlock8], builtin: BuiltinId) -> usize {
        blocks
            .iter()
            .filter(|bb| {
                matches!(
                    bb.terminator,
                    Terminator8::CallSetup {
                        callee_entry: CallTarget::Builtin(id),
                        ..
                    } if id == builtin
                )
            })
            .count()
    }

    fn mk_trivial_main_block() -> BasicBlock {
        BasicBlock {
            id: bid(0),
            insts: vec![Inst::I32Const(0)],
            terminator: Terminator::Return(Some(r(0))),
        }
    }

    fn assert_lower8_error_contains(module: &IrModule, memory_bytes_cap: u32, expected: &str) {
        let err = lower8_module(module, memory_bytes_cap)
            .err()
            .expect("expected lower8_module to fail");
        let msg = format!("{err:#}");
        assert!(
            msg.contains(expected),
            "expected error containing {expected:?}, got: {msg}"
        );
    }

    #[test]
    fn lower8_rejects_modules_that_exceed_function_pc_space() {
        let sig = mk_sig(&[], &[ValType::I32]);
        let body = crate::module::IrFuncBody::new(vec![], bid(0), vec![mk_trivial_main_block()]);
        let mut module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_entry_export(Some(0));
            set_ir_functions(
                module,
                (0..63)
                    .map(|_| (sig.clone(), Some(body.clone())))
                    .collect::<Vec<_>>(),
            );
        });

        assert_lower8_error_contains(&module, 65536, "function PC slot");

        module.set_entry_export(Some(62));
        assert_lower8_error_contains(&module, 65536, "function PC slot");
    }

    #[test]
    fn lower8_memory_end_uses_explicit_memory_cap() {
        let mut module = mk_single_main_module(mk_trivial_main_block());
        module.globals_mut().push(crate::module::GlobalInfo::new(
            ValType::I32,
            crate::module::ConstInit::I32(4096),
        ));

        let program = lower8_module(&module, 2048).expect("lower8_module should succeed");
        assert_eq!(program.memory_end, 2048);
    }

    #[test]
    fn lower8_memory_end_uses_global0_when_memory_cap_is_zero() {
        let mut module = mk_single_main_module(mk_trivial_main_block());
        module.globals_mut().push(crate::module::GlobalInfo::new(
            ValType::I32,
            crate::module::ConstInit::I32(4096),
        ));

        let program = lower8_module(&module, 0).expect("lower8_module should succeed");
        assert_eq!(program.memory_end, 4096);
    }

    #[test]
    fn lower8_zero_memory_cap_requires_global0_stack_pointer() {
        let module = mk_single_main_module(mk_trivial_main_block());
        assert_lower8_error_contains(
            &module,
            0,
            "runtime memory limit is 0, but global 0 (stack pointer) is missing",
        );
    }

    #[test]
    fn lower8_reject_memory_cap_above_16bit_address_space_limit() {
        let module = mk_single_main_module(mk_trivial_main_block());
        assert_lower8_error_contains(
            &module,
            crate::constants::MAX_ADDRESSABLE_MEMORY_BYTES + 1,
            "runtime memory limit 65537 exceeds 16-bit address space limit 65536",
        );
    }

    #[test]
    fn lower8_reject_memory_larger_than_16bit_addressing() {
        let mut module = mk_single_main_module(mk_trivial_main_block());
        module.set_num_pages(2);

        assert_lower8_error_contains(
            &module,
            65536,
            "linear memory too large for 16-bit addressing (131072 bytes)",
        );
    }

    #[test]
    fn lower8_reject_out_of_bounds_data_segment() {
        let mut module = mk_single_main_module(mk_trivial_main_block());
        module.preloaded_data_mut().push((65535, vec![0xaa, 0xbb]));

        assert_lower8_error_contains(&module, 65536, "data segment out of bounds");
    }

    #[test]
    fn lower8_reject_non_i32_global_type() {
        let mut module = mk_single_main_module(mk_trivial_main_block());
        module.globals_mut().push(crate::module::GlobalInfo::new(
            ValType::I64,
            crate::module::ConstInit::I32(0),
        ));

        assert_lower8_error_contains(&module, 65536, "global 0 type/init mismatch");
    }

    #[test]
    fn lower8_reject_out_of_range_word_mem_offset() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(0),
                Inst::Load {
                    ty: ValType::I32,
                    size: 32,
                    signed: false,
                    offset: 0x1234_5678,
                    addr: r(0),
                },
            ],
            terminator: Terminator::Return(Some(r(1))),
        });

        assert_lower8_error_contains(
            &module,
            65536,
            "load offset 0x12345678 exceeds 16-bit address space",
        );
    }

    #[test]
    fn lower8_reject_out_of_range_halfword_mem_offset() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(0),
                Inst::I32Const(0),
                Inst::Store {
                    ty: ValType::I32,
                    size: 16,
                    offset: u16::MAX as u32,
                    addr: r(0),
                    val: r(1),
                },
            ],
            terminator: Terminator::Return(Some(r(0))),
        });

        assert_lower8_error_contains(
            &module,
            65536,
            "store offset 0xffff exceeds 16-bit address space for 2-byte access",
        );
    }

    #[test]
    fn lower8_reject_legacy_one_byte_load_width_alias() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(0),
                Inst::Load {
                    ty: ValType::I32,
                    size: 1,
                    signed: false,
                    offset: 0,
                    addr: r(0),
                },
            ],
            terminator: Terminator::Return(Some(r(1))),
        });

        assert_lower8_error_contains(
            &module,
            65536,
            "load memory width 1 is unsupported (expected 8/16/32/64 bits)",
        );
    }

    #[test]
    fn lower8_reject_legacy_four_byte_store_width_alias() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(0),
                Inst::I32Const(0),
                Inst::Store {
                    ty: ValType::I32,
                    size: 4,
                    offset: 0,
                    addr: r(0),
                    val: r(1),
                },
            ],
            terminator: Terminator::Return(Some(r(0))),
        });

        assert_lower8_error_contains(
            &module,
            65536,
            "store memory width 4 is unsupported (expected 8/16/32/64 bits)",
        );
    }

    #[test]
    fn lower8_divrem_lowering_uses_shared_helpers_and_keeps_trap_path() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(100), // 0
                Inst::I32Const(7),   // 1
                Inst::Binary {
                    op: BinOp::DivU,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                }, // 2
                Inst::Binary {
                    op: BinOp::RemU,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                }, // 3
                Inst::Binary {
                    op: BinOp::DivS,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                }, // 4
                Inst::Binary {
                    op: BinOp::RemS,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                }, // 5
                Inst::Binary {
                    op: BinOp::Add,
                    ty: ValType::I32,
                    lhs: r(2),
                    rhs: r(3),
                }, // 6
                Inst::Binary {
                    op: BinOp::Add,
                    ty: ValType::I32,
                    lhs: r(4),
                    rhs: r(5),
                }, // 7
                Inst::Binary {
                    op: BinOp::Add,
                    ty: ValType::I32,
                    lhs: r(6),
                    rhs: r(7),
                }, // 8
            ],
            terminator: Terminator::Return(Some(r(8))),
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let main_blocks = &program.func_blocks[0];

        assert!(
            main_blocks.iter().any(|bb| matches!(
                bb.terminator,
                Terminator8::CallSetup {
                    callee_entry,
                    ..
                } if callee_entry == CallTarget::Pc(Pc::new(PC_STRIDE))
                    || callee_entry == CallTarget::Pc(Pc::new(2 * PC_STRIDE))
                    || callee_entry == CallTarget::Pc(Pc::new(3 * PC_STRIDE))
                    || callee_entry == CallTarget::Pc(Pc::new(4 * PC_STRIDE))
            )),
            "expected non-trivial div/rem constants to call shared helper functions"
        );
        assert!(
            program.func_blocks.iter().skip(1).any(|blocks| blocks
                .iter()
                .any(|bb| matches!(bb.terminator, Terminator8::Trap(TrapCode::DivisionByZero)))),
            "expected divide-by-zero trap blocks in shared helper functions"
        );
    }

    #[test]
    fn lower8_variable_divrem_calls_shared_builtin_function() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(100),
                Inst::Getchar,
                Inst::Binary {
                    op: BinOp::DivU,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
            ],
            terminator: Terminator::Return(Some(r(2))),
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        assert_eq!(
            program.func_blocks.len(),
            5,
            "helper function-id slots are preserved; only referenced helpers have non-empty blocks"
        );

        let main_blocks = &program.func_blocks[0];
        let div_u_entry = Pc::new(PC_STRIDE);
        assert!(
            main_blocks.iter().any(|bb| {
                matches!(
                    bb.terminator,
                    Terminator8::CallSetup { callee_entry, .. }
                        if callee_entry == CallTarget::Pc(div_u_entry)
                )
            }),
            "expected variable division to call shared builtin function entry {}",
            div_u_entry.index()
        );
        assert!(
            !program.func_blocks[1].is_empty(),
            "div_u helper should be emitted when referenced"
        );
        assert!(
            program.func_blocks[2].is_empty()
                && program.func_blocks[3].is_empty()
                && program.func_blocks[4].is_empty(),
            "unused helpers should not emit blocks"
        );
    }

    #[test]
    fn lower8_variable_divrem_uses_js_coprocessor_builtins_when_enabled() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::Getchar,
                Inst::I32Const(7),
                Inst::Binary {
                    op: BinOp::DivU,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
                Inst::Binary {
                    op: BinOp::RemU,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
            ],
            terminator: Terminator::Return(Some(r(3))),
        });

        let program = lower8_module_with_config(
            &module,
            65536,
            Lower8Config {
                js_coprocessor: true,
            },
        )
        .expect("lower8_module_with_config should succeed");

        assert_eq!(
            program.func_blocks.len(),
            1,
            "js coprocessor mode should not append div/rem helper functions"
        );
        let main_blocks = &program.func_blocks[0];
        assert!(
            main_blocks.iter().any(|bb| {
                matches!(
                    bb.terminator,
                    Terminator8::CallSetup { callee_entry, .. }
                        if callee_entry == CallTarget::Builtin(BuiltinId::DivU32)
                )
            }),
            "expected div_u builtin call when js coprocessor lowering is enabled"
        );
        assert!(
            main_blocks.iter().any(|bb| {
                matches!(
                    bb.terminator,
                    Terminator8::CallSetup { callee_entry, .. }
                        if callee_entry == CallTarget::Builtin(BuiltinId::RemU32)
                )
            }),
            "expected rem_u builtin call when js coprocessor lowering is enabled"
        );
    }

    #[test]
    fn lower8_const_nontrivial_divrem_uses_shared_helpers() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(123456789),
                Inst::I32Const(10),
                Inst::Binary {
                    op: BinOp::DivU,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
                Inst::Binary {
                    op: BinOp::RemU,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
                Inst::Binary {
                    op: BinOp::Add,
                    ty: ValType::I32,
                    lhs: r(2),
                    rhs: r(3),
                },
            ],
            terminator: Terminator::Return(Some(r(4))),
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let main_blocks = &program.func_blocks[0];
        assert_eq!(
            program.func_blocks.len(),
            5,
            "helper function-id slots are preserved for div/rem helpers"
        );
        let div_u_entry = Pc::new(PC_STRIDE);
        let rem_u_entry = Pc::new(2 * PC_STRIDE);

        assert!(
            main_blocks.iter().any(|bb| {
                matches!(
                    bb.terminator,
                    Terminator8::CallSetup { callee_entry, .. }
                        if callee_entry == CallTarget::Pc(div_u_entry)
                            || callee_entry == CallTarget::Pc(rem_u_entry)
                )
            }),
            "non-trivial constant div/rem should call shared div/rem helper functions"
        );
        assert!(
            !main_blocks
                .iter()
                .any(|bb| { matches!(bb.terminator, Terminator8::Trap(TrapCode::DivisionByZero)) }),
            "divide-by-zero trap should be inside shared helper, not caller block"
        );
        assert!(
            !program.func_blocks[1].is_empty() && !program.func_blocks[2].is_empty(),
            "div_u/rem_u helpers should be emitted when referenced"
        );
        assert!(
            program.func_blocks[3].is_empty() && program.func_blocks[4].is_empty(),
            "unused signed helpers should not emit blocks"
        );
    }

    #[test]
    fn lower8_divrem_helper_dispatches_on_high_dividend_bytes() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(100),
                Inst::Getchar,
                Inst::Binary {
                    op: BinOp::DivU,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
            ],
            terminator: Terminator::Return(Some(r(2))),
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let div_u_entry = Pc::new(PC_STRIDE);
        let main_blocks = &program.func_blocks[0];
        let helper_numer_word = main_blocks
            .iter()
            .find_map(|bb| match bb.terminator {
                Terminator8::CallSetup {
                    callee_entry,
                    ref callee_arg_vregs,
                    ..
                } if callee_entry == CallTarget::Pc(div_u_entry) => {
                    callee_arg_vregs.first().copied()
                }
                _ => None,
            })
            .expect("main should call div_u helper with argument vregs");

        let mut saw_b3_check = false;
        let mut saw_b2_check = false;
        let mut saw_b1_check = false;
        let helper_blocks = &program.func_blocks[1];
        for bb in helper_blocks {
            for inst in &bb.insts {
                if let Inst8Kind::Eq(lhs, rhs) = inst.kind {
                    let checks = |byte: Val8| {
                        (lhs == byte && rhs.imm_value() == Some(0))
                            || (rhs == byte && lhs.imm_value() == Some(0))
                    };
                    saw_b3_check |= checks(helper_numer_word.b3);
                    saw_b2_check |= checks(helper_numer_word.b2);
                    saw_b1_check |= checks(helper_numer_word.b1);
                }
            }
        }

        assert!(
            saw_b3_check && saw_b2_check && saw_b1_check,
            "expected div_u helper to check dividend high bytes b3/b2/b1 for short-path dispatch"
        );
    }

    #[test]
    fn lower8_divrem_helper_reuses_subtract_borrow_instead_of_ltu_chain() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::Getchar,
                Inst::Getchar,
                Inst::Binary {
                    op: BinOp::DivU,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
            ],
            terminator: Terminator::Return(Some(r(2))),
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let div_u_entry = Pc::new(PC_STRIDE);
        let main_blocks = &program.func_blocks[0];
        let helper_denom_word = main_blocks
            .iter()
            .find_map(|bb| match bb.terminator {
                Terminator8::CallSetup {
                    callee_entry,
                    ref callee_arg_vregs,
                    ..
                } if callee_entry == CallTarget::Pc(div_u_entry) => {
                    callee_arg_vregs.get(1).copied()
                }
                _ => None,
            })
            .expect("main should call div_u helper with denominator vregs");

        let helper_blocks = &program.func_blocks[1];
        assert!(
            helper_blocks
                .iter()
                .flat_map(|bb| bb.insts.iter())
                .all(|inst| !matches!(inst.kind, Inst8Kind::LtU(_, _))),
            "div_u helper should derive restore-step ordering from subtract borrow, not lt_u chain"
        );

        let mut defs = HashMap::new();
        for bb in helper_blocks {
            for inst in &bb.insts {
                if let Some(dst) = inst.dst {
                    defs.insert(dst, inst.kind);
                }
            }
        }

        assert!(
            helper_blocks
                .iter()
                .flat_map(|bb| bb.insts.iter())
                .any(|inst| {
                    let Inst8Kind::BoolNot(src) = inst.kind else {
                        return false;
                    };
                    matches!(defs.get(&src), Some(Inst8Kind::Sub32Borrow { rhs, .. }) if *rhs == helper_denom_word)
                }),
            "div_u helper should negate the final high-byte subtract borrow to form the quotient bit"
        );
    }

    #[test]
    fn lower8_const_shift_rotate_inlines_without_builtin_calls() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::Getchar,
                Inst::I32Const(5),
                Inst::Binary {
                    op: BinOp::Shl,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
                Inst::Binary {
                    op: BinOp::ShrU,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
                Inst::Binary {
                    op: BinOp::ShrS,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
                Inst::Binary {
                    op: BinOp::Rotl,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
                Inst::Binary {
                    op: BinOp::Rotr,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
            ],
            terminator: Terminator::Return(Some(r(6))),
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let main_blocks = &program.func_blocks[0];

        for builtin in [
            BuiltinId::Shl32,
            BuiltinId::ShrU32,
            BuiltinId::ShrS32,
            BuiltinId::Rotl32,
            BuiltinId::Rotr32,
        ] {
            assert_eq!(
                count_builtin_calls(main_blocks, builtin),
                0,
                "constant shifts/rotates should not call {}",
                builtin.name()
            );
        }
    }

    #[test]
    fn lower8_variable_shift_rotate_still_calls_builtins() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::Getchar,
                Inst::Getchar,
                Inst::Binary {
                    op: BinOp::Shl,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
                Inst::Binary {
                    op: BinOp::ShrU,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
                Inst::Binary {
                    op: BinOp::ShrS,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
                Inst::Binary {
                    op: BinOp::Rotl,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
                Inst::Binary {
                    op: BinOp::Rotr,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
            ],
            terminator: Terminator::Return(Some(r(6))),
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let main_blocks = &program.func_blocks[0];

        assert_eq!(count_builtin_calls(main_blocks, BuiltinId::Shl32), 1);
        assert_eq!(count_builtin_calls(main_blocks, BuiltinId::ShrU32), 1);
        assert_eq!(count_builtin_calls(main_blocks, BuiltinId::ShrS32), 1);
        assert_eq!(count_builtin_calls(main_blocks, BuiltinId::Rotl32), 1);
        assert_eq!(count_builtin_calls(main_blocks, BuiltinId::Rotr32), 1);
    }

    #[test]
    fn lower8_pow2_divu_constant_uses_inline_shift_path() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::Getchar,
                Inst::I32Const(8),
                Inst::Binary {
                    op: BinOp::DivU,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                },
            ],
            terminator: Terminator::Return(Some(r(2))),
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let main_blocks = &program.func_blocks[0];

        assert!(
            !main_blocks
                .iter()
                .any(|bb| matches!(bb.terminator, Terminator8::CallSetup { .. })),
            "div_u by a power-of-two constant should inline without call_setup"
        );
    }

    #[test]
    fn lower8_load8_s_sign_extends_to_word() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(0),
                Inst::Load {
                    ty: ValType::I32,
                    size: 8,
                    signed: true,
                    offset: 5,
                    addr: r(0),
                },
            ],
            terminator: Terminator::Return(Some(r(1))),
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let blocks = &program.func_blocks[0];
        let ret = find_main_return_word(blocks);

        let loads: Vec<(u16, u8)> = blocks
            .iter()
            .flat_map(|bb| bb.insts.iter())
            .filter_map(|inst| match inst.kind {
                Inst8Kind::LoadMem { base, lane, .. } => Some((base, lane)),
                _ => None,
            })
            .collect();
        assert_eq!(loads, vec![(5, 0)], "load8_s should load only one byte");

        let b1_fill = match find_inst_def_kind(blocks, ret.b1) {
            Inst8Kind::Copy(src) => *src,
            other => panic!("expected b1 sign fill copy, got {other:?}"),
        };
        let b2_fill = match find_inst_def_kind(blocks, ret.b2) {
            Inst8Kind::Copy(src) => *src,
            other => panic!("expected b2 sign fill copy, got {other:?}"),
        };
        let b3_fill = match find_inst_def_kind(blocks, ret.b3) {
            Inst8Kind::Copy(src) => *src,
            other => panic!("expected b3 sign fill copy, got {other:?}"),
        };
        assert_eq!(b1_fill, b2_fill);
        assert_eq!(b1_fill, b3_fill);
        let is_neg = match find_inst_def_kind(blocks, b1_fill) {
            Inst8Kind::Sub(zero, is_neg) if zero.imm_value() == Some(0) => *is_neg,
            other => panic!(
                "expected sign fill to come from subtracting a boolean from zero, got {other:?}"
            ),
        };
        assert!(
            matches!(find_inst_def_kind(blocks, is_neg), Inst8Kind::GeU(src, thresh) if *src == ret.b0 && thresh.imm_value() == Some(0x80)),
            "expected load8_s sign fill to test the high bit with >= 0x80"
        );
    }

    #[test]
    fn lower8_load16_s_sign_extends_to_word() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(0),
                Inst::Load {
                    ty: ValType::I32,
                    size: 16,
                    signed: true,
                    offset: 6,
                    addr: r(0),
                },
            ],
            terminator: Terminator::Return(Some(r(1))),
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let blocks = &program.func_blocks[0];
        let ret = find_main_return_word(blocks);

        let loads: Vec<(u16, u8)> = blocks
            .iter()
            .flat_map(|bb| bb.insts.iter())
            .filter_map(|inst| match inst.kind {
                Inst8Kind::LoadMem { base, lane, .. } => Some((base, lane)),
                _ => None,
            })
            .collect();
        assert_eq!(
            loads,
            vec![(6, 0), (6, 1)],
            "load16_s should load exactly two bytes"
        );

        let b2_fill = match find_inst_def_kind(blocks, ret.b2) {
            Inst8Kind::Copy(src) => *src,
            other => panic!("expected b2 sign fill copy, got {other:?}"),
        };
        let b3_fill = match find_inst_def_kind(blocks, ret.b3) {
            Inst8Kind::Copy(src) => *src,
            other => panic!("expected b3 sign fill copy, got {other:?}"),
        };
        assert_eq!(b2_fill, b3_fill);
        let is_neg = match find_inst_def_kind(blocks, b2_fill) {
            Inst8Kind::Sub(zero, is_neg) if zero.imm_value() == Some(0) => *is_neg,
            other => panic!(
                "expected sign fill to come from subtracting a boolean from zero, got {other:?}"
            ),
        };
        assert!(
            matches!(find_inst_def_kind(blocks, is_neg), Inst8Kind::GeU(src, thresh) if *src == ret.b1 && thresh.imm_value() == Some(0x80)),
            "expected load16_s sign fill to test the high bit with >= 0x80"
        );
    }

    #[test]
    fn lower8_load16_u_zero_extends_to_word() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(0),
                Inst::Load {
                    ty: ValType::I32,
                    size: 16,
                    signed: false,
                    offset: 6,
                    addr: r(0),
                },
            ],
            terminator: Terminator::Return(Some(r(1))),
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let blocks = &program.func_blocks[0];
        let ret = find_main_return_word(blocks);

        let loads: Vec<(u16, u8)> = blocks
            .iter()
            .flat_map(|bb| bb.insts.iter())
            .filter_map(|inst| match inst.kind {
                Inst8Kind::LoadMem { base, lane, .. } => Some((base, lane)),
                _ => None,
            })
            .collect();
        assert_eq!(
            loads,
            vec![(6, 0), (6, 1)],
            "load16_u should load two bytes"
        );

        let b2_fill = match find_inst_def_kind(blocks, ret.b2) {
            Inst8Kind::Copy(src) => *src,
            other => panic!("expected b2 zero fill copy, got {other:?}"),
        };
        let b3_fill = match find_inst_def_kind(blocks, ret.b3) {
            Inst8Kind::Copy(src) => *src,
            other => panic!("expected b3 zero fill copy, got {other:?}"),
        };
        assert_eq!(b2_fill, b3_fill);
        assert!(
            b2_fill == Val8::imm(0),
            "expected zero extension fill to be constant zero"
        );
    }

    #[test]
    fn lower8_store16_writes_two_bytes() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(0),
                Inst::I32Const(0x1122_3344),
                Inst::Store {
                    ty: ValType::I32,
                    size: 16,
                    offset: 9,
                    addr: r(0),
                    val: r(1),
                },
            ],
            terminator: Terminator::Return(Some(r(0))),
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let blocks = &program.func_blocks[0];
        let mut stores: Vec<(u16, u8)> = blocks
            .iter()
            .flat_map(|bb| bb.insts.iter())
            .filter_map(|inst| match inst.kind {
                Inst8Kind::StoreMem { base, lane, .. } => Some((base, lane)),
                _ => None,
            })
            .collect();
        stores.sort_unstable();
        assert_eq!(stores, vec![(9, 0), (9, 1)]);
    }

    #[test]
    fn lower8_table_size_returns_table_entry_count() {
        let mut module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![Inst::TableSize(0)],
            terminator: Terminator::Return(Some(r(0))),
        });
        module.tables_mut().push(crate::module::TableInfo::new(
            RefType::FUNCREF,
            vec![None, None, Some(0)],
        ));

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let ret = find_main_return_word(&program.func_blocks[0]);
        assert_eq!(
            ret,
            Word::from_u32_imm(3),
            "table.size should lower to the table's entry count"
        );
    }

    #[test]
    fn lower8_table_size_rejects_missing_table() {
        let module = mk_single_main_module(BasicBlock {
            id: bid(0),
            insts: vec![Inst::TableSize(1)],
            terminator: Terminator::Return(Some(r(0))),
        });

        assert_lower8_error_contains(
            &module,
            65536,
            "table.size references table 1 which does not exist",
        );
    }

    #[test]
    fn lower8_call_spills_live_non_local_address_word() {
        let fib_sig = mk_sig(&[ValType::I32], &[ValType::I32]);
        let main_sig = mk_sig(&[], &[ValType::I32]);

        let fib_block = BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::LocalGet(0), // 0: n
                Inst::I32Const(4), // 1
                Inst::Binary {
                    op: BinOp::Mul,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(1),
                }, // 2: n*4
                Inst::I32Const(16), // 3: memo base
                Inst::Binary {
                    op: BinOp::Add,
                    ty: ValType::I32,
                    lhs: r(3),
                    rhs: r(2),
                }, // 4: addr
                Inst::I32Const(1), // 5
                Inst::Binary {
                    op: BinOp::Sub,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(5),
                }, // 6: n-1
                Inst::Call {
                    func: 0,
                    args: vec![r(6)],
                }, // 7
                Inst::I32Const(2), // 8
                Inst::Binary {
                    op: BinOp::Sub,
                    ty: ValType::I32,
                    lhs: r(0),
                    rhs: r(8),
                }, // 9: n-2
                Inst::Call {
                    func: 0,
                    args: vec![r(9)],
                }, // 10
                Inst::Binary {
                    op: BinOp::Add,
                    ty: ValType::I32,
                    lhs: r(7),
                    rhs: r(10),
                }, // 11: fib(n-1)+fib(n-2)
                Inst::Store {
                    ty: ValType::I32,
                    size: 32,
                    offset: 0,
                    addr: r(4),
                    val: r(11),
                }, // 12: memo[n] = sum
            ],
            terminator: Terminator::Return(Some(r(11))),
        };

        let main_block = BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(8),
                Inst::Call {
                    func: 0,
                    args: vec![r(0)],
                },
            ],
            terminator: Terminator::Return(Some(r(1))),
        };

        let module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_entry_export(Some(1));
            set_ir_functions(
                module,
                vec![
                    (
                        fib_sig.clone(),
                        Some(crate::module::IrFuncBody::new(
                            vec![ValType::I32],
                            bid(0),
                            vec![fib_block],
                        )),
                    ),
                    (
                        main_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![],
                            bid(0),
                            vec![main_block],
                        )),
                    ),
                ],
            );
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let fib_blocks = &program.func_blocks[0];

        let mut memo_store_addr: Option<crate::ir8::Addr> = None;
        for blk in fib_blocks {
            for inst in &blk.insts {
                if let Inst8Kind::StoreMem { base, addr, .. } = &inst.kind
                    && *base == 0
                {
                    memo_store_addr = Some(*addr);
                    break;
                }
            }
            if memo_store_addr.is_some() {
                break;
            }
        }
        let addr = memo_store_addr.expect("expected lowered memo store");

        // One local word (n) => byte offsets [0..3] are local saves. Any spill must be >= 4.
        let spill_base = 4u16;
        let mut lo_spilled = false;
        let mut hi_spilled = false;
        let mut lo_restored = false;
        let mut hi_restored = false;

        for blk in fib_blocks {
            for inst in &blk.insts {
                match &inst.kind {
                    Inst8Kind::CsStore { offset, val } if *offset >= spill_base => {
                        if addr.lo.reg_index() == val.reg_index() {
                            lo_spilled = true;
                        }
                        if addr.hi.reg_index() == val.reg_index() {
                            hi_spilled = true;
                        }
                    }
                    Inst8Kind::CsLoad { offset } if *offset >= spill_base => {
                        if addr.lo.reg_index() == inst.dst.unwrap().reg_index() {
                            lo_restored = true;
                        }
                        if addr.hi.reg_index() == inst.dst.unwrap().reg_index() {
                            hi_restored = true;
                        }
                    }
                    _ => {}
                }
            }
        }

        assert!(
            lo_spilled,
            "memo address lo byte was not spilled around call"
        );
        assert!(
            hi_spilled,
            "memo address hi byte was not spilled around call"
        );
        assert!(
            lo_restored,
            "memo address lo byte was not restored after call"
        );
        assert!(
            hi_restored,
            "memo address hi byte was not restored after call"
        );
    }

    #[test]
    fn lower8_call_indirect_uses_dispatch_and_callstack_ra() {
        let callee_sig = mk_sig(&[ValType::I32], &[ValType::I32]);
        let main_sig = mk_sig(&[], &[ValType::I32]);

        let callee_block = BasicBlock {
            id: bid(0),
            insts: vec![Inst::LocalGet(0)],
            terminator: Terminator::Return(Some(r(0))),
        };

        let main_block = BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(7),
                Inst::I32Const(0),
                Inst::CallIndirect {
                    type_index: 0,
                    table_index: 0,
                    index: r(1),
                    args: vec![r(0)],
                },
            ],
            terminator: Terminator::Return(Some(r(2))),
        };

        let module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_entry_export(Some(1));
            *module.types_mut() = vec![callee_sig.clone()];
            *module.tables_mut() = vec![crate::module::TableInfo::new(
                RefType::FUNCREF,
                vec![Some(0)],
            )];
            set_ir_functions(
                module,
                vec![
                    (
                        callee_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![ValType::I32],
                            bid(0),
                            vec![callee_block],
                        )),
                    ),
                    (
                        main_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![],
                            bid(0),
                            vec![main_block],
                        )),
                    ),
                ],
            );
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let main_blocks = &program.func_blocks[1];

        assert!(
            main_blocks
                .iter()
                .any(|bb| matches!(bb.terminator, Terminator8::Switch { .. })),
            "expected indirect call dispatch switch in main"
        );
        assert!(
            main_blocks.iter().any(|bb| matches!(
                bb.terminator,
                Terminator8::CallSetup { callee_entry, .. }
                    if callee_entry == CallTarget::Pc(Pc::new(0))
            )),
            "expected call_indirect to call function 0"
        );
        assert!(
            main_blocks
                .iter()
                .flat_map(|bb| bb.insts.iter())
                .any(|inst| matches!(inst.kind, Inst8Kind::CsStorePc { .. })),
            "expected call_indirect to spill RA onto the call stack"
        );
        assert!(
            main_blocks
                .iter()
                .any(|bb| matches!(bb.terminator, Terminator8::Trap(TrapCode::Unreachable))),
            "expected invalid indirect targets to trap"
        );
    }

    #[test]
    fn lower8_call_indirect_rejects_missing_table() {
        let main_block = BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(0),
                Inst::CallIndirect {
                    type_index: 0,
                    table_index: 1,
                    index: r(0),
                    args: vec![],
                },
            ],
            terminator: Terminator::Return(Some(r(1))),
        };

        let module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_entry_export(Some(0));
            *module.types_mut() = vec![mk_sig(&[], &[ValType::I32])];
            set_ir_functions(
                module,
                vec![(
                    mk_sig(&[], &[ValType::I32]),
                    Some(crate::module::IrFuncBody::new(
                        vec![],
                        bid(0),
                        vec![main_block],
                    )),
                )],
            );
        });

        let err = match lower8_module(&module, 65536) {
            Ok(_) => panic!("missing call_indirect table should fail"),
            Err(err) => err,
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("call_indirect references table 1 which does not exist"),
            "unexpected lower8 error: {msg}"
        );
    }

    #[test]
    fn lower8_call_indirect_rejects_signature_arg_mismatch() {
        let main_block = BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(0),
                Inst::CallIndirect {
                    type_index: 0,
                    table_index: 0,
                    index: r(0),
                    args: vec![],
                },
            ],
            terminator: Terminator::Return(Some(r(1))),
        };

        let module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_entry_export(Some(0));
            *module.types_mut() = vec![mk_sig(&[ValType::I32], &[ValType::I32])];
            *module.tables_mut() =
                vec![crate::module::TableInfo::new(RefType::FUNCREF, vec![None])];
            set_ir_functions(
                module,
                vec![(
                    mk_sig(&[], &[ValType::I32]),
                    Some(crate::module::IrFuncBody::new(
                        vec![],
                        bid(0),
                        vec![main_block],
                    )),
                )],
            );
        });

        let err = match lower8_module(&module, 65536) {
            Ok(_) => panic!("call_indirect arg mismatch should fail"),
            Err(err) => err,
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("call_indirect type 0 expects 1 arg word(s), got 0"),
            "unexpected lower8 error: {msg}"
        );
    }

    #[test]
    fn lower8_call_indirect_rejects_table_larger_than_256_entries() {
        let main_block = BasicBlock {
            id: bid(0),
            insts: vec![
                Inst::I32Const(0),
                Inst::CallIndirect {
                    type_index: 0,
                    table_index: 0,
                    index: r(0),
                    args: vec![],
                },
            ],
            terminator: Terminator::Return(Some(r(1))),
        };

        let module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_entry_export(Some(0));
            *module.types_mut() = vec![mk_sig(&[], &[ValType::I32])];
            *module.tables_mut() = vec![crate::module::TableInfo::new(
                RefType::FUNCREF,
                vec![None; 257],
            )];
            set_ir_functions(
                module,
                vec![(
                    mk_sig(&[], &[ValType::I32]),
                    Some(crate::module::IrFuncBody::new(
                        vec![],
                        bid(0),
                        vec![main_block],
                    )),
                )],
            );
        });

        let err = match lower8_module(&module, 65536) {
            Ok(_) => panic!("oversized call_indirect table should fail"),
            Err(err) => err,
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("call_indirect table 0 has 257 entries; max supported is 256"),
            "unexpected lower8 error: {msg}"
        );
    }

    #[test]
    fn lower8_tail_call_direct_skips_caller_callstack_traffic() {
        let callee_sig = mk_sig(&[ValType::I32], &[ValType::I32]);
        let caller_sig = mk_sig(&[], &[ValType::I32]);

        let callee_block = BasicBlock {
            id: bid(0),
            insts: vec![Inst::LocalGet(0)],
            terminator: Terminator::Return(Some(r(0))),
        };

        let caller_block = BasicBlock {
            id: bid(0),
            insts: vec![Inst::I32Const(42)],
            terminator: Terminator::TailCall {
                func: 0,
                args: vec![r(0)],
            },
        };

        let module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_entry_export(Some(0));
            set_ir_functions(
                module,
                vec![
                    (
                        callee_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![ValType::I32],
                            bid(0),
                            vec![callee_block],
                        )),
                    ),
                    (
                        caller_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![],
                            bid(0),
                            vec![caller_block],
                        )),
                    ),
                ],
            );
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let caller_blocks = &program.func_blocks[1];

        assert!(caller_blocks.iter().any(|bb| matches!(
            bb.terminator,
            Terminator8::CallSetup {
                callee_entry,
                cont,
                ..
            } if callee_entry == CallTarget::Pc(Pc::new(0)) && cont == Pc::new(0)
        )));
        assert!(
            caller_blocks
                .iter()
                .flat_map(|bb| bb.insts.iter())
                .all(|inst| !matches!(
                    inst.kind,
                    Inst8Kind::CsStore { .. }
                        | Inst8Kind::CsLoad { .. }
                        | Inst8Kind::CsStorePc { .. }
                        | Inst8Kind::CsLoadPc { .. }
                        | Inst8Kind::CsAlloc(_)
                        | Inst8Kind::CsFree(_)
                )),
            "tail-call direct should not emit caller callstack save/restore instructions"
        );
    }

    #[test]
    fn lower8_tail_call_indirect_direct_target_skips_caller_callstack_traffic() {
        let callee_sig = mk_sig(&[ValType::I32], &[ValType::I32]);
        let caller_sig = mk_sig(&[], &[ValType::I32]);

        let callee_block = BasicBlock {
            id: bid(0),
            insts: vec![Inst::LocalGet(0)],
            terminator: Terminator::Return(Some(r(0))),
        };

        let caller_block = BasicBlock {
            id: bid(0),
            insts: vec![Inst::I32Const(7), Inst::I32Const(0)],
            terminator: Terminator::TailCallIndirect {
                type_index: 0,
                table_index: 0,
                index: r(1),
                args: vec![r(0)],
            },
        };

        let module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_entry_export(Some(0));
            *module.types_mut() = vec![callee_sig.clone()];
            *module.tables_mut() = vec![crate::module::TableInfo::new(
                RefType::FUNCREF,
                vec![Some(0)],
            )];
            set_ir_functions(
                module,
                vec![
                    (
                        callee_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![ValType::I32],
                            bid(0),
                            vec![callee_block],
                        )),
                    ),
                    (
                        caller_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![],
                            bid(0),
                            vec![caller_block],
                        )),
                    ),
                ],
            );
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let caller_blocks = &program.func_blocks[1];

        assert!(
            caller_blocks
                .iter()
                .any(|bb| matches!(bb.terminator, Terminator8::Switch { .. })),
            "expected indirect dispatch switch in caller"
        );
        assert!(caller_blocks.iter().any(|bb| matches!(
            bb.terminator,
            Terminator8::CallSetup {
                callee_entry,
                cont,
                ..
            } if callee_entry == CallTarget::Pc(Pc::new(0)) && cont == Pc::new(0)
        )));
        assert!(
            caller_blocks
                .iter()
                .flat_map(|bb| bb.insts.iter())
                .all(|inst| !matches!(
                    inst.kind,
                    Inst8Kind::CsStore { .. }
                        | Inst8Kind::CsLoad { .. }
                        | Inst8Kind::CsStorePc { .. }
                        | Inst8Kind::CsLoadPc { .. }
                        | Inst8Kind::CsAlloc(_)
                        | Inst8Kind::CsFree(_)
                )),
            "tail-call indirect direct target should not emit caller callstack save/restore instructions"
        );
    }

    #[test]
    fn lower8_tail_call_rejects_missing_function() {
        let helper_sig = mk_sig(&[], &[ValType::I32]);
        let main_sig = mk_sig(&[], &[ValType::I32]);

        let helper_block = BasicBlock {
            id: bid(0),
            insts: vec![],
            terminator: Terminator::TailCall {
                func: 99,
                args: vec![],
            },
        };

        let main_block = BasicBlock {
            id: bid(0),
            insts: vec![Inst::I32Const(0)],
            terminator: Terminator::Return(Some(r(0))),
        };

        let module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_entry_export(Some(1));
            set_ir_functions(
                module,
                vec![
                    (
                        helper_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![],
                            bid(0),
                            vec![helper_block],
                        )),
                    ),
                    (
                        main_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![],
                            bid(0),
                            vec![main_block],
                        )),
                    ),
                ],
            );
        });

        let err = match lower8_module(&module, 65536) {
            Ok(_) => panic!("tail_call to missing function should fail"),
            Err(err) => err,
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("tail_call references missing function 99"),
            "unexpected lower8 error: {msg}"
        );
    }

    #[test]
    fn lower8_tail_call_indirect_rejects_signature_arg_mismatch() {
        let helper_sig = mk_sig(&[], &[ValType::I32]);
        let main_sig = mk_sig(&[], &[ValType::I32]);

        let helper_block = BasicBlock {
            id: bid(0),
            insts: vec![Inst::I32Const(0)],
            terminator: Terminator::TailCallIndirect {
                type_index: 0,
                table_index: 0,
                index: r(0),
                args: vec![],
            },
        };

        let main_block = BasicBlock {
            id: bid(0),
            insts: vec![Inst::I32Const(0)],
            terminator: Terminator::Return(Some(r(0))),
        };

        let module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_entry_export(Some(1));
            *module.types_mut() = vec![mk_sig(&[ValType::I32], &[ValType::I32])];
            *module.tables_mut() =
                vec![crate::module::TableInfo::new(RefType::FUNCREF, vec![None])];
            set_ir_functions(
                module,
                vec![
                    (
                        helper_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![],
                            bid(0),
                            vec![helper_block],
                        )),
                    ),
                    (
                        main_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![],
                            bid(0),
                            vec![main_block],
                        )),
                    ),
                ],
            );
        });

        let err = match lower8_module(&module, 65536) {
            Ok(_) => panic!("tail_call_indirect arg mismatch should fail"),
            Err(err) => err,
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("tail_call_indirect type 0 expects 1 arg word(s), got 0"),
            "unexpected lower8 error: {msg}"
        );
    }

    #[test]
    fn lower8_tail_call_indirect_rejects_table_larger_than_256_entries() {
        let helper_sig = mk_sig(&[], &[ValType::I32]);
        let main_sig = mk_sig(&[], &[ValType::I32]);

        let helper_block = BasicBlock {
            id: bid(0),
            insts: vec![Inst::I32Const(0)],
            terminator: Terminator::TailCallIndirect {
                type_index: 0,
                table_index: 0,
                index: r(0),
                args: vec![],
            },
        };

        let main_block = BasicBlock {
            id: bid(0),
            insts: vec![Inst::I32Const(0)],
            terminator: Terminator::Return(Some(r(0))),
        };

        let module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_entry_export(Some(1));
            *module.types_mut() = vec![mk_sig(&[], &[ValType::I32])];
            *module.tables_mut() = vec![crate::module::TableInfo::new(
                RefType::FUNCREF,
                vec![None; 257],
            )];
            set_ir_functions(
                module,
                vec![
                    (
                        helper_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![],
                            bid(0),
                            vec![helper_block],
                        )),
                    ),
                    (
                        main_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![],
                            bid(0),
                            vec![main_block],
                        )),
                    ),
                ],
            );
        });

        let err = match lower8_module(&module, 65536) {
            Ok(_) => panic!("oversized tail_call_indirect table should fail"),
            Err(err) => err,
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("tail_call_indirect table 0 has 257 entries; max supported is 256"),
            "unexpected lower8 error: {msg}"
        );
    }

    #[test]
    fn lower8_tail_call_indirect_builtin_returns_through_caller_ra() {
        let putchar_sig = mk_sig(&[ValType::I32], &[ValType::I32]);
        let main_sig = mk_sig(&[], &[ValType::I32]);

        let caller_block = BasicBlock {
            id: bid(0),
            insts: vec![Inst::I32Const(b'X' as i32), Inst::I32Const(0)],
            terminator: Terminator::TailCallIndirect {
                type_index: 0,
                table_index: 0,
                index: r(1),
                args: vec![r(0)],
            },
        };

        let main_block = BasicBlock {
            id: bid(0),
            insts: vec![Inst::I32Const(0)],
            terminator: Terminator::Return(Some(r(0))),
        };

        let module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_num_imported_funcs(1);
            module.set_putchar_import(Some(0));
            module.set_entry_export(Some(2));
            *module.types_mut() = vec![putchar_sig.clone()];
            *module.tables_mut() = vec![crate::module::TableInfo::new(
                RefType::FUNCREF,
                vec![Some(0)],
            )];
            set_ir_functions(
                module,
                vec![
                    (putchar_sig.clone(), None),
                    (
                        main_sig.clone(),
                        Some(crate::module::IrFuncBody::new(
                            vec![],
                            bid(0),
                            vec![caller_block],
                        )),
                    ),
                    (
                        main_sig,
                        Some(crate::module::IrFuncBody::new(
                            vec![],
                            bid(0),
                            vec![main_block],
                        )),
                    ),
                ],
            );
        });

        let program = lower8_module(&module, 65536).expect("lower8_module should succeed");
        let caller_blocks = &program.func_blocks[1];
        assert!(
            caller_blocks
                .iter()
                .flat_map(|bb| bb.insts.iter())
                .any(|inst| matches!(inst.kind, Inst8Kind::Putchar(_))),
            "expected builtin putchar path in tail_call_indirect"
        );
        assert!(
            caller_blocks
                .iter()
                .flat_map(|bb| bb.insts.iter())
                .any(|inst| matches!(inst.kind, Inst8Kind::CsFree(1))),
            "expected tail_call_indirect builtin path to pop caller RA"
        );
        assert!(
            caller_blocks
                .iter()
                .flat_map(|bb| bb.insts.iter())
                .any(|inst| matches!(inst.kind, Inst8Kind::CsLoadPc { offset: 0 })),
            "expected tail_call_indirect builtin path to reload caller RA"
        );
        assert!(
            caller_blocks
                .iter()
                .any(|bb| matches!(bb.terminator, Terminator8::Return { .. })),
            "expected builtin tail_call_indirect to return through caller RA"
        );
    }

    #[test]
    fn lower8_main_tail_call_terminator_is_rejected() {
        let module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_entry_export(Some(0));
            set_ir_functions(
                module,
                vec![(
                    mk_sig(&[], &[ValType::I32]),
                    Some(crate::module::IrFuncBody::new(
                        vec![],
                        bid(0),
                        vec![BasicBlock {
                            id: bid(0),
                            insts: vec![],
                            terminator: Terminator::TailCall {
                                func: 0,
                                args: vec![],
                            },
                        }],
                    )),
                )],
            );
        });

        let err = match lower8_module(&module, 65536) {
            Ok(_) => panic!("main tail_call should fail"),
            Err(err) => err,
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("entry function '_start' must not contain tail_call terminators"),
            "unexpected lower8 error: {msg}"
        );
    }

    #[test]
    fn lower8_main_tail_call_indirect_terminator_is_rejected() {
        let module = mk_module_with(|module| {
            module.set_num_pages(1);
            module.set_entry_export(Some(0));
            set_ir_functions(
                module,
                vec![(
                    mk_sig(&[], &[ValType::I32]),
                    Some(crate::module::IrFuncBody::new(
                        vec![],
                        bid(0),
                        vec![BasicBlock {
                            id: bid(0),
                            insts: vec![Inst::I32Const(0)],
                            terminator: Terminator::TailCallIndirect {
                                type_index: 0,
                                table_index: 0,
                                index: r(0),
                                args: vec![],
                            },
                        }],
                    )),
                )],
            );
        });

        let err = match lower8_module(&module, 65536) {
            Ok(_) => panic!("main tail_call_indirect should fail"),
            Err(err) => err,
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("entry function '_start' must not contain tail_call_indirect terminators"),
            "unexpected lower8 error: {msg}"
        );
    }

    #[test]
    fn lower8_reject_module_without_start_export() {
        let module = IrModule::new(crate::module::ModuleInfo::default(), vec![]);
        let err = match lower8_module(&module, 65536) {
            Ok(_) => panic!("module without _start export should fail"),
            Err(err) => err,
        };
        let msg = format!("{err:#}");
        assert!(
            msg.contains("no '_start' export"),
            "unexpected lower8 error: {msg}"
        );
    }
}
