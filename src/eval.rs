use std::{cell::RefCell, rc::Rc};

use llzk::{
    dialect,
    prelude::{
        BlockLike, FuncDefOpLike, FuncDefOpRef, Module, OperationLike, OperationRef, RegionLike,
        StructDefOpLike,
    },
};

use std::collections::VecDeque;

use crate::{
    Error, Result,
    dispatch::{
        call_target, find_function, find_struct, fq_function_name, iter_block_ops, member_name,
        operands, parse_cmp_predicate, parse_felt_const, parse_usize_const, result_struct_name,
    },
    state::{ExecutionState, Frame},
    value::{ArrayInstance, Felt, StructInstance, Value},
};

/// Concrete interpreter for a small LLZK subset.
pub struct Interpreter<'c, 'm> {
    module: &'m Module<'c>,
    state: ExecutionState,
    nondet_queue: VecDeque<Felt>,
}

impl<'c, 'm> Interpreter<'c, 'm> {
    /// Creates a new interpreter for the given module.
    pub fn new(module: &'m Module<'c>) -> Self {
        Self {
            module,
            state: ExecutionState::default(),
            nondet_queue: VecDeque::new(),
        }
    }

    /// Returns the collected constraint checks.
    pub fn state(&self) -> &ExecutionState {
        &self.state
    }

    pub fn set_nondet_values(&mut self, values: impl IntoIterator<Item = Felt>) {
        self.nondet_queue = values.into_iter().collect();
    }

    /// Executes `@Struct::@compute` and returns the resulting struct instance.
    pub fn run_compute(&mut self, struct_name: &str, inputs: &[Value]) -> Result<StructInstance> {
        let struct_def = find_struct(self.module, struct_name)?;
        let compute = struct_def
            .get_compute_func()
            .ok_or_else(|| Error::SymbolNotFound(format!("compute for struct @{struct_name}")))?;
        let results = self.execute_function(&compute, inputs)?;
        let value = results
            .into_iter()
            .next()
            .ok_or_else(|| Error::MalformedOp("compute returned no values".into()))?;
        let struct_ref = value.as_struct().map_err(Error::TypeError)?;
        Ok(struct_ref.borrow().clone())
    }

    /// Executes `@Struct::@constrain` against a concrete self value and inputs.
    pub fn run_constrain(
        &mut self,
        struct_name: &str,
        self_value: StructInstance,
        inputs: &[Value],
    ) -> Result<()> {
        let struct_def = find_struct(self.module, struct_name)?;
        let constrain = struct_def
            .get_constrain_func()
            .ok_or_else(|| Error::SymbolNotFound(format!("constrain for struct @{struct_name}")))?;
        let mut args = Vec::with_capacity(inputs.len() + 1);
        args.push(Value::Struct(Rc::new(RefCell::new(self_value))));
        args.extend_from_slice(inputs);
        let _ = self.execute_function(&constrain, &args)?;
        Ok(())
    }

    /// Executes a fully qualified module-level or nested function name.
    pub fn run_function(&mut self, symbol: &str, args: &[Value]) -> Result<Vec<Value>> {
        let func = find_function(self.module, symbol)?;
        self.execute_function(&func, args)
    }

    fn execute_function(
        &mut self,
        func: &FuncDefOpRef<'c, '_>,
        args: &[Value],
    ) -> Result<Vec<Value>> {
        self.state.call_stack.push(fq_function_name(func));

        let block = func
            .region(0)
            .map_err(Error::from)?
            .first_block()
            .ok_or_else(|| Error::MalformedOp("function without entry block".into()))?;

        let mut frame = Frame::default();
        for (idx, arg) in args.iter().enumerate() {
            let block_arg = func.argument(idx).map_err(Error::from)?;
            frame.insert(block_arg.into(), arg.clone());
        }

        let result = (|| {
            for op in iter_block_ops(block) {
                if dialect::function::is_func_return(&op) {
                    return self.eval_return(&op, &frame);
                }
                self.eval_op(&op, &mut frame)?;
            }
            Err(Error::MalformedOp("function ended without return".into()))
        })();

        self.state.call_stack.pop();
        result
    }

    fn eval_return(&self, op: &OperationRef<'c, '_>, frame: &Frame) -> Result<Vec<Value>> {
        operands(op)?
            .into_iter()
            .map(|operand| {
                frame
                    .get(operand)
                    .cloned()
                    .ok_or_else(|| Error::MissingValue(format!("missing return operand {operand}")))
            })
            .collect()
    }

    fn eval_scf_if(&mut self, op: &OperationRef<'c, '_>, frame: &mut Frame) -> Result<()> {
        let ops = operands(op)?;
        let condition = frame
            .get(ops[0])
            .cloned()
            .ok_or_else(|| Error::MissingValue("missing scf.if condition".into()))?
            .as_bool()
            .map_err(Error::TypeError)?;

        // scf.if has two regions: then (index 0) and else (index 1).
        let region_index = if condition { 0 } else { 1 };
        let region = op.region(region_index).map_err(Error::from)?;
        let block = region
            .first_block()
            .ok_or_else(|| Error::MalformedOp("scf.if region without block".into()))?;

        // Execute the block. scf.yield terminates it.
        for inner_op in iter_block_ops(block) {
            if dialect::scf_ext::is_scf_yield(&inner_op) {
                // Bind yielded values to the scf.if results.
                let yield_operands = operands(&inner_op)?;
                for (index, yield_val) in yield_operands.into_iter().enumerate() {
                    let value = frame.get(yield_val).cloned().ok_or_else(|| {
                        Error::MissingValue(format!("missing scf.yield operand {index}"))
                    })?;
                    frame.insert(op.result(index).map_err(Error::from)?.into(), value);
                }
                return Ok(());
            }
            self.eval_op(&inner_op, frame)?;
        }
        // If the region has no yield (e.g. zero results), that's fine.
        Ok(())
    }

    fn eval_scf_while(&mut self, op: &OperationRef<'c, '_>, frame: &mut Frame) -> Result<()> {
        let before_block = op
            .region(0)
            .map_err(Error::from)?
            .first_block()
            .ok_or_else(|| Error::MalformedOp("scf.while before region without block".into()))?;
        let after_block = op
            .region(1)
            .map_err(Error::from)?
            .first_block()
            .ok_or_else(|| Error::MalformedOp("scf.while after region without block".into()))?;

        let init_operands = operands(op)?;
        let mut current: Vec<Value> = init_operands
            .into_iter()
            .map(|operand| {
                frame.get(operand).cloned().ok_or_else(|| {
                    Error::MissingValue(format!("missing scf.while init operand {operand}"))
                })
            })
            .collect::<Result<Vec<_>>>()?;

        loop {
            for (idx, value) in current.iter().enumerate() {
                let block_arg = before_block.argument(idx).map_err(Error::from)?;
                frame.insert(block_arg.into(), value.clone());
            }

            let mut condition_holds = false;
            let mut forwarded: Vec<Value> = Vec::new();
            let mut saw_condition = false;
            for inner_op in iter_block_ops(before_block) {
                if dialect::scf_ext::is_scf_condition(&inner_op) {
                    let cond_ops = operands(&inner_op)?;
                    let (cond_operand, value_operands) = cond_ops.split_first().ok_or_else(|| {
                        Error::MalformedOp("scf.condition missing predicate".into())
                    })?;
                    condition_holds = frame
                        .get(*cond_operand)
                        .cloned()
                        .ok_or_else(|| {
                            Error::MissingValue("missing scf.condition predicate".into())
                        })?
                        .as_bool()
                        .map_err(Error::TypeError)?;
                    forwarded = value_operands
                        .iter()
                        .map(|v| {
                            frame.get(*v).cloned().ok_or_else(|| {
                                Error::MissingValue(format!("missing scf.condition forward {v}"))
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    saw_condition = true;
                    break;
                }
                self.eval_op(&inner_op, frame)?;
            }
            if !saw_condition {
                return Err(Error::MalformedOp(
                    "scf.while before region without scf.condition".into(),
                ));
            }

            if !condition_holds {
                for (idx, value) in forwarded.into_iter().enumerate() {
                    frame.insert(op.result(idx).map_err(Error::from)?.into(), value);
                }
                return Ok(());
            }

            for (idx, value) in forwarded.iter().enumerate() {
                let block_arg = after_block.argument(idx).map_err(Error::from)?;
                frame.insert(block_arg.into(), value.clone());
            }

            let mut next: Option<Vec<Value>> = None;
            for inner_op in iter_block_ops(after_block) {
                if dialect::scf_ext::is_scf_yield(&inner_op) {
                    let yielded = operands(&inner_op)?
                        .into_iter()
                        .map(|v| {
                            frame.get(v).cloned().ok_or_else(|| {
                                Error::MissingValue(format!("missing scf.yield operand {v}"))
                            })
                        })
                        .collect::<Result<Vec<_>>>()?;
                    next = Some(yielded);
                    break;
                }
                self.eval_op(&inner_op, frame)?;
            }
            current = next.ok_or_else(|| {
                Error::MalformedOp("scf.while after region without scf.yield".into())
            })?;
        }
    }

    fn eval_op(&mut self, op: &OperationRef<'c, '_>, frame: &mut Frame) -> Result<()> {
        if op.name().as_string_ref().as_str() == Ok("arith.constant") {
            let attr = op.attribute("value").map_err(Error::from)?;
            let value = Value::Index(parse_usize_const(attr)?);
            frame.insert(op.result(0).map_err(Error::from)?.into(), value);
            return Ok(());
        }

        if dialect::llzk::is_nondet(op) {
            // Pull from the pre-supplied queue, falling back to zero.
            let value = self.nondet_queue.pop_front().unwrap_or_else(Felt::zero);
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Felt(value),
            );
            return Ok(());
        }

        if dialect::r#struct::is_struct_new(op) {
            let struct_name = result_struct_name(op)?;
            let value = Value::Struct(Rc::new(RefCell::new(StructInstance::new(struct_name))));
            frame.insert(op.result(0).map_err(Error::from)?.into(), value);
            return Ok(());
        }

        if dialect::array::is_array_new(op) {
            let values = operands(op)?
                .into_iter()
                .map(|operand| {
                    frame.get(operand).cloned().ok_or_else(|| {
                        Error::MissingValue(format!("missing array.new operand {operand}"))
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let array = if values.is_empty() {
                ArrayInstance::new()
            } else {
                ArrayInstance::from_values(values)
            };
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Array(Rc::new(RefCell::new(array))),
            );
            return Ok(());
        }

        if dialect::array::is_array_write(op) {
            let ops = operands(op)?;
            let (array_operand, rest) = ops
                .split_first()
                .ok_or_else(|| Error::MalformedOp("array.write missing array operand".into()))?;
            let (value_operand, index_operands) = rest
                .split_last()
                .ok_or_else(|| Error::MalformedOp("array.write missing value operand".into()))?;
            let array = frame
                .get(*array_operand)
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing array.write array".into()))?
                .as_array()
                .map_err(Error::TypeError)?;
            let indices = index_operands
                .iter()
                .map(|index| {
                    frame
                        .get(*index)
                        .cloned()
                        .ok_or_else(|| {
                            Error::MissingValue(format!("missing array.write index {index}"))
                        })?
                        .as_index()
                        .map_err(Error::TypeError)
                })
                .collect::<Result<Vec<_>>>()?;
            let value = frame
                .get(*value_operand)
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing array.write value".into()))?;
            array.borrow_mut().write(&indices, value);
            return Ok(());
        }

        if dialect::array::is_array_read(op) {
            let ops = operands(op)?;
            let (array_operand, index_operands) = ops
                .split_first()
                .ok_or_else(|| Error::MalformedOp("array.read missing array operand".into()))?;
            let array = frame
                .get(*array_operand)
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing array.read array".into()))?
                .as_array()
                .map_err(Error::TypeError)?;
            let indices = index_operands
                .iter()
                .map(|index| {
                    frame
                        .get(*index)
                        .cloned()
                        .ok_or_else(|| {
                            Error::MissingValue(format!("missing array.read index {index}"))
                        })?
                        .as_index()
                        .map_err(Error::TypeError)
                })
                .collect::<Result<Vec<_>>>()?;
            let value = array
                .borrow()
                .read(&indices)
                .ok_or_else(|| Error::MissingValue(format!("missing array element {indices:?}")))?;
            frame.insert(op.result(0).map_err(Error::from)?.into(), value);
            return Ok(());
        }

        if dialect::r#struct::is_struct_writem(op) {
            let member = member_name(op)?;
            let ops = operands(op)?;
            let target = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing struct.writem target".into()))?;
            let value = frame
                .get(ops[1])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing struct.writem value".into()))?;
            let struct_ref = target.as_struct().map_err(Error::TypeError)?;
            struct_ref.borrow_mut().members.insert(member, value);
            return Ok(());
        }

        if dialect::r#struct::is_struct_readm(op) {
            let member = member_name(op)?;
            let ops = operands(op)?;
            let target = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing struct.readm target".into()))?;
            let struct_ref = target.as_struct().map_err(Error::TypeError)?;
            let value = struct_ref
                .borrow()
                .members
                .get(&member)
                .cloned()
                .ok_or_else(|| Error::MissingValue(format!("missing struct member {member}")))?;
            frame.insert(op.result(0).map_err(Error::from)?.into(), value);
            return Ok(());
        }

        if dialect::felt::is_felt_const(op) {
            let attr = op.attribute("value").map_err(Error::from)?;
            let value = Value::Felt(parse_felt_const(attr)?);
            frame.insert(op.result(0).map_err(Error::from)?.into(), value);
            return Ok(());
        }

        if dialect::cast::is_cast_toindex(op) {
            let ops = operands(op)?;
            let value = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing cast.toindex operand".into()))?;
            let index = match value {
                Value::Index(index) => index,
                Value::Felt(felt) => felt
                    .as_biguint()
                    .try_into()
                    .map_err(|_| Error::TypeError("felt does not fit in usize index".into()))?,
                other => {
                    return Err(Error::TypeError(format!(
                        "expected felt or index for cast.toindex, got {other}"
                    )));
                }
            };
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Index(index),
            );
            return Ok(());
        }

        if dialect::cast::is_cast_tofelt(op) {
            let ops = operands(op)?;
            let value = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing cast.tofelt operand".into()))?;
            let felt = match value {
                Value::Felt(felt) => felt,
                Value::Index(index) => Felt::from_u64(index as u64),
                Value::Bool(b) => Felt::from_u64(b as u64),
                other => {
                    return Err(Error::TypeError(format!(
                        "expected felt, index, or bool for cast.tofelt, got {other}"
                    )));
                }
            };
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Felt(felt),
            );
            return Ok(());
        }

        if dialect::felt::is_felt_add(op)
            || dialect::felt::is_felt_sub(op)
            || dialect::felt::is_felt_mul(op)
            || dialect::felt::is_felt_div(op)
            || dialect::felt::is_felt_uintdiv(op)
            || dialect::felt::is_felt_umod(op)
        {
            let ops = operands(op)?;
            let lhs = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing felt lhs".into()))?
                .as_felt()
                .map_err(Error::TypeError)?;
            let rhs = frame
                .get(ops[1])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing felt rhs".into()))?
                .as_felt()
                .map_err(Error::TypeError)?;
            let value = if dialect::felt::is_felt_add(op) {
                lhs + rhs
            } else if dialect::felt::is_felt_sub(op) {
                lhs - rhs
            } else if dialect::felt::is_felt_mul(op) {
                lhs * rhs
            } else if dialect::felt::is_felt_div(op) {
                lhs / rhs
            } else if dialect::felt::is_felt_uintdiv(op) {
                Felt::new(lhs.as_biguint() / rhs.as_biguint())
            } else {
                Felt::new(lhs.as_biguint() % rhs.as_biguint())
            };
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Felt(value),
            );
            return Ok(());
        }

        if dialect::felt::is_felt_neg(op) {
            let ops = operands(op)?;
            let value = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing felt.neg operand".into()))?
                .as_felt()
                .map_err(Error::TypeError)?;
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Felt(-value),
            );
            return Ok(());
        }

        if dialect::felt::is_felt_bit_and(op)
            || dialect::felt::is_felt_bit_or(op)
            || dialect::felt::is_felt_bit_xor(op)
        {
            let ops = operands(op)?;
            let lhs = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing felt bitwise lhs".into()))?
                .as_felt()
                .map_err(Error::TypeError)?;
            let rhs = frame
                .get(ops[1])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing felt bitwise rhs".into()))?
                .as_felt()
                .map_err(Error::TypeError)?;
            let value = if dialect::felt::is_felt_bit_and(op) {
                lhs.bit_and(&rhs)
            } else if dialect::felt::is_felt_bit_or(op) {
                lhs.bit_or(&rhs)
            } else {
                lhs.bit_xor(&rhs)
            };
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Felt(value),
            );
            return Ok(());
        }

        if dialect::felt::is_felt_shl(op) || dialect::felt::is_felt_shr(op) {
            let ops = operands(op)?;
            let value = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing felt shift value".into()))?
                .as_felt()
                .map_err(Error::TypeError)?;
            let amount = frame
                .get(ops[1])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing felt shift amount".into()))?
                .as_felt()
                .map_err(Error::TypeError)?;
            let amount_u32: u32 = amount
                .as_biguint()
                .try_into()
                .map_err(|_| Error::MalformedOp("shift amount too large".into()))?;
            let result = if dialect::felt::is_felt_shl(op) {
                value.shl(amount_u32)
            } else {
                value.shr(amount_u32)
            };
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Felt(result),
            );
            return Ok(());
        }

        if dialect::felt::is_felt_pow(op) {
            let ops = operands(op)?;
            let base = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing felt.pow base".into()))?
                .as_felt()
                .map_err(Error::TypeError)?;
            let exp = frame
                .get(ops[1])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing felt.pow exponent".into()))?
                .as_felt()
                .map_err(Error::TypeError)?;
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Felt(base.pow(&exp)),
            );
            return Ok(());
        }

        if dialect::bool::is_bool_assert(op) {
            let ops = operands(op)?;
            let cond = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing bool.assert operand".into()))?
                .as_bool()
                .map_err(Error::TypeError)?;
            if !cond {
                return Err(Error::ConstraintFailed("bool.assert failed".into()));
            }
            return Ok(());
        }

        if dialect::scf_ext::is_scf_if(op) {
            return self.eval_scf_if(op, frame);
        }

        if dialect::scf_ext::is_scf_while(op) {
            return self.eval_scf_while(op, frame);
        }

        if op.name().as_string_ref().as_str() == Ok("bool.eq") {
            let ops = operands(op)?;
            let lhs = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing bool.eq lhs".into()))?;
            let rhs = frame
                .get(ops[1])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing bool.eq rhs".into()))?;
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Bool(lhs == rhs),
            );
            return Ok(());
        }

        if dialect::bool::is_bool_cmp(op) {
            let predicate = parse_cmp_predicate(op.attribute("predicate").map_err(Error::from)?)?;
            let ops = operands(op)?;
            let lhs = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing bool.cmp lhs".into()))?
                .as_felt()
                .map_err(Error::TypeError)?;
            let rhs = frame
                .get(ops[1])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing bool.cmp rhs".into()))?
                .as_felt()
                .map_err(Error::TypeError)?;
            let value = match predicate {
                "eq" => lhs == rhs,
                "ne" => lhs != rhs,
                "lt" => lhs.as_biguint() < rhs.as_biguint(),
                "le" => lhs.as_biguint() <= rhs.as_biguint(),
                "gt" => lhs.as_biguint() > rhs.as_biguint(),
                "ge" => lhs.as_biguint() >= rhs.as_biguint(),
                _ => unreachable!(),
            };
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Bool(value),
            );
            return Ok(());
        }

        if dialect::bool::is_bool_and(op) || dialect::bool::is_bool_or(op) {
            let ops = operands(op)?;
            let lhs = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing bool lhs".into()))?
                .as_bool()
                .map_err(Error::TypeError)?;
            let rhs = frame
                .get(ops[1])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing bool rhs".into()))?
                .as_bool()
                .map_err(Error::TypeError)?;
            let value = if dialect::bool::is_bool_and(op) {
                lhs && rhs
            } else {
                lhs || rhs
            };
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Bool(value),
            );
            return Ok(());
        }

        if dialect::bool::is_bool_not(op) {
            let ops = operands(op)?;
            let value = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing bool.not operand".into()))?
                .as_bool()
                .map_err(Error::TypeError)?;
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Bool(!value),
            );
            return Ok(());
        }

        if dialect::constrain::is_constrain_eq(op) {
            let ops = operands(op)?;
            let lhs = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing constrain.eq lhs".into()))?;
            let rhs = frame
                .get(ops[1])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing constrain.eq rhs".into()))?;
            self.state.record_constraint(lhs.clone(), rhs.clone());
            if lhs != rhs {
                return Err(Error::ConstraintFailed(format!("{lhs} != {rhs}")));
            }
            return Ok(());
        }

        if dialect::function::is_func_call(op) {
            let symbol = call_target(op)?;
            let args = operands(op)?
                .into_iter()
                .map(|operand| {
                    frame.get(operand).cloned().ok_or_else(|| {
                        Error::MissingValue(format!("missing call operand {operand}"))
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let callee = find_function(self.module, &symbol)?;
            let results = self.execute_function(&callee, &args)?;
            for (index, value) in results.into_iter().enumerate() {
                frame.insert(op.result(index).map_err(Error::from)?.into(), value);
            }
            return Ok(());
        }

        if dialect::ram::is_ram_store(op) {
            let ops = operands(op)?;
            let addr = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing ram.store address".into()))?
                .as_index()
                .map_err(Error::TypeError)?;
            let value = frame
                .get(ops[1])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing ram.store value".into()))?
                .as_felt()
                .map_err(Error::TypeError)?;
            self.state.ram_store(addr, value);
            return Ok(());
        }

        if dialect::ram::is_ram_load(op) {
            let ops = operands(op)?;
            let addr = frame
                .get(ops[0])
                .cloned()
                .ok_or_else(|| Error::MissingValue("missing ram.load address".into()))?
                .as_index()
                .map_err(Error::TypeError)?;
            let value = self.state.ram_load(addr);
            frame.insert(
                op.result(0).map_err(Error::from)?.into(),
                Value::Felt(value),
            );
            return Ok(());
        }

        let op_name = op
            .name()
            .as_string_ref()
            .as_str()
            .unwrap_or("<unknown-op>")
            .to_string();
        Err(Error::UnsupportedOp(op_name))
    }
}
