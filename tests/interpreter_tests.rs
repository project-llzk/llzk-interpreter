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

#[test]
fn interprets_scf_while_sum_first_n() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @sum_first_n(%n: !felt.type<"bn254">) -> !felt.type<"bn254"> {
    %zero = felt.const 0 <"bn254">
    %one = felt.const 1 <"bn254">
    %res:2 = scf.while (%i = %zero, %s = %zero) : (!felt.type<"bn254">, !felt.type<"bn254">) -> (!felt.type<"bn254">, !felt.type<"bn254">) {
      %cond = bool.cmp lt(%i, %n) : !felt.type<"bn254">, !felt.type<"bn254">
      scf.condition(%cond) %i, %s : !felt.type<"bn254">, !felt.type<"bn254">
    } do {
    ^bb0(%i_in: !felt.type<"bn254">, %s_in: !felt.type<"bn254">):
      %i_next = felt.add %i_in, %one : !felt.type<"bn254">, !felt.type<"bn254">
      %s_next = felt.add %s_in, %i_next : !felt.type<"bn254">, !felt.type<"bn254">
      scf.yield %i_next, %s_next : !felt.type<"bn254">, !felt.type<"bn254">
    }
    function.return %res#1 : !felt.type<"bn254">
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let result = interpreter
        .run_function("@sum_first_n", &[Value::Felt(Felt::from_u64(4))])
        .expect("function should run");

    // 1 + 2 + 3 + 4 = 10
    assert_eq!(result, vec![Value::Felt(Felt::from_u64(10))]);
}

#[test]
fn interprets_scf_while_zero_iterations() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @loop_zero() -> !felt.type<"bn254"> {
    %zero = felt.const 0 <"bn254">
    %one = felt.const 1 <"bn254">
    %res = scf.while (%s = %zero) : (!felt.type<"bn254">) -> !felt.type<"bn254"> {
      %cond = bool.cmp lt(%s, %zero) : !felt.type<"bn254">, !felt.type<"bn254">
      scf.condition(%cond) %s : !felt.type<"bn254">
    } do {
    ^bb0(%s_in: !felt.type<"bn254">):
      %s_next = felt.add %s_in, %one : !felt.type<"bn254">, !felt.type<"bn254">
      scf.yield %s_next : !felt.type<"bn254">
    }
    function.return %res : !felt.type<"bn254">
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let result = interpreter
        .run_function("@loop_zero", &[])
        .expect("function should run");

    assert_eq!(result, vec![Value::Felt(Felt::from_u64(0))]);
}

#[test]
fn interprets_cast_tofelt_from_index() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @index_to_felt() -> !felt.type<"bn254"> {
    %i = arith.constant 7 : index
    %f = cast.tofelt %i : index, !felt.type<"bn254">
    %one = felt.const 1 <"bn254">
    %r = felt.add %f, %one : !felt.type<"bn254">, !felt.type<"bn254">
    function.return %r : !felt.type<"bn254">
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let result = interpreter
        .run_function("@index_to_felt", &[])
        .expect("function should run");

    assert_eq!(result, vec![Value::Felt(Felt::from_u64(8))]);
}

fn run_int_function(ir: &str, name: &str, args: &[Value]) -> Vec<Value> {
    let context = LlzkContext::new();
    let module = Module::parse(&context, ir).unwrap_or_else(|| {
        panic!("module should parse:\n----\n{}\n----", ir);
    });
    let mut interpreter = Interpreter::new(&module);
    interpreter
        .run_function(name, args)
        .expect("function should run")
}

#[test]
fn arith_index_basic_arithmetic() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @idx() -> index {
    %a = arith.constant 10 : index
    %b = arith.constant 3 : index
    %s = arith.addi %a, %b : index
    %m = arith.muli %s, %b : index
    %d = arith.subi %m, %a : index
    function.return %d : index
  }
}
"#,
        "@idx",
        &[],
    );
    // (10 + 3) * 3 - 10 = 39 - 10 = 29
    assert_eq!(result, vec![Value::Index(29)]);
}

#[test]
fn arith_index_division_and_remainder() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @divs() -> (index, index, index, index) {
    %a = arith.constant 17 : index
    %b = arith.constant 5 : index
    %dv = arith.divui %a, %b : index
    %rm = arith.remui %a, %b : index
    %ce = arith.ceildivui %a, %b : index
    %fl = arith.floordivsi %a, %b : index
    function.return %dv, %rm, %ce, %fl : index, index, index, index
  }
}
"#,
        "@divs",
        &[],
    );
    assert_eq!(
        result,
        vec![
            Value::Index(3), // 17 / 5
            Value::Index(2), // 17 % 5
            Value::Index(4), // ceil(17/5)
            Value::Index(3), // floor(17/5)
        ]
    );
}

#[test]
fn arith_index_bitwise_and_shifts() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @bits() -> (index, index, index, index, index) {
    %a = arith.constant 12 : index
    %b = arith.constant 10 : index
    %two = arith.constant 2 : index
    %and = arith.andi %a, %b : index
    %or  = arith.ori  %a, %b : index
    %xor = arith.xori %a, %b : index
    %shl = arith.shli %a, %two : index
    %shr = arith.shrui %a, %two : index
    function.return %and, %or, %xor, %shl, %shr : index, index, index, index, index
  }
}
"#,
        "@bits",
        &[],
    );
    assert_eq!(
        result,
        vec![
            Value::Index(8),  // 12 & 10
            Value::Index(14), // 12 | 10
            Value::Index(6),  // 12 ^ 10
            Value::Index(48), // 12 << 2
            Value::Index(3),  // 12 >> 2
        ]
    );
}

#[test]
fn arith_index_min_max() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @mm() -> (index, index) {
    %a = arith.constant 7 : index
    %b = arith.constant 3 : index
    %mx = arith.maxui %a, %b : index
    %mn = arith.minui %a, %b : index
    function.return %mx, %mn : index, index
  }
}
"#,
        "@mm",
        &[],
    );
    assert_eq!(result, vec![Value::Index(7), Value::Index(3)]);
}

#[test]
fn arith_select_picks_branch_by_condition() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @sel(%c: i1) -> index {
    %a = arith.constant 100 : index
    %b = arith.constant 200 : index
    %r = arith.select %c, %a, %b : index
    function.return %r : index
  }
}
"#,
        "@sel",
        &[Value::Bool(true)],
    );
    assert_eq!(result, vec![Value::Index(100)]);

    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @sel(%c: i1) -> index {
    %a = arith.constant 100 : index
    %b = arith.constant 200 : index
    %r = arith.select %c, %a, %b : index
    function.return %r : index
  }
}
"#,
        "@sel",
        &[Value::Bool(false)],
    );
    assert_eq!(result, vec![Value::Index(200)]);
}

#[test]
fn arith_cmpi_unsigned_on_index() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @cmps() -> (i1, i1, i1, i1, i1) {
    %a = arith.constant 5 : index
    %b = arith.constant 10 : index
    %eq = arith.cmpi eq, %a, %b : index
    %ne = arith.cmpi ne, %a, %b : index
    %lt = arith.cmpi ult, %a, %b : index
    %le = arith.cmpi ule, %a, %a : index
    %gt = arith.cmpi ugt, %b, %a : index
    function.return %eq, %ne, %lt, %le, %gt : i1, i1, i1, i1, i1
  }
}
"#,
        "@cmps",
        &[],
    );
    assert_eq!(
        result,
        vec![
            Value::Bool(false),
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
            Value::Bool(true),
        ]
    );
}

#[test]
fn arith_constant_negative_i32_internal() {
    // Wider integers are not allowed at function boundaries, so we must
    // observe them indirectly. Here we form -1 : i32 and compare with itself.
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @neg() -> i1 {
    %x = arith.constant -1 : i32
    %y = arith.constant 4294967295 : i32
    %eq = arith.cmpi eq, %x, %y : i32
    function.return %eq : i1
  }
}
"#,
        "@neg",
        &[],
    );
    // -1 (i32) and 4294967295 (i32) share the same bit pattern.
    assert_eq!(result, vec![Value::Bool(true)]);
}

#[test]
fn arith_signed_vs_unsigned_compare_on_i32() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @sv() -> (i1, i1) {
    %a = arith.constant -1 : i32
    %b = arith.constant 1 : i32
    %slt = arith.cmpi slt, %a, %b : i32
    %ult = arith.cmpi ult, %a, %b : i32
    function.return %slt, %ult : i1, i1
  }
}
"#,
        "@sv",
        &[],
    );
    // -1 < 1 signed = true, but 0xFFFFFFFF < 1 unsigned = false.
    assert_eq!(result, vec![Value::Bool(true), Value::Bool(false)]);
}

#[test]
fn arith_addi_overflow_wraps_i8() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @wrap() -> i1 {
    %a = arith.constant 127 : i8
    %one = arith.constant 1 : i8
    %sum = arith.addi %a, %one : i8
    %expected = arith.constant -128 : i8
    %eq = arith.cmpi eq, %sum, %expected : i8
    function.return %eq : i1
  }
}
"#,
        "@wrap",
        &[],
    );
    // 127 + 1 in i8 wraps to -128.
    assert_eq!(result, vec![Value::Bool(true)]);
}

#[test]
fn arith_signed_division_truncates_toward_zero() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @divs() -> (i1, i1, i1) {
    %neg7 = arith.constant -7 : i32
    %two = arith.constant 2 : i32
    %ds = arith.divsi %neg7, %two : i32
    %rs = arith.remsi %neg7, %two : i32
    %fl = arith.floordivsi %neg7, %two : i32

    %m3 = arith.constant -3 : i32
    %m1 = arith.constant -1 : i32
    %m4 = arith.constant -4 : i32
    %eq_ds = arith.cmpi eq, %ds, %m3 : i32
    %eq_rs = arith.cmpi eq, %rs, %m1 : i32
    %eq_fl = arith.cmpi eq, %fl, %m4 : i32
    function.return %eq_ds, %eq_rs, %eq_fl : i1, i1, i1
  }
}
"#,
        "@divs",
        &[],
    );
    // divsi(-7,2) = -3 (truncation toward zero)
    // remsi(-7,2) = -1 (sign of dividend)
    // floordivsi(-7,2) = -4 (round toward -inf)
    assert_eq!(
        result,
        vec![Value::Bool(true), Value::Bool(true), Value::Bool(true)]
    );
}

#[test]
fn arith_unsigned_division_treats_negative_as_large() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @divu() -> i1 {
    %neg1 = arith.constant -1 : i32
    %two = arith.constant 2 : i32
    %du = arith.divui %neg1, %two : i32
    // (2^32 - 1) / 2 = 2^31 - 1 = 2147483647
    %expected = arith.constant 2147483647 : i32
    %eq = arith.cmpi eq, %du, %expected : i32
    function.return %eq : i1
  }
}
"#,
        "@divu",
        &[],
    );
    assert_eq!(result, vec![Value::Bool(true)]);
}

#[test]
fn arith_arithmetic_shift_right_preserves_sign() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @shr() -> (i1, i1) {
    %neg8 = arith.constant -8 : i32
    %two = arith.constant 2 : i32
    %srs = arith.shrsi %neg8, %two : i32
    %sru = arith.shrui %neg8, %two : i32
    %expected_s = arith.constant -2 : i32
    %expected_u = arith.constant 1073741822 : i32
    %eq_s = arith.cmpi eq, %srs, %expected_s : i32
    %eq_u = arith.cmpi eq, %sru, %expected_u : i32
    function.return %eq_s, %eq_u : i1, i1
  }
}
"#,
        "@shr",
        &[],
    );
    // -8 >>s 2 = -2 (arithmetic), -8 >>u 2 = 0xFFFFFFF8 >> 2 = 0x3FFFFFFE
    assert_eq!(result, vec![Value::Bool(true), Value::Bool(true)]);
}

#[test]
fn arith_extsi_extends_sign_bit() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @ext() -> (i1, i1) {
    %narrow = arith.constant -1 : i8
    %s = arith.extsi %narrow : i8 to i32
    %u = arith.extui %narrow : i8 to i32
    %neg1_32 = arith.constant -1 : i32
    %ff = arith.constant 255 : i32
    %eq_s = arith.cmpi eq, %s, %neg1_32 : i32
    %eq_u = arith.cmpi eq, %u, %ff : i32
    function.return %eq_s, %eq_u : i1, i1
  }
}
"#,
        "@ext",
        &[],
    );
    // extsi(-1:i8) -> -1:i32 ; extui(-1:i8) -> 0x000000FF
    assert_eq!(result, vec![Value::Bool(true), Value::Bool(true)]);
}

#[test]
fn arith_trunci_keeps_low_bits() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @tr() -> i1 {
    %wide = arith.constant 74565 : i32
    %narrow = arith.trunci %wide : i32 to i8
    %expected = arith.constant 69 : i8
    %eq = arith.cmpi eq, %narrow, %expected : i8
    function.return %eq : i1
  }
}
"#,
        "@tr",
        &[],
    );
    // 74565 = 0x12345; truncated to i8 = 0x45 = 69
    assert_eq!(result, vec![Value::Bool(true)]);
}

#[test]
fn arith_index_cast_to_and_from_i32() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @rt(%i: index) -> index {
    %as_i32 = arith.index_cast %i : index to i32
    %back = arith.index_cast %as_i32 : i32 to index
    function.return %back : index
  }
}
"#,
        "@rt",
        &[Value::Index(123)],
    );
    assert_eq!(result, vec![Value::Index(123)]);
}

#[test]
fn arith_signed_min_max_internal() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @mm() -> (i1, i1) {
    %neg = arith.constant -1 : i32
    %pos = arith.constant 1 : i32
    %xs = arith.maxsi %neg, %pos : i32
    %ns = arith.minsi %neg, %pos : i32
    %eq_xs = arith.cmpi eq, %xs, %pos : i32
    %eq_ns = arith.cmpi eq, %ns, %neg : i32
    function.return %eq_xs, %eq_ns : i1, i1
  }
}
"#,
        "@mm",
        &[],
    );
    assert_eq!(result, vec![Value::Bool(true), Value::Bool(true)]);
}

#[test]
fn arith_select_with_int_internal_branches() {
    let result = run_int_function(
        r#"
module attributes {llzk.lang} {
  function.def @sel(%c: i1) -> i1 {
    %a = arith.constant 10 : i32
    %b = arith.constant 20 : i32
    %r = arith.select %c, %a, %b : i32
    %expected = arith.constant 10 : i32
    %eq = arith.cmpi eq, %r, %expected : i32
    function.return %eq : i1
  }
}
"#,
        "@sel",
        &[Value::Bool(true)],
    );
    assert_eq!(result, vec![Value::Bool(true)]);
}

#[test]
fn arith_divsi_by_zero_fails() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @bad() -> index {
    %a = arith.constant 5 : i32
    %z = arith.constant 0 : i32
    %r = arith.divsi %a, %z : i32
    %idx = arith.constant 0 : index
    function.return %idx : index
  }
}
"#,
    )
    .expect("module should parse");
    let mut interpreter = Interpreter::new(&module);
    let err = interpreter
        .run_function("@bad", &[])
        .expect_err("division by zero should fail");
    assert!(err.to_string().contains("by zero"));
}

#[test]
fn bool_assert_in_constrain_rejects_dynamic_condition() {
    // The condition `%bit < 2` traces back to a struct member (witness),
    // so PCL would not lower this `bool.assert` to a real constraint.
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  struct.def @BadBit {
    struct.member @bit : !felt.type<"bn254"> {llzk.pub}
    function.def @compute(%b: !felt.type<"bn254">) -> !struct.type<@BadBit> {
      %self = struct.new : !struct.type<@BadBit>
      struct.writem %self[@bit] = %b : !struct.type<@BadBit>, !felt.type<"bn254">
      function.return %self : !struct.type<@BadBit>
    }
    function.def @constrain(%self: !struct.type<@BadBit>, %b: !felt.type<"bn254">) {
      %bit = struct.readm %self[@bit] : !struct.type<@BadBit>, !felt.type<"bn254">
      %two = felt.const 2 <"bn254">
      %ok = bool.cmp lt(%bit, %two) : !felt.type<"bn254">, !felt.type<"bn254">
      bool.assert %ok
      function.return
    }
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let computed = interpreter
        .run_compute("BadBit", &[Value::Felt(Felt::from_u64(1))])
        .expect("compute should succeed");
    let err = interpreter
        .run_constrain("BadBit", computed, &[Value::Felt(Felt::from_u64(1))])
        .expect_err("constrain should reject dynamic bool.assert");
    let msg = err.to_string();
    assert!(
        msg.contains("dynamic condition"),
        "unexpected error message: {msg}"
    );
}

#[test]
fn bool_assert_in_constrain_accepts_static_condition() {
    // Both `bool.cmp` operands are `felt.const`, so the condition folds to a
    // constant and PCL would emit it as a real polynomial constraint.
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  struct.def @StaticOk {
    struct.member @x : !felt.type<"bn254"> {llzk.pub}
    function.def @compute(%a: !felt.type<"bn254">) -> !struct.type<@StaticOk> {
      %self = struct.new : !struct.type<@StaticOk>
      struct.writem %self[@x] = %a : !struct.type<@StaticOk>, !felt.type<"bn254">
      function.return %self : !struct.type<@StaticOk>
    }
    function.def @constrain(%self: !struct.type<@StaticOk>, %a: !felt.type<"bn254">) {
      %one = felt.const 1 <"bn254">
      %two = felt.const 2 <"bn254">
      %ok = bool.cmp lt(%one, %two) : !felt.type<"bn254">, !felt.type<"bn254">
      bool.assert %ok
      function.return
    }
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let computed = interpreter
        .run_compute("StaticOk", &[Value::Felt(Felt::from_u64(0))])
        .expect("compute should succeed");
    interpreter
        .run_constrain("StaticOk", computed, &[Value::Felt(Felt::from_u64(0))])
        .expect("constrain with static bool.assert should succeed");
}

#[test]
fn bool_assert_in_compute_allows_dynamic_condition() {
    // Same body shape, but executed via `@compute` — `bool.assert` is the
    // Brillig trap idiom there, so it should remain a runtime check only.
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  struct.def @Bit {
    struct.member @bit : !felt.type<"bn254"> {llzk.pub}
    function.def @compute(%b: !felt.type<"bn254">) -> !struct.type<@Bit> {
      %two = felt.const 2 <"bn254">
      %ok = bool.cmp lt(%b, %two) : !felt.type<"bn254">, !felt.type<"bn254">
      bool.assert %ok
      %self = struct.new : !struct.type<@Bit>
      struct.writem %self[@bit] = %b : !struct.type<@Bit>, !felt.type<"bn254">
      function.return %self : !struct.type<@Bit>
    }
    function.def @constrain(%self: !struct.type<@Bit>, %b: !felt.type<"bn254">) {
      function.return
    }
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    interpreter
        .run_compute("Bit", &[Value::Felt(Felt::from_u64(1))])
        .expect("compute should allow dynamic bool.assert");
}

#[test]
fn bool_assert_in_constrain_helper_rejects_dynamic_condition() {
    // The dynamic operand crosses a `function.call` boundary: the helper's
    // `bool.assert` operand traces back through an arg origin, which we
    // propagate from the caller, so the check still fires.
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @range_check(%v: !felt.type<"bn254">) {
    %two = felt.const 2 <"bn254">
    %ok = bool.cmp lt(%v, %two) : !felt.type<"bn254">, !felt.type<"bn254">
    bool.assert %ok
    function.return
  }
  struct.def @BadBitHelper {
    struct.member @bit : !felt.type<"bn254"> {llzk.pub}
    function.def @compute(%b: !felt.type<"bn254">) -> !struct.type<@BadBitHelper> {
      %self = struct.new : !struct.type<@BadBitHelper>
      struct.writem %self[@bit] = %b : !struct.type<@BadBitHelper>, !felt.type<"bn254">
      function.return %self : !struct.type<@BadBitHelper>
    }
    function.def @constrain(%self: !struct.type<@BadBitHelper>, %b: !felt.type<"bn254">) {
      %bit = struct.readm %self[@bit] : !struct.type<@BadBitHelper>, !felt.type<"bn254">
      function.call @range_check(%bit) : (!felt.type<"bn254">) -> ()
      function.return
    }
  }
}
"#,
    )
    .expect("module should parse");

    let mut interpreter = Interpreter::new(&module);
    let computed = interpreter
        .run_compute("BadBitHelper", &[Value::Felt(Felt::from_u64(1))])
        .expect("compute should succeed");
    let err = interpreter
        .run_constrain("BadBitHelper", computed, &[Value::Felt(Felt::from_u64(1))])
        .expect_err("constrain should reject dynamic bool.assert in helper");
    assert!(err.to_string().contains("dynamic condition"));
}

#[test]
fn arith_remui_by_zero_on_index_fails() {
    let context = LlzkContext::new();
    let module = Module::parse(
        &context,
        r#"
module attributes {llzk.lang} {
  function.def @bad() -> index {
    %a = arith.constant 5 : index
    %z = arith.constant 0 : index
    %r = arith.remui %a, %z : index
    function.return %r : index
  }
}
"#,
    )
    .expect("module should parse");
    let mut interpreter = Interpreter::new(&module);
    let err = interpreter
        .run_function("@bad", &[])
        .expect_err("remainder by zero should fail");
    assert!(err.to_string().contains("by zero"));
}
