use std::collections::HashMap;

use llzk::prelude::{Value as MlirValue, ValueLike};

use crate::value::{Felt, Value};

#[derive(Clone, Debug)]
pub struct ConstraintRecord {
    pub lhs: Value,
    pub rhs: Value,
    pub satisfied: bool,
}

/// Whether the running function body is a witness-generation body (`@compute`,
/// Brillig, helpers called from those) or a constraint body (`@constrain` and
/// any helper transitively reached from it).
///
/// Only `Constrain` triggers the soundness checks that mirror what the proof
/// backend can actually lower to polynomial constraints.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Compute,
    Constrain,
}

/// Origin of an SSA value: a `Const` value's def chain folds to compile-time
/// constants; a `Dynamic` value depends on a nondet or a function argument.
///
/// PCL only lowers `bool.assert` in `@constrain` when its operand is `Const`,
/// so `bool.assert(Dynamic)` is a silent no-op in the prover and must error.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Origin {
    Const,
    Dynamic,
}

#[derive(Clone, Debug, Default)]
pub struct Frame {
    bindings: HashMap<usize, Value>,
    origins: HashMap<usize, Origin>,
}

impl Frame {
    pub fn insert<'c, 'a>(&mut self, key: MlirValue<'c, 'a>, value: Value) {
        self.bindings.insert(value_key(key), value);
    }

    pub fn insert_with_origin<'c, 'a>(
        &mut self,
        key: MlirValue<'c, 'a>,
        value: Value,
        origin: Origin,
    ) {
        let k = value_key(key);
        self.bindings.insert(k, value);
        self.origins.insert(k, origin);
    }

    pub fn get<'c, 'a>(&self, key: MlirValue<'c, 'a>) -> Option<&Value> {
        self.bindings.get(&value_key(key))
    }

    pub fn set_origin<'c, 'a>(&mut self, key: MlirValue<'c, 'a>, origin: Origin) {
        self.origins.insert(value_key(key), origin);
    }

    /// Defaults to `Dynamic` for values inserted without an explicit tag.
    pub fn origin<'c, 'a>(&self, key: MlirValue<'c, 'a>) -> Origin {
        self.origins
            .get(&value_key(key))
            .copied()
            .unwrap_or(Origin::Dynamic)
    }
}

/// Always-on counters; each costs a `u64` increment per event.
#[derive(Clone, Debug, Default)]
pub struct Stats {
    pub function_calls: u64,
    /// Includes top-level entry points plus every `function.call` dispatch.
    pub execute_function_invocations: u64,
    pub op_dispatches: u64,
    /// `function.def` ops indexed at construction. Coarse module-size signal.
    pub find_function_probes: u64,
    pub callee_counts: HashMap<String, u64>,
}

impl std::fmt::Display for Stats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "op dispatches:             {}", self.op_dispatches)?;
        writeln!(
            f,
            "execute_function calls:    {}",
            self.execute_function_invocations
        )?;
        writeln!(f, "function.call count:       {}", self.function_calls)?;
        writeln!(
            f,
            "cached function defs:      {}",
            self.find_function_probes
        )?;
        let ops_per_frame = if self.execute_function_invocations > 0 {
            self.op_dispatches as f64 / self.execute_function_invocations as f64
        } else {
            0.0
        };
        writeln!(f, "avg ops per frame:         {ops_per_frame:.1}")?;
        writeln!(f, "distinct callees:          {}", self.callee_counts.len())?;
        let mut top: Vec<(&String, &u64)> = self.callee_counts.iter().collect();
        top.sort_by(|a, b| b.1.cmp(a.1));
        writeln!(f, "top callees:")?;
        for (name, count) in top.iter().take(10) {
            writeln!(f, "  {count:>8}  {name}")?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
pub struct ExecutionState {
    pub call_stack: Vec<String>,
    pub constraints: Vec<ConstraintRecord>,
    /// Unwritten cells read as zero.
    ram: HashMap<usize, Felt>,
    pub stats: Stats,
}

impl ExecutionState {
    pub fn record_constraint(&mut self, lhs: Value, rhs: Value) {
        let satisfied = lhs == rhs;
        self.constraints.push(ConstraintRecord {
            lhs,
            rhs,
            satisfied,
        });
    }

    pub fn ram_store(&mut self, addr: usize, value: Felt) {
        self.ram.insert(addr, value);
    }

    pub fn ram_load(&self, addr: usize) -> Felt {
        self.ram.get(&addr).cloned().unwrap_or_else(Felt::zero)
    }
}

pub fn value_key<'c, 'a>(value: MlirValue<'c, 'a>) -> usize {
    value.to_raw().ptr as usize
}
