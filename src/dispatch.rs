use llzk::prelude::{
    Attribute, BlockLike, FlatSymbolRefAttribute, FuncDefOpLike, FuncDefOpRef, Module,
    OperationLike, OperationRef, StructDefOpLike, StructDefOpRef, SymbolRefAttribute, Value,
    ValueLike,
};
use num_bigint::BigUint;

use crate::{Error, Result, value::Felt};

/// Iterates over all operations in a block in order.
pub fn iter_block_ops<'c, 'a>(
    block: llzk::prelude::BlockRef<'c, 'a>,
) -> impl Iterator<Item = OperationRef<'c, 'a>> {
    std::iter::successors(block.first_operation(), |op| op.next_in_block())
}

/// Returns all operands of an operation.
pub fn operands<'c, 'a>(op: &OperationRef<'c, 'a>) -> Result<Vec<Value<'c, 'a>>> {
    (0..op.operand_count())
        .map(|i| op.operand(i).map_err(Into::into))
        .collect()
}

/// Returns the fully qualified function name.
pub fn fq_function_name<'c, 'a>(func: &FuncDefOpRef<'c, 'a>) -> String {
    func.fully_qualified_name().to_string()
}

/// Returns the callee symbol string from a `function.call`.
pub fn call_target<'c, 'a>(op: &OperationRef<'c, 'a>) -> Result<String> {
    Ok(SymbolRefAttribute::try_from(op.attribute("callee").map_err(Error::from)?)?.to_string())
}

/// Finds a struct definition by symbol name.
pub fn find_struct<'c, 'a>(module: &'a Module<'c>, name: &str) -> Result<StructDefOpRef<'c, 'a>> {
    for op in iter_block_ops(module.body()) {
        if let Ok(struct_def) = StructDefOpRef::try_from(op)
            && StructDefOpLike::name(&struct_def) == name
        {
            return Ok(struct_def);
        }
    }
    Err(Error::SymbolNotFound(format!("struct @{name}")))
}

/// Finds a function definition by fully qualified symbol string.
pub fn find_function<'c, 'a>(module: &'a Module<'c>, symbol: &str) -> Result<FuncDefOpRef<'c, 'a>> {
    for op in iter_block_ops(module.body()) {
        if let Ok(func) = FuncDefOpRef::try_from(op)
            && fq_function_name(&func) == symbol
        {
            return Ok(func);
        }
        if let Ok(struct_def) = StructDefOpRef::try_from(op) {
            for inner in iter_block_ops(struct_def.body()) {
                if let Ok(func) = FuncDefOpRef::try_from(inner)
                    && fq_function_name(&func) == symbol
                {
                    return Ok(func);
                }
            }
        }
    }
    Err(Error::SymbolNotFound(format!("function {symbol}")))
}

/// Reads the member name attribute from `struct.readm` or `struct.writem`.
pub fn member_name<'c, 'a>(op: &OperationRef<'c, 'a>) -> Result<String> {
    let attr = op.attribute("member_name").map_err(Error::from)?;
    Ok(FlatSymbolRefAttribute::try_from(attr)?.value().to_string())
}

/// Parses the decimal integer payload of a `felt.const` op.
pub fn parse_felt_const<'c>(attr: Attribute<'c>) -> Result<Felt> {
    let text = attr.to_string();
    let digits = text
        .split(|c: char| !c.is_ascii_digit())
        .find(|chunk| !chunk.is_empty())
        .ok_or_else(|| Error::ParseError(format!("invalid felt const attribute: {text}")))?;
    let value = BigUint::parse_bytes(digits.as_bytes(), 10)
        .ok_or_else(|| Error::ParseError(format!("invalid felt digits: {digits}")))?;
    Ok(Felt::new(value))
}

/// Parses the non-negative integer payload of an `arith.constant` op.
pub fn parse_usize_const<'c>(attr: Attribute<'c>) -> Result<usize> {
    let text = attr.to_string();
    let digits = text
        .split(|c: char| !c.is_ascii_digit())
        .find(|chunk| !chunk.is_empty())
        .ok_or_else(|| Error::ParseError(format!("invalid integer const attribute: {text}")))?;
    let value = BigUint::parse_bytes(digits.as_bytes(), 10)
        .ok_or_else(|| Error::ParseError(format!("invalid integer digits: {digits}")))?;
    value
        .try_into()
        .map_err(|_| Error::ParseError(format!("integer constant too large for usize: {text}")))
}

/// Parses the comparison predicate from a `bool.cmp` attribute.
pub fn parse_cmp_predicate<'c>(attr: Attribute<'c>) -> Result<&'static str> {
    let text = attr.to_string();
    let pred = text
        .split(|c: char| !c.is_ascii_alphabetic())
        .rfind(|chunk| !chunk.is_empty())
        .ok_or_else(|| Error::ParseError(format!("invalid bool.cmp predicate: {text}")))?;
    Ok(match pred {
        "eq" => "eq",
        "ne" => "ne",
        "lt" => "lt",
        "le" => "le",
        "gt" => "gt",
        "ge" => "ge",
        _ => return Err(Error::ParseError(format!("unknown cmp predicate: {pred}"))),
    })
}

/// Extracts the struct name from the result type of a `struct.new`.
pub fn result_struct_name<'c, 'a>(op: &OperationRef<'c, 'a>) -> Result<String> {
    let ty = op.result(0).map_err(Error::from)?.r#type().to_string();
    let name = ty
        .split('<')
        .nth(1)
        .and_then(|tail| tail.split('>').next())
        .ok_or_else(|| Error::ParseError(format!("invalid struct type: {ty}")))?;
    Ok(name.trim_start_matches('@').to_string())
}
