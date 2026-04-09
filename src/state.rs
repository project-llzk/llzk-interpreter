use std::collections::HashMap;

use llzk::prelude::{Value as MlirValue, ValueLike};

use crate::value::Value;

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

/// One function activation record.
#[derive(Clone, Debug, Default)]
pub struct Frame {
    bindings: HashMap<usize, Value>,
}

impl Frame {
    /// Inserts a runtime value for an SSA value.
    pub fn insert<'c, 'a>(&mut self, key: MlirValue<'c, 'a>, value: Value) {
        self.bindings.insert(value_key(key), value);
    }

    /// Retrieves a runtime value for an SSA value.
    pub fn get<'c, 'a>(&self, key: MlirValue<'c, 'a>) -> Option<&Value> {
        self.bindings.get(&value_key(key))
    }
}

/// Global execution bookkeeping.
#[derive(Clone, Debug, Default)]
pub struct ExecutionState {
    /// Active call stack, using the fully qualified function names.
    pub call_stack: Vec<String>,
    /// Concrete constraints checked so far.
    pub constraints: Vec<ConstraintRecord>,
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
}

/// Computes a stable key for an SSA value.
pub fn value_key<'c, 'a>(value: MlirValue<'c, 'a>) -> usize {
    value.to_raw().ptr as usize
}
