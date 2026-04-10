use llzk::prelude::{
    Block, BlockLike, LlzkContext, Module, OperationLike, RegionLike, Type, dialect, llzk_module,
    melior_dialects::arith,
};
use llzk::{
    builder::OpBuilder,
    dialect::array::{ArrayCtor, ArrayType},
    prelude::{FeltConstAttribute, FeltType},
};
use llzk_interpreter::{Felt, Interpreter, StructInstance, Value};
use melior::ir::{Location, attribute::IntegerAttribute, r#type::FunctionType};

#[test]
fn interprets_handwritten_arithmetic_function() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @double(%x: !felt.type<"bn254">) -> !felt.type<"bn254"> {
    %c = felt.const 2 <"bn254">
    %r = felt.mul %x, %c : !felt.type<"bn254">, !felt.type<"bn254">
    function.return %r : !felt.type<"bn254">
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let result = interpreter
        .run_function("@double", &[Value::Felt(Felt::from_u64(7))])
        .expect("function should run");

    assert_eq!(result, vec![Value::Felt(Felt::from_u64(14))]);
}

#[test]
fn interprets_handwritten_struct_compute_and_constrain() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  struct.def @Box {
    struct.member @x : !felt.type<"bn254"> {llzk.pub}
    function.def @compute(%a: !felt.type<"bn254">) -> !struct.type<@Box> {
      %self = struct.new : !struct.type<@Box>
      struct.writem %self[@x] = %a : !struct.type<@Box>, !felt.type<"bn254">
      function.return %self : !struct.type<@Box>
    }
    function.def @constrain(%self: !struct.type<@Box>, %a: !felt.type<"bn254">) {
      %x = struct.readm %self[@x] : !struct.type<@Box>, !felt.type<"bn254">
      constrain.eq %x, %a : !felt.type<"bn254">
      function.return
    }
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let computed = interpreter
        .run_compute("Box", &[Value::Felt(Felt::from_u64(9))])
        .expect("compute should succeed");
    assert_eq!(
        computed.members.get("x"),
        Some(&Value::Felt(Felt::from_u64(9)))
    );

    interpreter
        .run_constrain("Box", computed, &[Value::Felt(Felt::from_u64(9))])
        .expect("constrain should succeed");
    assert_eq!(interpreter.state().constraints.len(), 1);
}

#[test]
fn interprets_struct_memory_compute_and_constrain() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  struct.def @MemBox {
    struct.member @arr : !array.type<2 x !felt.type<"bn254">>
    function.def @compute(%a: !felt.type<"bn254">, %b: !felt.type<"bn254">) -> !struct.type<@MemBox> {
      %self = struct.new : !struct.type<@MemBox>
      %c0 = arith.constant 0 : index
      %c1 = arith.constant 1 : index
      %array = array.new  : <2 x !felt.type<"bn254">>
      array.write %array[%c0] = %a : <2 x !felt.type<"bn254">>, !felt.type<"bn254">
      array.write %array[%c1] = %b : <2 x !felt.type<"bn254">>, !felt.type<"bn254">
      struct.writem %self[@arr] = %array : !struct.type<@MemBox>, !array.type<2 x !felt.type<"bn254">>
      function.return %self : !struct.type<@MemBox>
    }
    function.def @constrain(%self: !struct.type<@MemBox>, %a: !felt.type<"bn254">, %b: !felt.type<"bn254">) {
      %c0 = arith.constant 0 : index
      %c1 = arith.constant 1 : index
      %array = struct.readm %self[@arr] : !struct.type<@MemBox>, !array.type<2 x !felt.type<"bn254">>
      %lhs = array.read %array[%c0] : <2 x !felt.type<"bn254">>, !felt.type<"bn254">
      %rhs = array.read %array[%c1] : <2 x !felt.type<"bn254">>, !felt.type<"bn254">
      constrain.eq %lhs, %a : !felt.type<"bn254">
      constrain.eq %rhs, %b : !felt.type<"bn254">
      function.return
    }
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let computed = interpreter
        .run_compute(
            "MemBox",
            &[
                Value::Felt(Felt::from_u64(3)),
                Value::Felt(Felt::from_u64(8)),
            ],
        )
        .expect("compute should succeed");

    interpreter
        .run_constrain(
            "MemBox",
            computed,
            &[
                Value::Felt(Felt::from_u64(3)),
                Value::Felt(Felt::from_u64(8)),
            ],
        )
        .expect("constrain should succeed");
    assert_eq!(interpreter.state().constraints.len(), 2);
}

#[test]
fn fails_on_struct_memory_constrain_mismatch() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  struct.def @MemBox {
    struct.member @arr : !array.type<2 x !felt.type<"bn254">>
    function.def @compute(%a: !felt.type<"bn254">, %b: !felt.type<"bn254">) -> !struct.type<@MemBox> {
      %self = struct.new : !struct.type<@MemBox>
      %c0 = arith.constant 0 : index
      %c1 = arith.constant 1 : index
      %array = array.new  : <2 x !felt.type<"bn254">>
      array.write %array[%c0] = %a : <2 x !felt.type<"bn254">>, !felt.type<"bn254">
      array.write %array[%c1] = %b : <2 x !felt.type<"bn254">>, !felt.type<"bn254">
      struct.writem %self[@arr] = %array : !struct.type<@MemBox>, !array.type<2 x !felt.type<"bn254">>
      function.return %self : !struct.type<@MemBox>
    }
    function.def @constrain(%self: !struct.type<@MemBox>, %a: !felt.type<"bn254">, %b: !felt.type<"bn254">) {
      %c0 = arith.constant 0 : index
      %c1 = arith.constant 1 : index
      %array = struct.readm %self[@arr] : !struct.type<@MemBox>, !array.type<2 x !felt.type<"bn254">>
      %lhs = array.read %array[%c0] : <2 x !felt.type<"bn254">>, !felt.type<"bn254">
      %rhs = array.read %array[%c1] : <2 x !felt.type<"bn254">>, !felt.type<"bn254">
      constrain.eq %lhs, %a : !felt.type<"bn254">
      constrain.eq %rhs, %b : !felt.type<"bn254">
      function.return
    }
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let computed = interpreter
        .run_compute(
            "MemBox",
            &[
                Value::Felt(Felt::from_u64(3)),
                Value::Felt(Felt::from_u64(8)),
            ],
        )
        .expect("compute should succeed");

    let err = interpreter
        .run_constrain(
            "MemBox",
            computed,
            &[
                Value::Felt(Felt::from_u64(3)),
                Value::Felt(Felt::from_u64(9)),
            ],
        )
        .expect_err("constrain should fail");
    assert!(err.to_string().contains("!="));
}

#[test]
fn interprets_handwritten_nested_function_call() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @double(%x: !felt.type<"bn254">) -> !felt.type<"bn254"> {
    %c = felt.const 2 <"bn254">
    %r = felt.mul %x, %c : !felt.type<"bn254">, !felt.type<"bn254">
    function.return %r : !felt.type<"bn254">
  }
  function.def @quadruple(%x: !felt.type<"bn254">) -> !felt.type<"bn254"> {
    %y = function.call @double(%x) : (!felt.type<"bn254">) -> !felt.type<"bn254">
    %z = function.call @double(%y) : (!felt.type<"bn254">) -> !felt.type<"bn254">
    function.return %z : !felt.type<"bn254">
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let result = interpreter
        .run_function("@quadruple", &[Value::Felt(Felt::from_u64(3))])
        .expect("function should run");

    assert_eq!(result, vec![Value::Felt(Felt::from_u64(12))]);
}

#[test]
fn fails_on_constrain_eq_mismatch() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  struct.def @Box {
    struct.member @x : !felt.type<"bn254"> {llzk.pub}
    function.def @compute(%a: !felt.type<"bn254">) -> !struct.type<@Box> {
      %self = struct.new : !struct.type<@Box>
      struct.writem %self[@x] = %a : !struct.type<@Box>, !felt.type<"bn254">
      function.return %self : !struct.type<@Box>
    }
    function.def @constrain(%self: !struct.type<@Box>, %a: !felt.type<"bn254">) {
      %x = struct.readm %self[@x] : !struct.type<@Box>, !felt.type<"bn254">
      constrain.eq %x, %a : !felt.type<"bn254">
      function.return
    }
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let err = interpreter
        .run_constrain(
            "Box",
            StructInstance {
                type_name: "Box".to_string(),
                members: [("x".to_string(), Value::Felt(Felt::from_u64(1)))]
                    .into_iter()
                    .collect(),
            },
            &[Value::Felt(Felt::from_u64(2))],
        )
        .expect_err("constraint should fail");

    assert!(err.to_string().contains("!="));
}

#[test]
fn interprets_handwritten_array_write_and_read() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @pick_second(%x: !felt.type<"bn254">, %y: !felt.type<"bn254">) -> !felt.type<"bn254"> {
    %c0 = arith.constant 0 : index
    %c1 = arith.constant 1 : index
    %array = array.new  : <2 x !felt.type<"bn254">>
    array.write %array[%c0] = %x : <2 x !felt.type<"bn254">>, !felt.type<"bn254">
    array.write %array[%c1] = %y : <2 x !felt.type<"bn254">>, !felt.type<"bn254">
    %result = array.read %array[%c1] : <2 x !felt.type<"bn254">>, !felt.type<"bn254">
    function.return %result : !felt.type<"bn254">
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let result = interpreter
        .run_function(
            "@pick_second",
            &[
                Value::Felt(Felt::from_u64(7)),
                Value::Felt(Felt::from_u64(11)),
            ],
        )
        .expect("function should run");

    assert_eq!(result, vec![Value::Felt(Felt::from_u64(11))]);
}

#[test]
fn interprets_last_array_write_wins() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @overwrite(%x: !felt.type<"bn254">, %y: !felt.type<"bn254">) -> !felt.type<"bn254"> {
    %c0 = arith.constant 0 : index
    %array = array.new  : <1 x !felt.type<"bn254">>
    array.write %array[%c0] = %x : <1 x !felt.type<"bn254">>, !felt.type<"bn254">
    array.write %array[%c0] = %y : <1 x !felt.type<"bn254">>, !felt.type<"bn254">
    %result = array.read %array[%c0] : <1 x !felt.type<"bn254">>, !felt.type<"bn254">
    function.return %result : !felt.type<"bn254">
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let result = interpreter
        .run_function(
            "@overwrite",
            &[
                Value::Felt(Felt::from_u64(5)),
                Value::Felt(Felt::from_u64(9)),
            ],
        )
        .expect("function should run");

    assert_eq!(result, vec![Value::Felt(Felt::from_u64(9))]);
}

#[test]
fn arrays_keep_independent_state() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @sum_heads(%x: !felt.type<"bn254">, %y: !felt.type<"bn254">) -> !felt.type<"bn254"> {
    %c0 = arith.constant 0 : index
    %left = array.new  : <1 x !felt.type<"bn254">>
    %right = array.new  : <1 x !felt.type<"bn254">>
    array.write %left[%c0] = %x : <1 x !felt.type<"bn254">>, !felt.type<"bn254">
    array.write %right[%c0] = %y : <1 x !felt.type<"bn254">>, !felt.type<"bn254">
    %lhs = array.read %left[%c0] : <1 x !felt.type<"bn254">>, !felt.type<"bn254">
    %rhs = array.read %right[%c0] : <1 x !felt.type<"bn254">>, !felt.type<"bn254">
    %sum = felt.add %lhs, %rhs : !felt.type<"bn254">, !felt.type<"bn254">
    function.return %sum : !felt.type<"bn254">
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let result = interpreter
        .run_function(
            "@sum_heads",
            &[
                Value::Felt(Felt::from_u64(4)),
                Value::Felt(Felt::from_u64(7)),
            ],
        )
        .expect("function should run");

    assert_eq!(result, vec![Value::Felt(Felt::from_u64(11))]);
}

#[test]
fn fails_on_reading_uninitialized_array_element() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @missing() -> !felt.type<"bn254"> {
    %c0 = arith.constant 0 : index
    %array = array.new  : <1 x !felt.type<"bn254">>
    %result = array.read %array[%c0] : <1 x !felt.type<"bn254">>, !felt.type<"bn254">
    function.return %result : !felt.type<"bn254">
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let err = interpreter
        .run_function("@missing", &[])
        .expect_err("read should fail for missing array slot");

    assert!(err.to_string().contains("missing array element"));
}

#[test]
fn interprets_handwritten_cast_toindex_for_array_read() {
    let context = LlzkContext::new();
    let loc = Location::unknown(&context);
    let module = llzk_module(loc);
    let felt_type: Type<'_> = FeltType::new(&context).into();
    let index_type = Type::index(&context);
    let function = dialect::function::def(
        loc,
        "lookup",
        FunctionType::new(&context, &[felt_type], &[felt_type]),
        &[],
        None,
    )
    .expect("function should build");
    {
        let block = Block::new(&[(felt_type, loc)]);
        let builder = OpBuilder::new(&context);
        let idx: llzk::prelude::Value<'_, '_> = block.argument(0).expect("arg").into();

        let c0 = block.append_operation(arith::constant(
            &context,
            IntegerAttribute::new(index_type, 0).into(),
            loc,
        ));
        let c1 = block.append_operation(arith::constant(
            &context,
            IntegerAttribute::new(index_type, 1).into(),
            loc,
        ));
        let c2 = block.append_operation(arith::constant(
            &context,
            IntegerAttribute::new(index_type, 2).into(),
            loc,
        ));

        let a = block.append_operation(
            dialect::felt::constant(loc, FeltConstAttribute::new(&context, 10, None))
                .expect("felt const"),
        );
        let b = block.append_operation(
            dialect::felt::constant(loc, FeltConstAttribute::new(&context, 20, None))
                .expect("felt const"),
        );
        let c = block.append_operation(
            dialect::felt::constant(loc, FeltConstAttribute::new(&context, 30, None))
                .expect("felt const"),
        );

        let array = block.append_operation(dialect::array::new(
            &builder,
            loc,
            ArrayType::new_with_dims(felt_type, &[3]),
            ArrayCtor::Empty,
        ));
        block.append_operation(dialect::array::write(
            loc,
            array.result(0).expect("array result").into(),
            &[c0.result(0).expect("c0 result").into()],
            a.result(0).expect("a result").into(),
        ));
        block.append_operation(dialect::array::write(
            loc,
            array.result(0).expect("array result").into(),
            &[c1.result(0).expect("c1 result").into()],
            b.result(0).expect("b result").into(),
        ));
        block.append_operation(dialect::array::write(
            loc,
            array.result(0).expect("array result").into(),
            &[c2.result(0).expect("c2 result").into()],
            c.result(0).expect("c result").into(),
        ));
        let cast = block.append_operation(dialect::cast::toindex(loc, idx));
        let result = block.append_operation(dialect::array::read(
            loc,
            felt_type,
            array.result(0).expect("array result").into(),
            &[cast.result(0).expect("cast result").into()],
        ));
        block.append_operation(dialect::function::r#return(
            loc,
            &[result.result(0).expect("read result").into()],
        ));
        function
            .region(0)
            .expect("function region")
            .append_block(block);
    }
    module.body().append_operation(function.into());

    let mut interpreter = Interpreter::new(&module);
    let result = interpreter
        .run_function("@lookup", &[Value::Felt(Felt::from_u64(2))])
        .expect("function should run");

    assert_eq!(result, vec![Value::Felt(Felt::from_u64(30))]);
}

#[test]
fn interprets_handwritten_bool_eq_on_felts() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @felt_eq(%x: !felt.type<"bn254">, %y: !felt.type<"bn254">) -> i1 {
    %is_eq = bool.cmp eq(%x, %y) : !felt.type<"bn254">, !felt.type<"bn254">
    function.return %is_eq : i1
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let equal = interpreter
        .run_function(
            "@felt_eq",
            &[
                Value::Felt(Felt::from_u64(5)),
                Value::Felt(Felt::from_u64(5)),
            ],
        )
        .expect("function should run");
    let different = interpreter
        .run_function(
            "@felt_eq",
            &[
                Value::Felt(Felt::from_u64(5)),
                Value::Felt(Felt::from_u64(6)),
            ],
        )
        .expect("function should run");

    assert_eq!(equal, vec![Value::Bool(true)]);
    assert_eq!(different, vec![Value::Bool(false)]);
}

#[test]
fn nondet_consumes_pre_supplied_values_in_order() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @sum_two_nondet() -> !felt.type<"bn254"> {
    %a = llzk.nondet : !felt.type<"bn254">
    %b = llzk.nondet : !felt.type<"bn254">
    %r = felt.add %a, %b : !felt.type<"bn254">, !felt.type<"bn254">
    function.return %r : !felt.type<"bn254">
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    interpreter.set_nondet_values([Felt::from_u64(7), Felt::from_u64(11)]);
    let result = interpreter
        .run_function("@sum_two_nondet", &[])
        .expect("function should run");

    assert_eq!(result, vec![Value::Felt(Felt::from_u64(18))]);
}

#[test]
fn nondet_falls_back_to_zero_when_queue_is_empty() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @nondet_then_add_one() -> !felt.type<"bn254"> {
    %x = llzk.nondet : !felt.type<"bn254">
    %one = felt.const 1 <"bn254">
    %r = felt.add %x, %one : !felt.type<"bn254">, !felt.type<"bn254">
    function.return %r : !felt.type<"bn254">
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    // No values supplied — nondet should default to zero.
    let result = interpreter
        .run_function("@nondet_then_add_one", &[])
        .expect("function should run");

    assert_eq!(result, vec![Value::Felt(Felt::from_u64(1))]);
}

#[test]
fn nondet_partial_queue_drains_then_falls_back_to_zero() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @three_nondet() -> !felt.type<"bn254"> {
    %a = llzk.nondet : !felt.type<"bn254">
    %b = llzk.nondet : !felt.type<"bn254">
    %c = llzk.nondet : !felt.type<"bn254">
    %s1 = felt.add %a, %b : !felt.type<"bn254">, !felt.type<"bn254">
    %s2 = felt.add %s1, %c : !felt.type<"bn254">, !felt.type<"bn254">
    function.return %s2 : !felt.type<"bn254">
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    // Only two values for three nondet ops; the third should default to zero.
    interpreter.set_nondet_values([Felt::from_u64(5), Felt::from_u64(8)]);
    let result = interpreter
        .run_function("@three_nondet", &[])
        .expect("function should run");

    assert_eq!(result, vec![Value::Felt(Felt::from_u64(13))]);
}
