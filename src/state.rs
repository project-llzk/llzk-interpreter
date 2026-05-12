use std::collections::HashMap;

use llzk::prelude::{Value as MlirValue, ValueLike};

use crate::value::{Felt, Value};

/// A concrete equality check observed during `constrain`.
#[derive(Clone, Debug)]
pub struct ConstraintRecord {
    /// Left-hand side runtime value.
    pub lhs: Value,
    /// Right-hand side runtime value.
    pub rhs: Value,
    /// Whether the equality held concretely.
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

/// One function activation record.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    bindings: HashMap<usize, Value>,
    origins: HashMap<usize, Origin>,
}

impl Frame {
    /// Inserts a runtime value for an SSA value. The origin defaults to
    /// `Dynamic`; callers may override with [`Frame::set_origin`].
    pub fn insert<'c, 'a>(&mut self, key: MlirValue<'c, 'a>, value: Value) {
        self.bindings.insert(value_key(key), value);
    }

    /// Inserts a runtime value together with its provenance.
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

    /// Retrieves a runtime value for an SSA value.
    pub fn get<'c, 'a>(&self, key: MlirValue<'c, 'a>) -> Option<&Value> {
        self.bindings.get(&value_key(key))
    }

    /// Overrides the origin tag of an already-inserted value.
    pub fn set_origin<'c, 'a>(&mut self, key: MlirValue<'c, 'a>, origin: Origin) {
        self.origins.insert(value_key(key), origin);
    }

    /// Returns the origin tag of an SSA value, defaulting to `Dynamic` for
    /// values inserted without explicit provenance.
    pub fn origin<'c, 'a>(&self, key: MlirValue<'c, 'a>) -> Origin {
        self.origins
            .get(&value_key(key))
            .copied()
            .unwrap_or(Origin::Dynamic)
    }
}

/// Global execution bookkeeping.
#[derive(Clone, Debug, Default)]
pub struct ExecutionState {
    /// Active call stack, using the fully qualified function names.
    pub call_stack: Vec<String>,
    /// Concrete constraints checked so far.
    pub constraints: Vec<ConstraintRecord>,
    /// Flat felt memory region addressed by `ram.load` / `ram.store`.
    /// Unwritten cells read as zero.
    ram: HashMap<usize, Felt>,
}

impl ExecutionState {
    /// Records a checked equality.
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

/// Computes a stable key for an SSA value.
pub fn value_key<'c, 'a>(value: MlirValue<'c, 'a>) -> usize {
    value.to_raw().ptr as usize
}
