use std::{cell::RefCell, rc::Rc};

use llzk::{
    dialect,
    prelude::{
        BlockLike, FuncDefOpLike, FuncDefOpRef, IntegerType, Module, OperationLike, OperationRef,
        RegionLike, StructDefOpLike, TypeLike, ValueLike,
    },
};
use num_bigint::{BigInt, BigUint};
use num_traits::{One, Zero};

use std::collections::VecDeque;

use crate::{
    Error, Result,
    dispatch::{
        CmpIPredicate, call_target, find_function, find_struct, fq_function_name, iter_block_ops,
        member_name, operands, parse_arith_const_value, parse_cmp_predicate, parse_cmpi_predicate,
        parse_felt_const, result_struct_name,
    },
    state::{ExecutionState, Frame, Origin, Phase},
    value::{ArrayInstance, Felt, IntValue, StructInstance, Value},
};

/// Concrete interpreter for a small LLZK subset.
pub struct Interpreter<'c, 'm> {
    module: &'m Module<'c>,
    state: ExecutionState,
    nondet_queue: VecDeque<Felt>,
    phase: Phase,
}

impl<'c, 'm> Interpreter<'c, 'm> {
    /// Creates a new interpreter for the given module.
    pub fn new(module: &'m Module<'c>) -> Self {
        Self {
            module,
            state: ExecutionState::default(),
            nondet_queue: VecDeque::new(),
            phase: Phase::Compute,
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
        let prev_phase = std::mem::replace(&mut self.phase, Phase::Compute);
        let origins = vec![Origin::Dynamic; inputs.len()];
        let result = self.execute_function(&compute, inputs, &origins);
        self.phase = prev_phase;
        let results = result?;
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
        let origins = vec![Origin::Dynamic; args.len()];
        let prev_phase = std::mem::replace(&mut self.phase, Phase::Constrain);
        let result = self.execute_function(&constrain, &args, &origins);
        self.phase = prev_phase;
        let _ = result?;
        Ok(())
    }

    /// Executes a fully qualified module-level or nested function name.
    pub fn run_function(&mut self, symbol: &str, args: &[Value]) -> Result<Vec<Value>> {
        let func = find_function(self.module, symbol)?;
        let origins = vec![Origin::Dynamic; args.len()];
        self.execute_function(&func, args, &origins)
    }

    fn execute_function(
        &mut self,
        func: &FuncDefOpRef<'c, '_>,
        args: &[Value],
        arg_origins: &[Origin],
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
            let origin = arg_origins.get(idx).copied().unwrap_or(Origin::Dynamic);
            frame.insert_with_origin(block_arg.into(), arg.clone(), origin);
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

    fn eval_arith_op(
        &mut self,
        suffix: &str,
        op: &OperationRef<'c, '_>,
        frame: &mut Frame,
    ) -> Result<()> {
        match suffix {
            "constant" => {
                let attr = op.attribute("value").map_err(Error::from)?;
                let result_ty = op.result(0).map_err(Error::from)?.r#type();
                let value = parse_arith_const_value(attr, result_ty)?;
                frame.insert(op.result(0).map_err(Error::from)?.into(), value);
                Ok(())
            }
            "addi" | "subi" | "muli" | "divsi" | "divui" | "remsi" | "remui" | "ceildivsi"
            | "ceildivui" | "floordivsi" | "andi" | "ori" | "xori" | "shli" | "shrsi" | "shrui"
            | "maxsi" | "maxui" | "minsi" | "minui" => self.eval_arith_binary(suffix, op, frame),
            "cmpi" => self.eval_arith_cmpi(op, frame),
            "select" => self.eval_arith_select(op, frame),
            "extsi" | "extui" | "trunci" | "index_cast" | "index_castui" => {
                self.eval_arith_cast(suffix, op, frame)
            }
            other => Err(Error::UnsupportedOp(format!("arith.{other}"))),
        }
    }

    fn eval_arith_binary(
        &mut self,
        suffix: &str,
        op: &OperationRef<'c, '_>,
        frame: &mut Frame,
    ) -> Result<()> {
        let ops = operands(op)?;
        let lhs = frame
            .get(ops[0])
            .cloned()
            .ok_or_else(|| Error::MissingValue(format!("missing arith.{suffix} lhs")))?;
        let rhs = frame
            .get(ops[1])
            .cloned()
            .ok_or_else(|| Error::MissingValue(format!("missing arith.{suffix} rhs")))?;

        let value = match (&lhs, &rhs) {
            (Value::Index(a), Value::Index(b)) => {
                let result = arith_binary_index(suffix, *a, *b)?;
                Value::Index(result)
            }
            (Value::Int(a), Value::Int(b)) => {
                if a.width() != b.width() {
                    return Err(Error::TypeError(format!(
                        "arith.{suffix} operand width mismatch: i{} vs i{}",
                        a.width(),
                        b.width()
                    )));
                }
                Value::Int(arith_binary_int(suffix, a, b)?)
            }
            (Value::Bool(a), Value::Bool(b)) => match suffix {
                "andi" => Value::Bool(*a && *b),
                "ori" => Value::Bool(*a || *b),
                "xori" => Value::Bool(*a ^ *b),
                _ => {
                    return Err(Error::TypeError(format!(
                        "arith.{suffix} not supported on i1 operands"
                    )));
                }
            },
            _ => {
                return Err(Error::TypeError(format!(
                    "arith.{suffix} requires matching integer operands, got {lhs} and {rhs}"
                )));
            }
        };
        frame.insert(op.result(0).map_err(Error::from)?.into(), value);
        Ok(())
    }

    fn eval_arith_cmpi(&mut self, op: &OperationRef<'c, '_>, frame: &mut Frame) -> Result<()> {
        let predicate = parse_cmpi_predicate(op.attribute("predicate").map_err(Error::from)?)?;
        let ops = operands(op)?;
        let lhs = frame
            .get(ops[0])
            .cloned()
            .ok_or_else(|| Error::MissingValue("missing arith.cmpi lhs".into()))?;
        let rhs = frame
            .get(ops[1])
            .cloned()
            .ok_or_else(|| Error::MissingValue("missing arith.cmpi rhs".into()))?;

        let result = match (&lhs, &rhs) {
            (Value::Index(a), Value::Index(b)) => cmpi_unsigned(predicate, &BigInt::from(*a), &BigInt::from(*b), &BigUint::from(*a), &BigUint::from(*b)),
            (Value::Int(a), Value::Int(b)) => {
                if a.width() != b.width() {
                    return Err(Error::TypeError(format!(
                        "arith.cmpi operand width mismatch: i{} vs i{}",
                        a.width(),
                        b.width()
                    )));
                }
                cmpi_unsigned(predicate, &a.as_signed(), &b.as_signed(), a.as_unsigned(), b.as_unsigned())
            }
            (Value::Bool(a), Value::Bool(b)) => {
                // i1 signed interpretation: 1 -> -1, 0 -> 0.
                let av = if *a { BigInt::from(-1i64) } else { BigInt::from(0) };
                let bv = if *b { BigInt::from(-1i64) } else { BigInt::from(0) };
                let au = BigUint::from(*a as u64);
                let bu = BigUint::from(*b as u64);
                cmpi_unsigned(predicate, &av, &bv, &au, &bu)
            }
            _ => {
                return Err(Error::TypeError(format!(
                    "arith.cmpi requires matching integer operands, got {lhs} and {rhs}"
                )));
            }
        };
        frame.insert(
            op.result(0).map_err(Error::from)?.into(),
            Value::Bool(result),
        );
        Ok(())
    }

    fn eval_arith_select(&mut self, op: &OperationRef<'c, '_>, frame: &mut Frame) -> Result<()> {
        let ops = operands(op)?;
        let cond = frame
            .get(ops[0])
            .cloned()
            .ok_or_else(|| Error::MissingValue("missing arith.select condition".into()))?
            .as_bool()
            .map_err(Error::TypeError)?;
        let true_val = frame
            .get(ops[1])
            .cloned()
            .ok_or_else(|| Error::MissingValue("missing arith.select true value".into()))?;
        let false_val = frame
            .get(ops[2])
            .cloned()
            .ok_or_else(|| Error::MissingValue("missing arith.select false value".into()))?;
        frame.insert(
            op.result(0).map_err(Error::from)?.into(),
            if cond { true_val } else { false_val },
        );
        Ok(())
    }

    fn eval_arith_cast(
        &mut self,
        suffix: &str,
        op: &OperationRef<'c, '_>,
        frame: &mut Frame,
    ) -> Result<()> {
        let ops = operands(op)?;
        let operand = frame
            .get(ops[0])
            .cloned()
            .ok_or_else(|| Error::MissingValue(format!("missing arith.{suffix} operand")))?;
        let result_ty = op.result(0).map_err(Error::from)?.r#type();

        let value = match suffix {
            "extsi" => {
                let signed = value_as_signed(&operand)?;
                let int_ty = IntegerType::try_from(result_ty)
                    .map_err(|_| Error::TypeError("arith.extsi result must be integer".into()))?;
                let width = int_ty.width();
                if width == 1 {
                    Value::Bool(!signed.is_zero())
                } else {
                    Value::Int(IntValue::from_signed(signed, width))
                }
            }
            "extui" => {
                let unsigned = value_as_unsigned(&operand)?;
                let int_ty = IntegerType::try_from(result_ty)
                    .map_err(|_| Error::TypeError("arith.extui result must be integer".into()))?;
                let width = int_ty.width();
                if width == 1 {
                    Value::Bool(!unsigned.is_zero())
                } else {
                    Value::Int(IntValue::new(unsigned, width))
                }
            }
            "trunci" => {
                let unsigned = value_as_unsigned(&operand)?;
                let int_ty = IntegerType::try_from(result_ty)
                    .map_err(|_| Error::TypeError("arith.trunci result must be integer".into()))?;
                let width = int_ty.width();
                if width == 1 {
                    Value::Bool(!(unsigned & BigUint::one()).is_zero())
                } else {
                    Value::Int(IntValue::new(unsigned, width))
                }
            }
            "index_cast" => {
                if result_ty.is_index() {
                    let signed = value_as_signed(&operand)?;
                    let unsigned = signed
                        .to_biguint()
                        .ok_or_else(|| {
                            Error::TypeError(
                                "arith.index_cast cannot convert negative value to index".into(),
                            )
                        })?;
                    let idx: usize = unsigned.try_into().map_err(|_| {
                        Error::TypeError("arith.index_cast value too large for usize".into())
                    })?;
                    Value::Index(idx)
                } else {
                    let int_ty = IntegerType::try_from(result_ty).map_err(|_| {
                        Error::TypeError("arith.index_cast result must be index or int".into())
                    })?;
                    let signed = value_as_signed(&operand)?;
                    let width = int_ty.width();
                    if width == 1 {
                        Value::Bool(!signed.is_zero())
                    } else {
                        Value::Int(IntValue::from_signed(signed, width))
                    }
                }
            }
            "index_castui" => {
                if result_ty.is_index() {
                    let unsigned = value_as_unsigned(&operand)?;
                    let idx: usize = unsigned.try_into().map_err(|_| {
                        Error::TypeError("arith.index_castui value too large for usize".into())
                    })?;
                    Value::Index(idx)
                } else {
                    let int_ty = IntegerType::try_from(result_ty).map_err(|_| {
                        Error::TypeError("arith.index_castui result must be index or int".into())
                    })?;
                    let unsigned = value_as_unsigned(&operand)?;
                    let width = int_ty.width();
                    if width == 1 {
                        Value::Bool(!unsigned.is_zero())
                    } else {
                        Value::Int(IntValue::new(unsigned, width))
                    }
                }
            }
            _ => unreachable!(),
        };
        frame.insert(op.result(0).map_err(Error::from)?.into(), value);
        Ok(())
    }

    fn eval_op(&mut self, op: &OperationRef<'c, '_>, frame: &mut Frame) -> Result<()> {
        self.check_bool_assert_origin(op, frame)?;
        self.dispatch_op(op, frame)?;
        self.propagate_result_origins(op, frame);
        Ok(())
    }

    /// Enforces the PCL backend rule: in `@constrain` (and helpers transitively
    /// called from it), `bool.assert` is only lowered to a real polynomial
    /// constraint when its condition folds to a static constant. A dynamic
    /// condition silently disappears at proof time, which is a soundness gap.
    fn check_bool_assert_origin(
        &self,
        op: &OperationRef<'c, '_>,
        frame: &Frame,
    ) -> Result<()> {
        if self.phase != Phase::Constrain || !dialect::bool::is_bool_assert(op) {
            return Ok(());
        }
        let ops = operands(op)?;
        let cond = ops
            .first()
            .ok_or_else(|| Error::MalformedOp("bool.assert without operand".into()))?;
        if frame.origin(*cond) != Origin::Const {
            return Err(Error::ConstraintFailed(
                "bool.assert in @constrain has a dynamic condition; PCL will not lower it. \
                 Replace with `constrain.eq(cast.tofelt(<bool>), 1)`."
                    .into(),
            ));
        }
        Ok(())
    }

    /// Tags this op's result origins as `Const` iff all of its operands are
    /// `Const` and the op itself isn't an intrinsic source of dynamic data
    /// (`llzk.nondet`); known constant-producing ops are forced to `Const`.
    fn propagate_result_origins(&self, op: &OperationRef<'c, '_>, frame: &mut Frame) {
        let origin = if dialect::llzk::is_nondet(op) {
            Origin::Dynamic
        } else if is_const_producing_op(op) {
            Origin::Const
        } else {
            let mut all_const = true;
            for i in 0..op.operand_count() {
                let Ok(operand) = op.operand(i) else {
                    all_const = false;
                    break;
                };
                if frame.origin(operand) != Origin::Const {
                    all_const = false;
                    break;
                }
            }
            if all_const {
                Origin::Const
            } else {
                Origin::Dynamic
            }
        };
        for i in 0..op.result_count() {
            if let Ok(result) = op.result(i) {
                frame.set_origin(result.into(), origin);
            }
        }
    }

    fn dispatch_op(&mut self, op: &OperationRef<'c, '_>, frame: &mut Frame) -> Result<()> {
        if let Ok(name) = op.name().as_string_ref().as_str()
            && let Some(rest) = name.strip_prefix("arith.")
        {
            return self.eval_arith_op(rest, op, frame);
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
            let operand_values = operands(op)?;
            let args = operand_values
                .iter()
                .map(|operand| {
                    frame.get(*operand).cloned().ok_or_else(|| {
                        Error::MissingValue(format!("missing call operand {operand}"))
                    })
                })
                .collect::<Result<Vec<_>>>()?;
            let arg_origins: Vec<Origin> = operand_values
                .iter()
                .map(|operand| frame.origin(*operand))
                .collect();
            let callee = find_function(self.module, &symbol)?;
            // Functions marked `allow_witness` are treated by PCL as witness
            // oracles, not as constraint-emitting bodies, so `bool.assert`
            // inside them (the Brillig trap idiom) is legitimate even when
            // reached from `@constrain`.
            let callee_phase = if callee.has_allow_witness_attr() {
                Phase::Compute
            } else {
                self.phase
            };
            let prev_phase = std::mem::replace(&mut self.phase, callee_phase);
            let results = self.execute_function(&callee, &args, &arg_origins);
            self.phase = prev_phase;
            let results = results?;
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

/// Ops that produce values directly from their attributes (no SSA operands)
/// and therefore always count as `Origin::Const` regardless of context.
fn is_const_producing_op<'c, 'a>(op: &OperationRef<'c, 'a>) -> bool {
    if dialect::felt::is_felt_const(op) {
        return true;
    }
    let name = op.name();
    let name_ref = name.as_string_ref();
    let Ok(name_str) = name_ref.as_str() else {
        return false;
    };
    matches!(name_str, "arith.constant" | "index.constant")
}

fn value_as_signed(value: &Value) -> Result<BigInt> {
    match value {
        Value::Index(v) => Ok(BigInt::from(*v)),
        Value::Int(v) => Ok(v.as_signed()),
        // i1 signed interpretation: true is -1, false is 0.
        Value::Bool(b) => Ok(if *b { BigInt::from(-1i64) } else { BigInt::zero() }),
        other => Err(Error::TypeError(format!(
            "expected integer-like value, got {other}"
        ))),
    }
}

fn value_as_unsigned(value: &Value) -> Result<BigUint> {
    match value {
        Value::Index(v) => Ok(BigUint::from(*v)),
        Value::Int(v) => Ok(v.as_unsigned().clone()),
        Value::Bool(b) => Ok(BigUint::from(*b as u64)),
        other => Err(Error::TypeError(format!(
            "expected integer-like value, got {other}"
        ))),
    }
}

fn arith_binary_index(suffix: &str, a: usize, b: usize) -> Result<usize> {
    let result = match suffix {
        "addi" => a.wrapping_add(b),
        "subi" => a.wrapping_sub(b),
        "muli" => a.wrapping_mul(b),
        "divsi" | "divui" => {
            if b == 0 {
                return Err(Error::ConstraintFailed(format!("arith.{suffix} by zero")));
            }
            a / b
        }
        "remsi" | "remui" => {
            if b == 0 {
                return Err(Error::ConstraintFailed(format!("arith.{suffix} by zero")));
            }
            a % b
        }
        "ceildivsi" | "ceildivui" => {
            if b == 0 {
                return Err(Error::ConstraintFailed(format!("arith.{suffix} by zero")));
            }
            a.div_ceil(b)
        }
        "floordivsi" => {
            if b == 0 {
                return Err(Error::ConstraintFailed("arith.floordivsi by zero".into()));
            }
            a / b
        }
        "andi" => a & b,
        "ori" => a | b,
        "xori" => a ^ b,
        "shli" => {
            u32::try_from(b)
                .ok()
                .and_then(|amt| a.checked_shl(amt))
                .unwrap_or(0)
        }
        "shrsi" | "shrui" => {
            u32::try_from(b)
                .ok()
                .and_then(|amt| a.checked_shr(amt))
                .unwrap_or(0)
        }
        "maxsi" | "maxui" => a.max(b),
        "minsi" | "minui" => a.min(b),
        _ => {
            return Err(Error::UnsupportedOp(format!("arith.{suffix} on index")));
        }
    };
    Ok(result)
}

fn arith_binary_int(suffix: &str, a: &IntValue, b: &IntValue) -> Result<IntValue> {
    let width = a.width();
    match suffix {
        "addi" => Ok(IntValue::new(a.as_unsigned() + b.as_unsigned(), width)),
        "subi" => Ok(IntValue::from_signed(a.as_signed() - b.as_signed(), width)),
        "muli" => Ok(IntValue::new(a.as_unsigned() * b.as_unsigned(), width)),
        "divsi" => {
            let bv = b.as_signed();
            if bv.is_zero() {
                return Err(Error::ConstraintFailed("arith.divsi by zero".into()));
            }
            // Truncated division (toward zero), per LLVM semantics for sdiv.
            Ok(IntValue::from_signed(a.as_signed() / bv, width))
        }
        "divui" => {
            let bv = b.as_unsigned();
            if bv.is_zero() {
                return Err(Error::ConstraintFailed("arith.divui by zero".into()));
            }
            Ok(IntValue::new(a.as_unsigned() / bv, width))
        }
        "remsi" => {
            let bv = b.as_signed();
            if bv.is_zero() {
                return Err(Error::ConstraintFailed("arith.remsi by zero".into()));
            }
            // Truncated remainder (sign of dividend).
            Ok(IntValue::from_signed(a.as_signed() % bv, width))
        }
        "remui" => {
            let bv = b.as_unsigned();
            if bv.is_zero() {
                return Err(Error::ConstraintFailed("arith.remui by zero".into()));
            }
            Ok(IntValue::new(a.as_unsigned() % bv, width))
        }
        "ceildivsi" => {
            let bv = b.as_signed();
            if bv.is_zero() {
                return Err(Error::ConstraintFailed("arith.ceildivsi by zero".into()));
            }
            let av = a.as_signed();
            let q = &av / &bv;
            let r = &av % &bv;
            // Round toward +inf.
            let adjusted = if !r.is_zero()
                && ((av.sign() == num_bigint::Sign::Plus)
                    == (bv.sign() == num_bigint::Sign::Plus))
            {
                q + BigInt::one()
            } else {
                q
            };
            Ok(IntValue::from_signed(adjusted, width))
        }
        "ceildivui" => {
            let bv = b.as_unsigned();
            if bv.is_zero() {
                return Err(Error::ConstraintFailed("arith.ceildivui by zero".into()));
            }
            let au = a.as_unsigned();
            let q = au / bv;
            let r = au % bv;
            let adjusted = if r.is_zero() { q } else { q + BigUint::one() };
            Ok(IntValue::new(adjusted, width))
        }
        "floordivsi" => {
            let bv = b.as_signed();
            if bv.is_zero() {
                return Err(Error::ConstraintFailed("arith.floordivsi by zero".into()));
            }
            let av = a.as_signed();
            let q = &av / &bv;
            let r = &av % &bv;
            // Round toward -inf.
            let adjusted = if !r.is_zero()
                && ((av.sign() == num_bigint::Sign::Minus)
                    != (bv.sign() == num_bigint::Sign::Minus))
            {
                q - BigInt::one()
            } else {
                q
            };
            Ok(IntValue::from_signed(adjusted, width))
        }
        "andi" => Ok(IntValue::new(a.as_unsigned() & b.as_unsigned(), width)),
        "ori" => Ok(IntValue::new(a.as_unsigned() | b.as_unsigned(), width)),
        "xori" => Ok(IntValue::new(a.as_unsigned() ^ b.as_unsigned(), width)),
        "shli" => {
            let amount: u32 = b
                .as_unsigned()
                .try_into()
                .map_err(|_| Error::MalformedOp("arith.shli amount out of range".into()))?;
            if amount >= width {
                Ok(IntValue::new(BigUint::zero(), width))
            } else {
                Ok(IntValue::new(a.as_unsigned() << amount, width))
            }
        }
        "shrui" => {
            let amount: u32 = b
                .as_unsigned()
                .try_into()
                .map_err(|_| Error::MalformedOp("arith.shrui amount out of range".into()))?;
            if amount >= width {
                Ok(IntValue::new(BigUint::zero(), width))
            } else {
                Ok(IntValue::new(a.as_unsigned() >> amount, width))
            }
        }
        "shrsi" => {
            let amount: u32 = b
                .as_unsigned()
                .try_into()
                .map_err(|_| Error::MalformedOp("arith.shrsi amount out of range".into()))?;
            let signed = a.as_signed();
            if amount >= width {
                let extended = if signed.sign() == num_bigint::Sign::Minus {
                    BigInt::from(-1i64)
                } else {
                    BigInt::zero()
                };
                Ok(IntValue::from_signed(extended, width))
            } else {
                // Arithmetic shift right is floor-divide by 2^amount.
                let divisor = BigInt::one() << amount;
                let q = floor_div_bigint(&signed, &divisor);
                Ok(IntValue::from_signed(q, width))
            }
        }
        "maxsi" => Ok(IntValue::from_signed(a.as_signed().max(b.as_signed()), width)),
        "minsi" => Ok(IntValue::from_signed(a.as_signed().min(b.as_signed()), width)),
        "maxui" => Ok(IntValue::new(
            a.as_unsigned().max(b.as_unsigned()).clone(),
            width,
        )),
        "minui" => Ok(IntValue::new(
            a.as_unsigned().min(b.as_unsigned()).clone(),
            width,
        )),
        _ => Err(Error::UnsupportedOp(format!("arith.{suffix}"))),
    }
}

fn cmpi_unsigned(
    predicate: CmpIPredicate,
    lhs_signed: &BigInt,
    rhs_signed: &BigInt,
    lhs_unsigned: &BigUint,
    rhs_unsigned: &BigUint,
) -> bool {
    match predicate {
        CmpIPredicate::Eq => lhs_unsigned == rhs_unsigned,
        CmpIPredicate::Ne => lhs_unsigned != rhs_unsigned,
        CmpIPredicate::Slt => lhs_signed < rhs_signed,
        CmpIPredicate::Sle => lhs_signed <= rhs_signed,
        CmpIPredicate::Sgt => lhs_signed > rhs_signed,
        CmpIPredicate::Sge => lhs_signed >= rhs_signed,
        CmpIPredicate::Ult => lhs_unsigned < rhs_unsigned,
        CmpIPredicate::Ule => lhs_unsigned <= rhs_unsigned,
        CmpIPredicate::Ugt => lhs_unsigned > rhs_unsigned,
        CmpIPredicate::Uge => lhs_unsigned >= rhs_unsigned,
    }
}

fn floor_div_bigint(dividend: &BigInt, divisor: &BigInt) -> BigInt {
    let q = dividend / divisor;
    let r = dividend % divisor;
    if !r.is_zero()
        && ((dividend.sign() == num_bigint::Sign::Minus)
            != (divisor.sign() == num_bigint::Sign::Minus))
    {
        q - BigInt::one()
    } else {
        q
    }
}
