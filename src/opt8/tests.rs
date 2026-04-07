use super::{
    copy_elim, dead_code_elim, instcombine::instcombine, local_dead_mem_store_elim, run,
    thread_empty_gotos_func,
};
use crate::ir8::{
    Addr, BUILTIN_REM_U32, BUILTIN_SHL_32, BasicBlock8, BoolNary8, FrameInfo, Inst8, Inst8Kind,
    Ir8Program, MemoryLayout, Pc, Terminator8, Val8, Word,
};

fn r(i: u16) -> Val8 {
    Val8::vreg(i)
}

fn bool_or(regs: &[Val8]) -> Inst8Kind {
    Inst8Kind::BoolOr(BoolNary8::from_regs(regs).unwrap())
}

fn mk_prog(blocks: Vec<BasicBlock8>) -> Ir8Program {
    Ir8Program {
        entry_func: 0,
        num_vregs: 64,
        func_blocks: vec![blocks],
        cycles: Vec::new(),
        frame_infos: vec![FrameInfo {
            entry: Pc::new(0),
            num_locals: 0,
        }],
        memory_layout: MemoryLayout {
            memory_end: 0,
            init_bytes: Vec::new(),
        },
        global_init: Vec::new(),
    }
}

#[test]
fn opt8_dce_keeps_cs_load_pc_before_return() {
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::no_dst(Inst8Kind::CsFree(1)),
            Inst8::no_dst(Inst8Kind::CsLoadPc { offset: 0 }),
        ],
        terminator: Terminator8::Return { val: None },
    }]);

    let _ = dead_code_elim(&mut prog);
    let insts = &prog.func_blocks[0][0].insts;
    assert!(
        insts
            .iter()
            .any(|i| matches!(i.kind, Inst8Kind::CsLoadPc { .. }))
    );
}

#[test]
fn opt8_dce_drops_unused_add32_byte_lanes() {
    let lhs = Word::new(r(0), r(1), r(2), r(3));
    let rhs = Word::new(r(4), r(5), r(6), r(7));
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::with_dst(r(20), Inst8Kind::Add32Byte { lhs, rhs, lane: 0 }),
            Inst8::with_dst(r(21), Inst8Kind::Add32Byte { lhs, rhs, lane: 1 }),
            Inst8::with_dst(r(22), Inst8Kind::Add32Byte { lhs, rhs, lane: 2 }),
            Inst8::with_dst(r(23), Inst8Kind::Add32Byte { lhs, rhs, lane: 3 }),
        ],
        terminator: Terminator8::Exit {
            val: Some(Word::new(r(23), Val8::imm(0), Val8::imm(0), Val8::imm(0))),
        },
    }]);

    assert!(dead_code_elim(&mut prog));
    let insts = &prog.func_blocks[0][0].insts;
    assert_eq!(insts.len(), 1);
    assert!(matches!(
        insts[0].kind,
        Inst8Kind::Add32Byte { lane: 3, .. }
    ));
}

#[test]
fn opt8_local_dead_mem_store_elim_removes_overwritten_same_byte_store() {
    let addr = Addr::new(r(10), r(11));
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 0,
                val: r(20),
            }),
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 0,
                val: r(21),
            }),
        ],
        terminator: Terminator8::Exit { val: None },
    }]);

    assert!(local_dead_mem_store_elim(&mut prog));
    let insts = &prog.func_blocks[0][0].insts;
    assert_eq!(insts.len(), 1);
    let Inst8Kind::StoreMem { val, .. } = insts[0].kind else {
        panic!("expected store.mem");
    };
    assert_eq!(val, r(21));
}

#[test]
fn opt8_local_dead_mem_store_elim_keeps_store_when_intervening_load_may_observe() {
    let addr = Addr::new(r(10), r(11));
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 0,
                val: r(20),
            }),
            Inst8::with_dst(
                r(30),
                Inst8Kind::LoadMem {
                    base: 0,
                    addr,
                    lane: 0,
                },
            ),
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 0,
                val: r(21),
            }),
        ],
        terminator: Terminator8::Exit { val: None },
    }]);

    assert!(!local_dead_mem_store_elim(&mut prog));
    assert_eq!(prog.func_blocks[0][0].insts.len(), 3);
}

#[test]
fn opt8_local_dead_mem_store_elim_keeps_store_across_addr_reg_redefinition() {
    let addr = Addr::new(r(10), r(11));
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 0,
                val: r(20),
            }),
            Inst8::with_dst(r(10), Inst8Kind::Copy(Val8::imm(7))),
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 0,
                val: r(21),
            }),
        ],
        terminator: Terminator8::Exit { val: None },
    }]);

    assert!(!local_dead_mem_store_elim(&mut prog));
    assert_eq!(prog.func_blocks[0][0].insts.len(), 3);
}

#[test]
fn opt8_local_dead_mem_store_elim_matches_effective_byte_offset_base_plus_lane() {
    let addr = Addr::new(r(10), r(11));
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 1,
                addr,
                lane: 0,
                val: r(20),
            }),
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 1,
                val: r(21),
            }),
        ],
        terminator: Terminator8::Exit { val: None },
    }]);

    assert!(
        local_dead_mem_store_elim(&mut prog),
        "second store overwrites the same effective byte (base + lane)"
    );
    let insts = &prog.func_blocks[0][0].insts;
    assert_eq!(insts.len(), 1);
    let Inst8Kind::StoreMem {
        base, lane, val, ..
    } = insts[0].kind
    else {
        panic!("expected remaining store.mem");
    };
    assert_eq!(base, 0);
    assert_eq!(lane, 1);
    assert_eq!(val, r(21));
}

#[test]
fn opt8_local_dead_mem_store_elim_keeps_store_for_different_effective_offsets() {
    let addr = Addr::new(r(10), r(11));
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 1,
                addr,
                lane: 0,
                val: r(20),
            }),
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 0,
                val: r(21),
            }),
        ],
        terminator: Terminator8::Exit { val: None },
    }]);

    assert!(!local_dead_mem_store_elim(&mut prog));
    assert_eq!(prog.func_blocks[0][0].insts.len(), 2);
}

#[test]
fn opt8_copy_elim_rewrites_cross_block_copy_uses_when_source_is_stable() {
    let mut prog = mk_prog(vec![
        BasicBlock8 {
            id: Pc::new(0),
            insts: vec![
                Inst8::with_dst(r(4), Inst8Kind::Copy(Val8::imm(7))),
                Inst8::with_dst(r(8), Inst8Kind::Copy(r(4))),
            ],
            terminator: Terminator8::Goto(Pc::new(1)),
        },
        BasicBlock8 {
            id: Pc::new(1),
            insts: vec![],
            terminator: Terminator8::Exit {
                val: Some(Word::new(r(8), r(4), r(4), r(4))),
            },
        },
    ]);

    let _ = copy_elim(&mut prog);

    let b0 = &prog.func_blocks[0][0];
    assert!(
        b0.insts.iter().all(|i| i.dst != Some(r(8))),
        "copy result register should be eliminated"
    );

    let Terminator8::Exit { val: Some(w) } = &prog.func_blocks[0][1].terminator else {
        panic!("expected exit value");
    };
    assert_eq!(w.b0, Val8::imm(7));
}

#[test]
fn opt8_copy_elim_pools_repeated_constants() {
    let mut prog = mk_prog(vec![
        BasicBlock8 {
            id: Pc::new(0),
            insts: vec![Inst8::with_dst(r(4), Inst8Kind::Copy(Val8::imm(0xff)))],
            terminator: Terminator8::Goto(Pc::new(1)),
        },
        BasicBlock8 {
            id: Pc::new(1),
            insts: vec![Inst8::with_dst(r(8), Inst8Kind::Copy(Val8::imm(0xff)))],
            terminator: Terminator8::Exit {
                val: Some(Word::new(r(4), r(8), r(4), r(8))),
            },
        },
    ]);

    let _ = copy_elim(&mut prog);

    let count_ff = prog.func_blocks[0]
        .iter()
        .flat_map(|bb| bb.insts.iter())
        .filter(|inst| matches!(inst.kind, Inst8Kind::Copy(src) if src == Val8::imm(0xff)))
        .count();
    assert!(count_ff <= 1);
}

#[test]
fn opt8_coalesces_linear_goto_chain() {
    let mut prog = mk_prog(vec![
        BasicBlock8 {
            id: Pc::new(0),
            insts: vec![Inst8::with_dst(r(4), Inst8Kind::Copy(Val8::imm(1)))],
            terminator: Terminator8::Goto(Pc::new(1)),
        },
        BasicBlock8 {
            id: Pc::new(1),
            insts: vec![Inst8::with_dst(r(8), Inst8Kind::Copy(r(4)))],
            terminator: Terminator8::Goto(Pc::new(2)),
        },
        BasicBlock8 {
            id: Pc::new(2),
            insts: vec![Inst8::with_dst(r(12), Inst8Kind::Copy(r(8)))],
            terminator: Terminator8::Goto(Pc::new(3)),
        },
        BasicBlock8 {
            id: Pc::new(3),
            insts: vec![],
            terminator: Terminator8::Exit {
                val: Some(Word::new(r(12), r(4), r(4), r(4))),
            },
        },
    ]);

    run(&mut prog);
    assert_eq!(prog.func_blocks[0].len(), 1);
}

#[test]
fn opt8_threads_empty_goto_blocks() {
    let mut prog = mk_prog(vec![
        BasicBlock8 {
            id: Pc::new(0),
            insts: vec![Inst8::with_dst(r(4), Inst8Kind::Copy(Val8::imm(1)))],
            terminator: Terminator8::Branch {
                cond: r(4),
                if_true: Pc::new(1),
                if_false: Pc::new(2),
            },
        },
        BasicBlock8 {
            id: Pc::new(1),
            insts: vec![],
            terminator: Terminator8::Goto(Pc::new(3)),
        },
        BasicBlock8 {
            id: Pc::new(2),
            insts: vec![],
            terminator: Terminator8::Goto(Pc::new(3)),
        },
        BasicBlock8 {
            id: Pc::new(3),
            insts: vec![],
            terminator: Terminator8::Exit { val: None },
        },
    ]);

    run(&mut prog);

    assert!(
        prog.func_blocks[0]
            .iter()
            .all(|b| b.id != Pc::new(1) && b.id != Pc::new(2)),
        "threaded empty goto blocks should be removed"
    );

    let entry = prog.func_blocks[0]
        .iter()
        .find(|b| b.id == Pc::new(0))
        .expect("entry block should exist");
    let succ = entry.terminator.successors();
    assert!(
        succ.iter().all(|&pc| pc != Pc::new(1) && pc != Pc::new(2)),
        "entry should no longer target empty goto blocks"
    );
}

#[test]
fn opt8_thread_empty_gotos_rewrites_callsetup_callee_and_cont_targets() {
    let mut blocks = vec![
        BasicBlock8 {
            id: Pc::new(0),
            insts: vec![],
            terminator: Terminator8::CallSetup {
                callee_entry: Pc::new(1),
                cont: Pc::new(2),
                args: vec![],
                callee_arg_vregs: vec![],
            },
        },
        BasicBlock8 {
            id: Pc::new(1),
            insts: vec![],
            terminator: Terminator8::Goto(Pc::new(3)),
        },
        BasicBlock8 {
            id: Pc::new(2),
            insts: vec![],
            terminator: Terminator8::Goto(Pc::new(4)),
        },
        BasicBlock8 {
            id: Pc::new(3),
            insts: vec![],
            terminator: Terminator8::Exit { val: None },
        },
        BasicBlock8 {
            id: Pc::new(4),
            insts: vec![],
            terminator: Terminator8::Exit { val: None },
        },
    ];

    assert!(thread_empty_gotos_func(&mut blocks, Some(Pc::new(0))));
    let Terminator8::CallSetup {
        callee_entry, cont, ..
    } = blocks[0].terminator
    else {
        panic!("expected entry callsetup");
    };
    assert_eq!(callee_entry, Pc::new(3));
    assert_eq!(cont, Pc::new(4));
}

#[test]
fn opt8_prefers_arch_register_after_copy() {
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::with_dst(r(20), Inst8Kind::Add(r(8), r(9))),
            Inst8::with_dst(r(6), Inst8Kind::Copy(r(20))),
            Inst8::with_dst(r(30), Inst8Kind::Ne(r(20), r(4))),
        ],
        terminator: Terminator8::Exit {
            val: Some(Word::new(r(6), r(30), r(4), r(4))),
        },
    }]);

    run(&mut prog);
    let bb = &prog.func_blocks[0][0];
    let ne_inst = bb
        .insts
        .iter()
        .find(|i| i.dst == Some(r(30)))
        .expect("ne inst should exist");
    let Inst8Kind::Ne(lhs, _) = ne_inst.kind else {
        panic!("expected ne");
    };
    assert_eq!(lhs, r(6));
}

#[test]
fn opt8_does_not_prefer_arbitrary_lower_temp_after_copy() {
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::with_dst(r(20), Inst8Kind::Add(r(8), r(9))),
            Inst8::with_dst(r(16), Inst8Kind::Copy(r(20))),
            Inst8::with_dst(r(30), Inst8Kind::Ne(r(20), r(4))),
        ],
        terminator: Terminator8::Exit {
            val: Some(Word::new(r(16), r(30), r(4), r(4))),
        },
    }]);

    run(&mut prog);
    let bb = &prog.func_blocks[0][0];
    let ne_inst = bb
        .insts
        .iter()
        .find(|i| i.dst == Some(r(30)))
        .expect("ne inst should exist");
    let Inst8Kind::Ne(lhs, _) = ne_inst.kind else {
        panic!("expected ne");
    };
    assert_eq!(lhs, r(20));
}

#[test]
fn opt8_combines_bool_or_tree_into_nary_bool_or() {
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::with_dst(r(10), bool_or(&[r(1), r(2)])),
            Inst8::with_dst(r(11), bool_or(&[r(3), r(4)])),
            Inst8::with_dst(r(12), bool_or(&[r(10), r(11)])),
        ],
        terminator: Terminator8::Branch {
            cond: r(12),
            if_true: Pc::new(1),
            if_false: Pc::new(2),
        },
    }]);

    run(&mut prog);
    let bb = &prog.func_blocks[0][0];

    let root = bb
        .insts
        .iter()
        .find(|i| i.dst == Some(r(12)))
        .expect("root bool.or should still exist");
    let Inst8Kind::BoolOr(op) = root.kind else {
        panic!("expected nary bool.or");
    };
    assert_eq!(op.as_slice(), &[r(1), r(2), r(3), r(4)]);
    assert!(
        bb.insts
            .iter()
            .all(|i| i.dst != Some(r(10)) && i.dst != Some(r(11))),
        "intermediate bool.or defs should be removed"
    );
}

#[test]
fn opt8_combines_large_bool_or_tree_into_single_nary_op() {
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::with_dst(r(10), bool_or(&[r(1), r(2)])),
            Inst8::with_dst(r(11), bool_or(&[r(3), r(4)])),
            Inst8::with_dst(r(12), bool_or(&[r(5), r(6)])),
            Inst8::with_dst(r(13), bool_or(&[r(10), r(11)])),
            Inst8::with_dst(r(14), bool_or(&[r(13), r(12)])),
        ],
        terminator: Terminator8::Branch {
            cond: r(14),
            if_true: Pc::new(1),
            if_false: Pc::new(2),
        },
    }]);

    run(&mut prog);
    let bb = &prog.func_blocks[0][0];

    let root = bb
        .insts
        .iter()
        .find(|i| i.dst == Some(r(14)))
        .expect("root bool.or should still exist");
    let Inst8Kind::BoolOr(op) = root.kind else {
        panic!("expected nary bool.or");
    };
    assert_eq!(op.as_slice(), &[r(1), r(2), r(3), r(4), r(5), r(6)]);
    assert!(
        bb.insts
            .iter()
            .all(|i| i.dst != Some(r(10)) && i.dst != Some(r(11)) && i.dst != Some(r(13))),
        "intermediate bool.or defs should be removed"
    );
}

#[test]
fn opt8_combines_ne_bool_or_tree_into_nary_bool_or() {
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::with_dst(r(10), Inst8Kind::Ne(r(1), r(2))),
            Inst8::with_dst(r(11), Inst8Kind::Ne(r(3), r(4))),
            Inst8::with_dst(r(12), Inst8Kind::Ne(r(5), r(6))),
            Inst8::with_dst(r(13), Inst8Kind::Ne(r(7), r(8))),
            Inst8::with_dst(r(15), bool_or(&[r(10), r(11)])),
            Inst8::with_dst(r(16), bool_or(&[r(12), r(13)])),
            Inst8::with_dst(r(14), bool_or(&[r(15), r(16)])),
        ],
        terminator: Terminator8::Branch {
            cond: r(14),
            if_true: Pc::new(1),
            if_false: Pc::new(2),
        },
    }]);

    run(&mut prog);
    let bb = &prog.func_blocks[0][0];

    let root = bb
        .insts
        .iter()
        .find(|i| i.dst == Some(r(14)))
        .expect("root logical result should still exist");
    let Inst8Kind::BoolOr(op) = root.kind else {
        panic!("expected nary bool.or");
    };
    assert_eq!(op.as_slice(), &[r(10), r(11), r(12), r(13)]);
    let ne_count = bb
        .insts
        .iter()
        .filter(|i| matches!(i.kind, Inst8Kind::Ne(_, _)))
        .count();
    assert_eq!(ne_count, 4, "compare leaves should stay explicit defs");
}

#[test]
fn opt8_pools_repeated_consts_even_with_redefined_dests() {
    let mut prog = mk_prog(vec![
        BasicBlock8 {
            id: Pc::new(0),
            insts: vec![
                Inst8::with_dst(r(20), Inst8Kind::Copy(Val8::imm(0))),
                Inst8::with_dst(r(21), Inst8Kind::Copy(Val8::imm(0))),
            ],
            terminator: Terminator8::Goto(Pc::new(1)),
        },
        BasicBlock8 {
            id: Pc::new(1),
            insts: vec![
                Inst8::with_dst(r(20), Inst8Kind::Copy(Val8::imm(0))),
                Inst8::with_dst(r(22), Inst8Kind::Copy(r(20))),
            ],
            terminator: Terminator8::Exit {
                val: Some(Word::new(r(20), r(21), r(22), r(20))),
            },
        },
    ]);

    run(&mut prog);
    let const_zero_count = prog.func_blocks[0]
        .iter()
        .flat_map(|bb| bb.insts.iter())
        .filter(|inst| matches!(inst.kind, Inst8Kind::Copy(src) if src == Val8::imm(0)))
        .count();
    assert!(
        const_zero_count <= 1,
        "repeated const zero materializations should pool to one canonical const"
    );
}

#[test]
fn opt8_simplifies_false_compare_checks_inside_nary_bool_or() {
    let mut prog = mk_prog(vec![
        BasicBlock8 {
            id: Pc::new(0),
            insts: vec![
                Inst8::with_dst(r(2), Inst8Kind::Copy(Val8::imm(0))),
                Inst8::with_dst(r(3), Inst8Kind::Ne(r(10), r(11))),
                Inst8::with_dst(r(20), Inst8Kind::Ne(r(3), r(2))),
                Inst8::with_dst(r(21), Inst8Kind::Ne(r(2), r(2))),
                Inst8::with_dst(r(22), Inst8Kind::Ne(r(2), r(2))),
                Inst8::with_dst(r(23), Inst8Kind::Ne(r(2), r(2))),
                Inst8::with_dst(r(4), bool_or(&[r(20), r(21), r(22), r(23)])),
            ],
            terminator: Terminator8::Branch {
                cond: r(4),
                if_true: Pc::new(1),
                if_false: Pc::new(2),
            },
        },
        BasicBlock8 {
            id: Pc::new(1),
            insts: vec![],
            terminator: Terminator8::Exit { val: None },
        },
        BasicBlock8 {
            id: Pc::new(2),
            insts: vec![],
            terminator: Terminator8::Exit { val: None },
        },
    ]);

    run(&mut prog);

    let entry = prog.func_blocks[0]
        .iter()
        .find(|b| b.id == Pc::new(0))
        .expect("entry block should remain");
    let Terminator8::Branch { cond, .. } = entry.terminator else {
        panic!("entry should still branch");
    };
    if cond == r(3) {
        return;
    }
    assert_eq!(
        cond,
        r(4),
        "branch should resolve to the folded boolean value"
    );
    let mut src = match entry
        .insts
        .iter()
        .find(|i| i.dst == Some(r(4)))
        .expect("root bool value should still be materialized")
        .kind
    {
        Inst8Kind::Copy(src) => src,
        other => panic!("expected folded root copy, got {other:?}"),
    };
    while src != r(3) {
        let next = entry
            .insts
            .iter()
            .find(|i| i.dst == Some(src))
            .and_then(|i| match i.kind {
                Inst8Kind::Copy(next) => Some(next),
                _ => None,
            })
            .expect("folded root should resolve through copy chain to live boolean input");
        src = next;
    }
}

#[test]
fn opt8_folds_ltu_self_comparison_to_false() {
    let mut prog = mk_prog(vec![
        BasicBlock8 {
            id: Pc::new(0),
            insts: vec![Inst8::with_dst(r(20), Inst8Kind::LtU(r(8), r(8)))],
            terminator: Terminator8::Branch {
                cond: r(20),
                if_true: Pc::new(1),
                if_false: Pc::new(2),
            },
        },
        BasicBlock8 {
            id: Pc::new(1),
            insts: vec![],
            terminator: Terminator8::Exit { val: None },
        },
        BasicBlock8 {
            id: Pc::new(2),
            insts: vec![],
            terminator: Terminator8::Exit { val: None },
        },
    ]);

    run(&mut prog);

    let entry = prog.func_blocks[0]
        .iter()
        .find(|b| b.id == Pc::new(0))
        .expect("entry block should remain");
    assert!(
        !entry
            .insts
            .iter()
            .any(|i| matches!(i.kind, Inst8Kind::LtU(_, _))),
        "lt_u x x should fold away"
    );
    match entry.terminator {
        Terminator8::Goto(target) => assert_eq!(target, Pc::new(2)),
        Terminator8::Branch { cond, .. } => assert!(
            entry.insts.iter().any(|i| {
                i.dst == Some(cond) && matches!(i.kind, Inst8Kind::Copy(src) if src == Val8::imm(0))
            }),
            "branch condition should be folded to const false"
        ),
        Terminator8::Exit { .. } => {}
        _ => panic!("unexpected entry terminator"),
    }
}

#[test]
fn opt8_instcombine_simplifies_common_arithmetic_and_bitwise_identities() {
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::with_dst(r(20), Inst8Kind::Add(r(8), Val8::imm(0))),
            Inst8::with_dst(r(21), Inst8Kind::Sub(r(9), Val8::imm(0))),
            Inst8::with_dst(r(22), Inst8Kind::MulLo(r(10), Val8::imm(1))),
            Inst8::with_dst(r(23), Inst8Kind::And8(r(11), r(11))),
            Inst8::with_dst(r(24), Inst8Kind::Xor8(r(12), r(12))),
        ],
        terminator: Terminator8::Exit {
            val: Some(Word::new(r(20), r(21), r(22), r(24))),
        },
    }]);

    assert!(instcombine(&mut prog));
    let insts = &prog.func_blocks[0][0].insts;
    assert_eq!(insts[0].kind, Inst8Kind::Copy(r(8)));
    assert_eq!(insts[1].kind, Inst8Kind::Copy(r(9)));
    assert_eq!(insts[2].kind, Inst8Kind::Copy(r(10)));
    assert_eq!(insts[3].kind, Inst8Kind::Copy(r(11)));
    assert_eq!(insts[4].kind, Inst8Kind::Copy(Val8::imm(0)));
}

#[test]
fn opt8_instcombine_pushes_bool_not_into_unique_boolean_defs() {
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::with_dst(r(20), Inst8Kind::Eq(r(8), r(9))),
            Inst8::with_dst(r(21), Inst8Kind::BoolNot(r(20))),
            Inst8::with_dst(r(22), Inst8Kind::BoolNot(r(21))),
        ],
        terminator: Terminator8::Exit {
            val: Some(Word::new(r(20), r(21), r(22), r(4))),
        },
    }]);

    assert!(instcombine(&mut prog));
    let insts = &prog.func_blocks[0][0].insts;
    assert_eq!(insts[0].kind, Inst8Kind::Eq(r(8), r(9)));
    assert_eq!(insts[1].kind, Inst8Kind::Ne(r(8), r(9)));
    assert_eq!(insts[2].kind, Inst8Kind::Copy(r(20)));
}

#[test]
fn opt8_instcombine_simplifies_selects_that_reuse_boolean_condition() {
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::with_dst(r(20), Inst8Kind::Ne(r(8), r(9))),
            Inst8::with_dst(r(21), Inst8Kind::Sel(r(20), r(20), Val8::imm(0))),
            Inst8::with_dst(r(22), Inst8Kind::Sel(r(20), Val8::imm(1), r(20))),
            Inst8::with_dst(r(23), Inst8Kind::Sel(r(20), Val8::imm(0), r(20))),
            Inst8::with_dst(r(24), Inst8Kind::Sel(r(20), r(20), Val8::imm(1))),
        ],
        terminator: Terminator8::Exit {
            val: Some(Word::new(r(21), r(22), r(23), r(24))),
        },
    }]);

    assert!(instcombine(&mut prog));
    let insts = &prog.func_blocks[0][0].insts;
    assert_eq!(insts[1].kind, Inst8Kind::Copy(r(20)));
    assert_eq!(insts[2].kind, Inst8Kind::Copy(r(20)));
    assert_eq!(insts[3].kind, Inst8Kind::Copy(Val8::imm(0)));
    assert_eq!(insts[4].kind, Inst8Kind::Copy(Val8::imm(1)));
}

#[test]
fn opt8_instcombine_flips_select_using_bool_not_inner_condition() {
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::with_dst(r(20), Inst8Kind::Ne(r(8), r(9))),
            Inst8::with_dst(r(21), Inst8Kind::BoolNot(r(20))),
            Inst8::with_dst(r(22), Inst8Kind::Sel(r(21), r(10), r(11))),
        ],
        terminator: Terminator8::Exit {
            val: Some(Word::new(r(22), r(20), r(21), r(4))),
        },
    }]);

    assert!(instcombine(&mut prog));
    let insts = &prog.func_blocks[0][0].insts;
    assert_eq!(insts[2].kind, Inst8Kind::Sel(r(20), r(11), r(10)));
}

#[test]
fn opt8_instcombine_uses_inner_bool_for_ne_condition() {
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::with_dst(r(20), Inst8Kind::Ne(r(8), r(9))),
            Inst8::with_dst(r(21), Inst8Kind::Ne(r(20), Val8::imm(0))),
            Inst8::with_dst(r(22), Inst8Kind::Sel(r(21), r(10), r(11))),
        ],
        terminator: Terminator8::Exit {
            val: Some(Word::new(r(22), r(20), r(21), r(4))),
        },
    }]);

    assert!(instcombine(&mut prog));
    let insts = &prog.func_blocks[0][0].insts;
    assert_eq!(insts[2].kind, Inst8Kind::Sel(r(20), r(10), r(11)));
}

#[test]
fn opt8_instcombine_flips_branch_using_bool_not_inner_condition() {
    let mut prog = mk_prog(vec![
        BasicBlock8 {
            id: Pc::new(0),
            insts: vec![
                Inst8::with_dst(r(20), Inst8Kind::Ne(r(8), r(9))),
                Inst8::with_dst(r(21), Inst8Kind::BoolNot(r(20))),
            ],
            terminator: Terminator8::Branch {
                cond: r(21),
                if_true: Pc::new(1),
                if_false: Pc::new(2),
            },
        },
        BasicBlock8 {
            id: Pc::new(1),
            insts: vec![],
            terminator: Terminator8::Exit { val: None },
        },
        BasicBlock8 {
            id: Pc::new(2),
            insts: vec![],
            terminator: Terminator8::Exit { val: None },
        },
    ]);

    assert!(instcombine(&mut prog));
    let entry = &prog.func_blocks[0][0];
    let Terminator8::Branch {
        cond,
        if_true,
        if_false,
    } = entry.terminator
    else {
        panic!("expected branch");
    };
    assert_eq!(cond, r(20));
    assert_eq!(if_true, Pc::new(2));
    assert_eq!(if_false, Pc::new(1));
}

#[test]
fn opt8_instcombine_uses_inner_bool_for_ne_branch_condition() {
    let mut prog = mk_prog(vec![
        BasicBlock8 {
            id: Pc::new(0),
            insts: vec![
                Inst8::with_dst(r(20), Inst8Kind::Ne(r(8), r(9))),
                Inst8::with_dst(r(21), Inst8Kind::Ne(r(20), Val8::imm(0))),
            ],
            terminator: Terminator8::Branch {
                cond: r(21),
                if_true: Pc::new(1),
                if_false: Pc::new(2),
            },
        },
        BasicBlock8 {
            id: Pc::new(1),
            insts: vec![],
            terminator: Terminator8::Exit { val: None },
        },
        BasicBlock8 {
            id: Pc::new(2),
            insts: vec![],
            terminator: Terminator8::Exit { val: None },
        },
    ]);

    assert!(instcombine(&mut prog));
    let entry = &prog.func_blocks[0][0];
    let Terminator8::Branch {
        cond,
        if_true,
        if_false,
    } = entry.terminator
    else {
        panic!("expected branch");
    };
    assert_eq!(cond, r(20));
    assert_eq!(if_true, Pc::new(1));
    assert_eq!(if_false, Pc::new(2));
}

#[test]
fn opt8_copy_elimination_does_not_rewrite_live_in_value_to_later_def() {
    let addr = Addr::new(r(12), r(13));
    let mut prog = mk_prog(vec![BasicBlock8 {
        id: Pc::new(0),
        insts: vec![
            Inst8::with_dst(r(20), Inst8Kind::Copy(r(8))),
            Inst8::no_dst(Inst8Kind::StoreMem {
                base: 0,
                addr,
                lane: 0,
                val: r(20),
            }),
            Inst8::with_dst(
                r(30),
                Inst8Kind::LoadMem {
                    base: 0,
                    addr,
                    lane: 0,
                },
            ),
            Inst8::with_dst(r(8), Inst8Kind::Copy(r(30))),
        ],
        terminator: Terminator8::Exit { val: None },
    }]);

    run(&mut prog);

    let store = prog.func_blocks[0][0]
        .insts
        .iter()
        .find(|inst| matches!(inst.kind, Inst8Kind::StoreMem { .. }))
        .expect("store.mem should remain");
    let Inst8Kind::StoreMem { val, .. } = store.kind else {
        panic!("expected store.mem");
    };
    assert_ne!(
        val,
        r(30),
        "live-in source must not rewrite to a later load"
    );
}

#[test]
fn opt8_optimizer_keeps_saved_ret_lane_across_later_callsetup() {
    let mut prog = mk_prog(vec![
        BasicBlock8 {
            id: Pc::new(0),
            insts: vec![],
            terminator: Terminator8::CallSetup {
                callee_entry: BUILTIN_SHL_32,
                cont: Pc::new(1),
                args: vec![],
                callee_arg_vregs: vec![],
            },
        },
        BasicBlock8 {
            id: Pc::new(1),
            insts: vec![
                Inst8::with_dst(r(20), Inst8Kind::Copy(r(3))),
                Inst8::with_dst(r(21), Inst8Kind::Or8(r(20), Val8::imm(0))),
            ],
            terminator: Terminator8::CallSetup {
                callee_entry: BUILTIN_REM_U32,
                cont: Pc::new(2),
                args: vec![],
                callee_arg_vregs: vec![],
            },
        },
        BasicBlock8 {
            id: Pc::new(2),
            insts: vec![
                Inst8::with_dst(r(22), Inst8Kind::Copy(r(3))),
                Inst8::with_dst(r(23), Inst8Kind::Xor8(r(21), r(22))),
            ],
            terminator: Terminator8::Exit {
                val: Some(Word::new(Val8::imm(0), Val8::imm(0), Val8::imm(0), r(23))),
            },
        },
    ]);

    run(&mut prog);

    let exit_word = prog.func_blocks[0]
        .iter()
        .find_map(|bb| match bb.terminator {
            Terminator8::Exit { val: Some(w) } => Some(w),
            _ => None,
        })
        .expect("exit word should remain");
    assert_ne!(
        exit_word.b3,
        Val8::imm(0),
        "saved return lane should survive across a later callsetup"
    );
}
