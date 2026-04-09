use std::fmt::Write;

use crate::ast::{AstRef, BinOp, Node, RelOp, UnOp};
use crate::ir::{BasicBlock, Inst, IrNode, Terminator};
use crate::ir8::{
    BoolNary8, BuiltinId, CallTarget, Inst8Kind, Ir8Program, PC_STRIDE, Pc, Terminator8, TrapCode,
    Val8, Word,
};
use crate::module::{AstFuncBody, AstModule, ConstInit, IrFuncBody, IrModule};

fn fmt_valtype(vt: &wasmparser::ValType) -> String {
    format!("{:?}", vt).to_lowercase()
}

fn fmt_sig(params: &[wasmparser::ValType], results: &[wasmparser::ValType]) -> String {
    let ps: Vec<_> = params.iter().map(fmt_valtype).collect();
    let rs: Vec<_> = results.iter().map(fmt_valtype).collect();
    format!("({}) -> ({})", ps.join(", "), rs.join(", "))
}

fn fmt_const_init(init: ConstInit) -> String {
    match init {
        ConstInit::I32(v) => format!("{}", v),
    }
}

pub fn print_module_ast(module: &AstModule) -> String {
    let mut out = String::new();
    for (idx, g) in module.globals().iter().enumerate() {
        writeln!(
            out,
            "global {} {} = {}",
            idx,
            fmt_valtype(&g.content_type()),
            fmt_const_init(g.init())
        )
        .unwrap();
    }
    if !module.globals().is_empty() {
        writeln!(out).unwrap();
    }
    for (idx, body) in module.bodies().iter().enumerate() {
        if let Some(body) = body {
            writeln!(
                out,
                "fn {} {}",
                idx,
                fmt_sig(
                    module.function_type_at(idx as u32).unwrap().params(),
                    module.function_type_at(idx as u32).unwrap().results()
                )
            )
            .unwrap();
            print_func_body(&mut out, body, 1);
            let more_functions_with_bodies =
                module.bodies()[idx + 1..].iter().any(|body| body.is_some());
            if more_functions_with_bodies {
                writeln!(out).unwrap();
            }
        }
    }
    out
}

pub fn print_module_ir(module: &IrModule) -> String {
    let mut out = String::new();
    for (idx, body) in module.bodies().iter().enumerate() {
        if let Some(body) = body {
            writeln!(
                out,
                "fn {} {}",
                idx,
                fmt_sig(
                    module.function_type_at(idx as u32).unwrap().params(),
                    module.function_type_at(idx as u32).unwrap().results()
                )
            )
            .unwrap();
            print_ir_func_body(&mut out, body, 1);
            let more_functions_with_bodies =
                module.bodies()[idx + 1..].iter().any(|body| body.is_some());
            if more_functions_with_bodies {
                writeln!(out).unwrap();
            }
        }
    }
    out
}

fn print_ir_func_body(out: &mut String, body: &IrFuncBody, indent: usize) {
    let pad = "  ".repeat(indent);
    if !body.locals().is_empty() {
        write!(out, "{}locals:", pad).unwrap();
        for (i, t) in body.locals().iter().enumerate() {
            write!(out, " l{}:{}", i, fmt_valtype(t)).unwrap();
        }
        writeln!(out).unwrap();
    }
    let mut ref_offset = IrNode::new(0);
    for bb in body.blocks() {
        let is_entry = bb.id == body.entry();
        print_ir_basic_block(out, bb, &pad, ref_offset, is_entry);
        ref_offset += bb.insts.len();
    }
}

fn print_ir_basic_block(
    out: &mut String,
    bb: &BasicBlock,
    pad: &str,
    ref_offset: IrNode,
    is_entry: bool,
) {
    if is_entry {
        writeln!(out, "{}$B{}: ; entry", pad, bb.id).unwrap();
    } else {
        writeln!(out, "{}$B{}:", pad, bb.id).unwrap();
    }
    let inner_pad = format!("{}  ", pad);
    for (i, inst) in bb.insts.iter().enumerate() {
        write!(out, "{}", inner_pad).unwrap();
        print_ir_inst(out, ref_offset + i, inst);
        writeln!(out).unwrap();
    }
    write!(out, "{}", inner_pad).unwrap();
    print_terminator(out, &bb.terminator);
    writeln!(out).unwrap();
}

fn ir_produces_value(inst: &Inst) -> bool {
    !matches!(
        inst,
        Inst::Drop
            | Inst::LocalSet(..)
            | Inst::GlobalSet(..)
            | Inst::Store { .. }
            | Inst::Putchar(_)
            | Inst::Getchar
    )
}

fn write_ir_operand(out: &mut String, v: IrNode) {
    if let Some(imm) = v.imm_i32_value() {
        write!(out, "{}", imm).unwrap();
    } else {
        write!(out, "%{}", v).unwrap();
    }
}

fn print_ir_inst(out: &mut String, r: IrNode, inst: &Inst) {
    if ir_produces_value(inst) {
        write!(out, "%{} = ", r).unwrap();
    }
    match inst {
        Inst::I32Const(n) => write!(out, "i32.const {}", n).unwrap(),
        Inst::LocalGet(local) => write!(out, "local.get {}", local).unwrap(),
        Inst::LocalTee(local, v) => {
            write!(out, "local.tee {} ", local).unwrap();
            write_ir_operand(out, *v);
        }
        Inst::GlobalGet(global) => write!(out, "global.get {}", global).unwrap(),
        Inst::MemorySize => write!(out, "memory.size").unwrap(),
        Inst::TableSize(table) => write!(out, "table.size {}", table).unwrap(),
        Inst::Unary(op, v) => {
            write!(out, "{} ", unop_name(op)).unwrap();
            write_ir_operand(out, *v);
        }
        Inst::Binary(op, l, r) => {
            write!(out, "{} ", binop_name(op)).unwrap();
            write_ir_operand(out, *l);
            write!(out, " ").unwrap();
            write_ir_operand(out, *r);
        }
        Inst::Compare(op, l, r) => {
            write!(out, "{} ", relop_name(op)).unwrap();
            write_ir_operand(out, *l);
            write!(out, " ").unwrap();
            write_ir_operand(out, *r);
        }
        Inst::Select {
            cond,
            if_true,
            if_false,
        } => {
            write!(out, "select ").unwrap();
            write_ir_operand(out, *cond);
            write!(out, " ").unwrap();
            write_ir_operand(out, *if_true);
            write!(out, " ").unwrap();
            write_ir_operand(out, *if_false);
        }
        Inst::Load {
            size,
            signed,
            offset,
            addr,
        } => {
            let sign = if *signed { "s" } else { "u" };
            write!(out, "load i32.{}b{} offset={:#x} ", size, sign, offset).unwrap();
            write_ir_operand(out, *addr);
        }
        Inst::Call { func, args } => {
            write!(out, "call {}", func).unwrap();
            for a in args {
                write!(out, " ").unwrap();
                write_ir_operand(out, *a);
            }
        }
        Inst::CallIndirect {
            type_index,
            table_index,
            index,
            args,
        } => {
            write!(
                out,
                "call_indirect type={} table={} ",
                type_index, table_index
            )
            .unwrap();
            write_ir_operand(out, *index);
            for a in args {
                write!(out, " ").unwrap();
                write_ir_operand(out, *a);
            }
        }
        Inst::Putchar(v) => {
            write!(out, "putchar ").unwrap();
            write_ir_operand(out, *v);
        }
        Inst::Getchar => write!(out, "getchar").unwrap(),
        Inst::Drop => write!(out, "drop").unwrap(),
        Inst::LocalSet(local, v) => {
            write!(out, "local.set {} ", local).unwrap();
            write_ir_operand(out, *v);
        }
        Inst::GlobalSet(global, v) => {
            write!(out, "global.set {} ", global).unwrap();
            write_ir_operand(out, *v);
        }
        Inst::Store {
            size,
            offset,
            addr,
            val,
        } => {
            write!(out, "store i32.{}b offset={:#x} ", size, offset).unwrap();
            write_ir_operand(out, *addr);
            write!(out, " ").unwrap();
            write_ir_operand(out, *val);
        }
    }
}

fn print_terminator(out: &mut String, term: &Terminator) {
    match term {
        Terminator::Goto(id) => write!(out, "goto $B{}", id).unwrap(),
        Terminator::Branch {
            cond,
            if_true,
            if_false,
        } => {
            write!(out, "branch ").unwrap();
            write_ir_operand(out, *cond);
            write!(out, " $B{} $B{}", if_true, if_false).unwrap();
        }
        Terminator::Switch {
            index,
            targets,
            default,
        } => {
            write!(out, "switch ").unwrap();
            write_ir_operand(out, *index);
            for (i, t) in targets.iter().enumerate() {
                write!(out, " {}:$B{}", i, t).unwrap();
            }
            write!(out, " default:$B{}", default).unwrap();
        }
        Terminator::TailCall { func, args } => {
            write!(out, "tail_call {}", func).unwrap();
            for a in args {
                write!(out, " ").unwrap();
                write_ir_operand(out, *a);
            }
        }
        Terminator::TailCallIndirect {
            type_index,
            table_index,
            index,
            args,
        } => {
            write!(
                out,
                "tail_call_indirect type={} table={} ",
                type_index, table_index
            )
            .unwrap();
            write_ir_operand(out, *index);
            for a in args {
                write!(out, " ").unwrap();
                write_ir_operand(out, *a);
            }
        }
        Terminator::Return(Some(r)) => {
            write!(out, "return ").unwrap();
            write_ir_operand(out, *r);
        }
        Terminator::Return(None) => write!(out, "return").unwrap(),
        Terminator::Unreachable => write!(out, "unreachable").unwrap(),
    }
}

fn print_func_body(out: &mut String, body: &AstFuncBody, indent: usize) {
    let pad = "  ".repeat(indent);
    if !body.locals().is_empty() {
        write!(out, "{}locals:", pad).unwrap();
        for (i, t) in body.locals().iter().enumerate() {
            write!(out, " l{}:{}", i, fmt_valtype(t)).unwrap();
        }
        writeln!(out).unwrap();
    }
    print_insts(out, body.insts(), indent);
}

fn print_insts(out: &mut String, insts: &[Node], indent: usize) {
    let pad = "  ".repeat(indent);
    for (i, inst) in insts.iter().enumerate() {
        write!(out, "{}", pad).unwrap();
        print_inst(out, AstRef::new(i), inst, indent);
        writeln!(out).unwrap();
    }
}

fn print_inst(out: &mut String, idx: AstRef, inst: &Node, indent: usize) {
    match inst {
        Node::I32Const(n) => write!(out, "%{} = i32.const {}", idx, n).unwrap(),
        Node::LocalGet(local) => write!(out, "%{} = local.get {}", idx, local).unwrap(),
        Node::LocalTee(local, r) => write!(out, "%{} = local.tee {} %{}", idx, local, r).unwrap(),
        Node::GlobalGet(global) => write!(out, "%{} = global.get {}", idx, global).unwrap(),
        Node::MemorySize => write!(out, "%{} = memory.size", idx).unwrap(),
        Node::TableSize(table) => write!(out, "%{} = table.size {}", idx, table).unwrap(),
        Node::Unary(op, r) => write!(out, "%{} = {} %{}", idx, unop_name(op), r).unwrap(),
        Node::Binary(op, l, r) => {
            write!(out, "%{} = {} %{} %{}", idx, binop_name(op), l, r).unwrap()
        }
        Node::Compare(op, l, r) => {
            write!(out, "%{} = {} %{} %{}", idx, relop_name(op), l, r).unwrap()
        }
        Node::Select {
            cond,
            then_val,
            else_val,
        } => write!(
            out,
            "%{} = select %{} %{} %{}",
            idx, cond, then_val, else_val
        )
        .unwrap(),
        Node::Load {
            size,
            signed,
            offset,
            address,
        } => {
            let sign = if *signed { "s" } else { "u" };
            write!(
                out,
                "%{} = load i32 {}b{} offset={:#x} %{}",
                idx, size, sign, offset, address
            )
            .unwrap();
        }
        Node::Call(func, args) => {
            write!(out, "%{} = call {}", idx, func).unwrap();
            for a in args {
                write!(out, " %{}", a).unwrap();
            }
        }
        Node::CallIndirect {
            type_index,
            table_index,
            index,
            args,
        } => {
            write!(
                out,
                "%{} = call_indirect type={} table={} %{}",
                idx, type_index, table_index, index
            )
            .unwrap();
            for a in args {
                write!(out, " %{}", a).unwrap();
            }
        }
        Node::Drop(r) => write!(out, "drop %{}", r).unwrap(),
        Node::LocalSet(local, r) => write!(out, "local.set {} %{}", local, r).unwrap(),
        Node::GlobalSet(global, r) => write!(out, "global.set {} %{}", global, r).unwrap(),
        Node::Store {
            size,
            offset,
            value,
            address,
        } => {
            write!(
                out,
                "store i32 {}b offset={:#x} %{} %{}",
                size, offset, address, value
            )
            .unwrap();
        }
        Node::Block(insts) => {
            writeln!(out, "block").unwrap();
            print_insts(out, insts, indent + 1);
            write!(out, "{}end block", "  ".repeat(indent)).unwrap();
        }
        Node::Loop(insts) => {
            writeln!(out, "loop").unwrap();
            print_insts(out, insts, indent + 1);
            write!(out, "{}end loop", "  ".repeat(indent)).unwrap();
        }
        Node::If {
            cond,
            then_body,
            else_body,
        } => {
            writeln!(out, "if %{}", cond).unwrap();
            print_insts(out, then_body, indent + 1);
            if !else_body.is_empty() {
                writeln!(out, "{}else", "  ".repeat(indent)).unwrap();
                print_insts(out, else_body, indent + 1);
            }
            write!(out, "{}end if", "  ".repeat(indent)).unwrap();
        }
        Node::Br(depth) => write!(out, "br {}", depth).unwrap(),
        Node::BrIf(depth, r) => write!(out, "br_if {} %{}", depth, r).unwrap(),
        Node::BrTable(targets, default, r) => write!(
            out,
            "br_table [{}] default={} %{}",
            targets.len(),
            default,
            r
        )
        .unwrap(),
        Node::Return(Some(r)) => write!(out, "return %{}", r).unwrap(),
        Node::Return(None) => write!(out, "return").unwrap(),
        Node::Unreachable => write!(out, "unreachable").unwrap(),
    }
}

fn binop_name(op: &BinOp) -> String {
    let op_str = match op {
        BinOp::Add => "add",
        BinOp::Sub => "sub",
        BinOp::Mul => "mul",
        BinOp::DivS => "div_s",
        BinOp::DivU => "div_u",
        BinOp::RemS => "rem_s",
        BinOp::RemU => "rem_u",
        BinOp::And => "and",
        BinOp::Or => "or",
        BinOp::Xor => "xor",
        BinOp::Shl => "shl",
        BinOp::ShrS => "shr_s",
        BinOp::ShrU => "shr_u",
        BinOp::Rotl => "rotl",
        BinOp::Rotr => "rotr",
    };
    format!("{}.{}", "i32", op_str)
}

fn relop_name(op: &RelOp) -> String {
    let op_str = match op {
        RelOp::Eq => "eq",
        RelOp::Ne => "ne",
        RelOp::LtS => "lt_s",
        RelOp::LtU => "lt_u",
        RelOp::GtS => "gt_s",
        RelOp::GtU => "gt_u",
        RelOp::LeS => "le_s",
        RelOp::LeU => "le_u",
        RelOp::GeS => "ge_s",
        RelOp::GeU => "ge_u",
    };
    format!("{}.{}", "i32", op_str)
}

fn unop_name(op: &UnOp) -> String {
    let op_str = match op {
        UnOp::Clz => "clz",
        UnOp::Ctz => "ctz",
        UnOp::Popcnt => "popcnt",
        UnOp::Eqz => "eqz",
        UnOp::Extend8S => "extend8_s",
        UnOp::Extend16S => "extend16_s",
    };
    format!("{}.{}", "i32", op_str)
}

// ─── IR8 printer ─────────────────────────────────────────────────────────────

pub fn print_ir8_program(prog: &Ir8Program) -> String {
    let mut out = String::new();
    writeln!(out, "entry: fn {}", prog.entry_func).unwrap();
    writeln!(out, "num_vregs: {}", prog.num_vregs).unwrap();
    for (func_id, blocks) in prog.func_blocks.iter().enumerate() {
        if blocks.is_empty() {
            continue;
        }
        let num_locals = prog.func_num_locals[func_id];
        writeln!(out, "\nfn {} [{} locals]", func_id, num_locals).unwrap();
        for bb in blocks {
            writeln!(out, "  $B{}:", bb.id.index()).unwrap();
            for inst in &bb.insts {
                write!(out, "    ").unwrap();
                print_inst8(&mut out, inst);
                writeln!(out).unwrap();
            }
            write!(out, "    ").unwrap();
            print_term8(&mut out, &bb.terminator);
            writeln!(out).unwrap();
        }
    }
    out
}

pub fn print_program(prog: &Ir8Program) -> String {
    let mut out = String::new();
    let entry = Pc::new(prog.entry_func as u16 * PC_STRIDE);
    writeln!(out, "entry: $B{}", entry.index()).unwrap();
    writeln!(out, "num_regs: {}", prog.num_vregs).unwrap();
    writeln!(out, "cycles: {}", prog.cycles.len()).unwrap();
    for cycle in &prog.cycles {
        writeln!(out, "\n  $B{}:", cycle.pc.index()).unwrap();
        for inst in &cycle.ops {
            write!(out, "    ").unwrap();
            print_inst8(&mut out, inst);
            writeln!(out).unwrap();
        }
        write!(out, "    ").unwrap();
        print_term8(&mut out, &cycle.terminator);
        writeln!(out).unwrap();
    }
    out
}

fn fmt_val8(r: Val8) -> String {
    match r {
        Val8::VReg(v) => format!("%r{}", v),
        Val8::Imm(i) => format!("{:#04x}", i),
    }
}

fn fmt_word(w: Word) -> String {
    format!(
        "({}:{}:{}:{})",
        fmt_val8(w.b0),
        fmt_val8(w.b1),
        fmt_val8(w.b2),
        fmt_val8(w.b3)
    )
}

fn fmt_pc(pc: Pc) -> String {
    format!("$B{}", pc.index())
}

fn fmt_call_target(target: CallTarget) -> String {
    match target {
        CallTarget::Pc(pc) => fmt_pc(pc),
        CallTarget::Builtin(builtin) => fmt_builtin(builtin).to_string(),
    }
}

fn fmt_builtin(builtin: BuiltinId) -> &'static str {
    builtin.name()
}

fn fmt_bool_nary(op: &BoolNary8) -> String {
    op.as_slice()
        .iter()
        .map(|r| fmt_val8(*r))
        .collect::<Vec<_>>()
        .join(" ")
}

fn print_inst8(out: &mut String, inst: &crate::ir8::Inst8) {
    if let Some(dst) = inst.dst {
        write!(out, "{} = ", fmt_val8(dst)).unwrap();
    }
    match &inst.kind {
        Inst8Kind::Copy(s) => write!(out, "copy {}", fmt_val8(*s)).unwrap(),

        Inst8Kind::Add32Byte { lhs, rhs, lane } => {
            write!(out, "add32.b{} {} {}", lane, fmt_word(*lhs), fmt_word(*rhs)).unwrap()
        }
        Inst8Kind::Sub32Byte { lhs, rhs, lane } => {
            write!(out, "sub32.b{} {} {}", lane, fmt_word(*lhs), fmt_word(*rhs)).unwrap()
        }
        Inst8Kind::Sub32Borrow { lhs, rhs } => {
            write!(out, "sub32.borrow {} {}", fmt_word(*lhs), fmt_word(*rhs)).unwrap()
        }

        Inst8Kind::Add(l, r) => write!(out, "add {} {}", fmt_val8(*l), fmt_val8(*r)).unwrap(),
        Inst8Kind::Carry(l, r) => write!(out, "carry {} {}", fmt_val8(*l), fmt_val8(*r)).unwrap(),
        Inst8Kind::Sub(l, r) => write!(out, "sub {} {}", fmt_val8(*l), fmt_val8(*r)).unwrap(),

        Inst8Kind::MulLo(l, r) => write!(out, "mul.lo {} {}", fmt_val8(*l), fmt_val8(*r)).unwrap(),
        Inst8Kind::MulHi(l, r) => write!(out, "mul.hi {} {}", fmt_val8(*l), fmt_val8(*r)).unwrap(),

        Inst8Kind::And8(l, r) => write!(out, "and {} {}", fmt_val8(*l), fmt_val8(*r)).unwrap(),
        Inst8Kind::Or8(l, r) => write!(out, "or  {} {}", fmt_val8(*l), fmt_val8(*r)).unwrap(),
        Inst8Kind::Xor8(l, r) => write!(out, "xor {} {}", fmt_val8(*l), fmt_val8(*r)).unwrap(),

        Inst8Kind::Eq(l, r) => write!(out, "eq  {} {}", fmt_val8(*l), fmt_val8(*r)).unwrap(),
        Inst8Kind::Ne(l, r) => write!(out, "ne  {} {}", fmt_val8(*l), fmt_val8(*r)).unwrap(),
        Inst8Kind::LtU(l, r) => write!(out, "lt_u {} {}", fmt_val8(*l), fmt_val8(*r)).unwrap(),
        Inst8Kind::GeU(l, r) => write!(out, "ge_u {} {}", fmt_val8(*l), fmt_val8(*r)).unwrap(),

        Inst8Kind::BoolAnd(op) => write!(out, "bool.and {}", fmt_bool_nary(op)).unwrap(),
        Inst8Kind::BoolOr(op) => write!(out, "bool.or  {}", fmt_bool_nary(op)).unwrap(),
        Inst8Kind::BoolNot(v) => write!(out, "bool.not {}", fmt_val8(*v)).unwrap(),

        Inst8Kind::Sel(c, t, f) => write!(
            out,
            "sel {} {} {}",
            fmt_val8(*c),
            fmt_val8(*t),
            fmt_val8(*f)
        )
        .unwrap(),

        Inst8Kind::GlobalGetByte { global_idx, lane } => {
            write!(out, "global.get g{}[{}]", global_idx, lane).unwrap()
        }
        Inst8Kind::GlobalSetByte {
            global_idx,
            lane,
            val,
        } => write!(
            out,
            "global.set g{}[{}] {}",
            global_idx,
            lane,
            fmt_val8(*val)
        )
        .unwrap(),

        Inst8Kind::LoadMem { base, addr, lane } => write!(
            out,
            "load.mem [{:#x}+{}:{}] lane={}",
            base,
            fmt_val8(addr.lo),
            fmt_val8(addr.hi),
            lane
        )
        .unwrap(),
        Inst8Kind::StoreMem {
            base,
            addr,
            lane,
            val,
        } => write!(
            out,
            "store.mem [{:#x}+{}:{}] lane={} {}",
            base,
            fmt_val8(addr.lo),
            fmt_val8(addr.hi),
            lane,
            fmt_val8(*val)
        )
        .unwrap(),

        Inst8Kind::Getchar => write!(out, "getchar").unwrap(),
        Inst8Kind::Putchar(v) => write!(out, "putchar {}", fmt_val8(*v)).unwrap(),

        Inst8Kind::CsStore { offset, val } => {
            write!(out, "cs.store [cs_sp+{}] {}", offset, fmt_val8(*val)).unwrap()
        }
        Inst8Kind::CsLoad { offset } => write!(out, "cs.load [cs_sp+{}]", offset).unwrap(),
        Inst8Kind::CsStorePc { offset, val } => {
            write!(out, "cs.store_pc [cs_sp+{}] {}", offset, fmt_pc(*val)).unwrap()
        }
        Inst8Kind::CsLoadPc { offset } => write!(out, "cs.load_pc [cs_sp+{}]", offset).unwrap(),
        Inst8Kind::CsAlloc(size) => write!(out, "cs.alloc {}", size).unwrap(),
        Inst8Kind::CsFree(size) => write!(out, "cs.free {}", size).unwrap(),
    }
}

fn print_term8(out: &mut String, term: &Terminator8) {
    match term {
        Terminator8::Goto(pc) => write!(out, "goto {}", fmt_pc(*pc)).unwrap(),
        Terminator8::Branch {
            cond,
            if_true,
            if_false,
        } => write!(
            out,
            "branch {} {} {}",
            fmt_val8(*cond),
            fmt_pc(*if_true),
            fmt_pc(*if_false)
        )
        .unwrap(),
        Terminator8::Switch {
            index,
            targets,
            default,
        } => {
            write!(out, "switch {}", fmt_val8(*index)).unwrap();
            for (i, t) in targets.iter().enumerate() {
                write!(out, " {}:{}", i, fmt_pc(*t)).unwrap();
            }
            write!(out, " default:{}", fmt_pc(*default)).unwrap();
        }
        Terminator8::Return { val: Some(w) } => write!(out, "return {}", fmt_word(*w)).unwrap(),
        Terminator8::Return { val: None } => write!(out, "return").unwrap(),
        Terminator8::Exit { val: Some(w) } => write!(out, "exit {}", fmt_word(*w)).unwrap(),
        Terminator8::Exit { val: None } => write!(out, "exit").unwrap(),
        Terminator8::CallSetup {
            callee_entry,
            cont,
            args,
            callee_arg_vregs,
        } => {
            write!(
                out,
                "call_setup {} cont={}",
                fmt_call_target(*callee_entry),
                fmt_pc(*cont)
            )
            .unwrap();
            write!(out, " args=[").unwrap();
            for (i, a) in args.iter().enumerate() {
                if i > 0 {
                    write!(out, ", ").unwrap();
                }
                write!(out, "{}", fmt_word(*a)).unwrap();
            }
            write!(out, "] -> [").unwrap();
            for (i, p) in callee_arg_vregs.iter().enumerate() {
                if i > 0 {
                    write!(out, ", ").unwrap();
                }
                write!(out, "{}", fmt_word(*p)).unwrap();
            }
            write!(out, "]").unwrap();
        }
        Terminator8::Trap(code) => {
            let name = match code {
                TrapCode::CallstackOverflow => "trap callstack_overflow",
                TrapCode::Exited => "trap exited",
                TrapCode::Unreachable => "trap unreachable",
                TrapCode::InvalidMemoryAccess => "trap invalid_memory_access",
                TrapCode::DivisionByZero => "trap division_by_zero",
            };
            write!(out, "trap {}", name).unwrap();
        }
    }
}
