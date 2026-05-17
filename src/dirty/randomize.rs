use std::collections::{HashMap, HashSet};

use super::{SplitMix64, fisher_yates};
use crate::ir8::{Ir8Program, Pc};
use crate::schedule::{rewrite_inst_pcs, rewrite_term_pcs};

pub fn randomize_pcs(ir8: &mut Ir8Program, seed: u64) -> anyhow::Result<()> {
    apply_pc_relabel(ir8, seed, |n, rng| {
        let mut labels: Vec<u16> = (1..=n).collect();
        fisher_yates(&mut labels, rng);
        Ok(labels)
    })
}

pub fn sparsify_pcs(ir8: &mut Ir8Program, seed: u64) -> anyhow::Result<()> {
    apply_pc_relabel(ir8, seed, |n, rng| {
        // Trap PCs are negative i32; the positive u16 range is free.
        let pool = u32::from(u16::MAX);
        anyhow::ensure!(
            u32::from(n) <= pool,
            "program has too many cycles to sparsify within the 16-bit PC space"
        );
        let mut taken: HashSet<u16> = HashSet::with_capacity(usize::from(n));
        let mut labels: Vec<u16> = Vec::with_capacity(usize::from(n));
        while labels.len() < usize::from(n) {
            let v = ((rng.next_u64() % u64::from(pool)) + 1) as u16;
            if taken.insert(v) {
                labels.push(v);
            }
        }
        Ok(labels)
    })
}

fn apply_pc_relabel(
    ir8: &mut Ir8Program,
    seed: u64,
    gen_labels: impl FnOnce(u16, &mut SplitMix64) -> anyhow::Result<Vec<u16>>,
) -> anyhow::Result<()> {
    let n = ir8.cycles.len();
    if n == 0 {
        return Ok(());
    }
    let n_u16 = u16::try_from(n)
        .map_err(|_| anyhow::anyhow!("program has more cycles than fit in a 16-bit PC"))?;
    let mut rng = SplitMix64::new(seed);
    let labels = gen_labels(n_u16, &mut rng)?;
    assert_eq!(labels.len(), n);

    let mut remap: HashMap<Pc, Pc> = HashMap::with_capacity(n);
    for (i, cycle) in ir8.cycles.iter().enumerate() {
        remap.insert(cycle.pc, Pc::new(labels[i]));
    }

    for cycle in &mut ir8.cycles {
        cycle.pc = *remap.get(&cycle.pc).expect("missing pc in remap");
        cycle.terminator = rewrite_term_pcs(cycle.terminator.clone(), &remap)?;
        let ops = std::mem::take(&mut cycle.ops);
        cycle.ops = ops
            .into_iter()
            .map(|inst| rewrite_inst_pcs(inst, &remap))
            .collect::<anyhow::Result<Vec<_>>>()?;
    }

    for pc in &mut ir8.func_entries {
        if let Some(&new) = remap.get(pc) {
            *pc = new;
        }
    }

    ir8.cycles.sort_by_key(|c| c.pc.index());
    Ok(())
}
