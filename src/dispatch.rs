use llzk::prelude::{
    Attribute, AttributeLike, BlockLike, FlatSymbolRefAttribute, FuncDefOpLike, FuncDefOpRef,
    IntegerAttribute, IntegerType, OperationLike, OperationRef, SymbolRefAttribute, Type, TypeLike,
    Value, ValueLike,
};
use num_bigint::{BigInt, BigUint, Sign};
use num_traits::Zero;

use crate::{
    Error, Result,
    value::{Felt, IntValue, Value as RuntimeValue},
};

pub fn iter_block_ops<'c, 'a>(
    block: llzk::prelude::BlockRef<'c, 'a>,
) -> impl Iterator<Item = OperationRef<'c, 'a>> {
    std::iter::successors(block.first_operation(), |op| op.next_in_block())
}

pub fn operands<'c, 'a>(op: &OperationRef<'c, 'a>) -> Result<Vec<Value<'c, 'a>>> {
    (0..op.operand_count())
        .map(|i| op.operand(i).map_err(Into::into))
        .collect()
}

pub fn fq_function_name<'c, 'a>(func: &FuncDefOpRef<'c, 'a>) -> String {
    func.fully_qualified_name().to_string()
}

pub fn call_target<'c, 'a>(op: &OperationRef<'c, 'a>) -> Result<String> {
    Ok(SymbolRefAttribute::try_from(op.attribute("callee").map_err(Error::from)?)?.to_string())
}

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
    let value = parse_signed_int_const(attr)?;
    if value.sign() == Sign::Minus {
        return Err(Error::ParseError(format!(
            "negative constant cannot fit in usize: {value}"
        )));
    }
    let unsigned: BigUint = value
        .to_biguint()
        .ok_or_else(|| Error::ParseError(format!("integer constant out of range: {value}")))?;
    unsigned
        .try_into()
        .map_err(|_| Error::ParseError(format!("integer constant too large for usize: {value}")))
}

/// Parses a possibly-negative integer payload from an `arith.constant` value attribute.
pub fn parse_signed_int_const<'c>(attr: Attribute<'c>) -> Result<BigInt> {
    if let Ok(integer) = IntegerAttribute::try_from(attr) {
        let ty = integer.r#type();
        if let Ok(int_ty) = IntegerType::try_from(ty)
            && int_ty.is_unsigned()
        {
            return Ok(BigInt::from(integer.unsigned_value()));
        }
        return Ok(BigInt::from(integer.value()));
    }

    let text = attr.to_string();
    let body = text.trim();
    let body = body.split(':').next().unwrap_or(body).trim();
    let (sign, digits) = if let Some(rest) = body.strip_prefix('-') {
        (Sign::Minus, rest.trim_start())
    } else {
        (Sign::Plus, body)
    };
    let digit_chunk = digits
        .split(|c: char| !c.is_ascii_digit())
        .find(|chunk| !chunk.is_empty())
        .ok_or_else(|| Error::ParseError(format!("invalid integer const attribute: {text}")))?;
    let unsigned = BigUint::parse_bytes(digit_chunk.as_bytes(), 10)
        .ok_or_else(|| Error::ParseError(format!("invalid integer digits: {digit_chunk}")))?;
    if unsigned.is_zero() {
        Ok(BigInt::from(0))
    } else {
        Ok(BigInt::from_biguint(sign, unsigned))
    }
}

/// Builds a runtime value for an `arith.constant` op, matching the result type.
pub fn parse_arith_const_value<'c, 'a>(
    attr: Attribute<'c>,
    result_type: Type<'a>,
) -> Result<RuntimeValue> {
    if result_type.is_index() {
        return Ok(RuntimeValue::Index(parse_usize_const(attr)?));
    }
    if let Ok(int_ty) = IntegerType::try_from(result_type) {
        let width = int_ty.width();
        let value = parse_signed_int_const(attr)?;
        if width == 1 {
            return Ok(RuntimeValue::Bool(!value.is_zero()));
        }
        return Ok(RuntimeValue::Int(IntValue::from_signed(value, width)));
    }
    Err(Error::TypeError(format!(
        "unsupported arith.constant result type: {}",
        result_type
    )))
}

#[derive(Clone, Copy, Debug)]
pub enum CmpIPredicate {
    Eq,
    Ne,
    Slt,
    Sle,
    Sgt,
    Sge,
    Ult,
    Ule,
    Ugt,
    Uge,
}

/// Parses an `arith.cmpi` predicate attribute. The attribute is stored as an
/// integer matching the LLVM/MLIR `arith::CmpIPredicate` enum order.
pub fn parse_cmpi_predicate<'c>(attr: Attribute<'c>) -> Result<CmpIPredicate> {
    let raw = if let Ok(integer) = IntegerAttribute::try_from(attr) {
        integer.value()
    } else {
        // Fall back to textual parsing.
        let text = attr.to_string();
        let body = text.split(':').next().unwrap_or(&text).trim();
        body.parse::<i64>().map_err(|_| {
            Error::ParseError(format!("invalid arith.cmpi predicate attribute: {text}"))
        })?
    };
    Ok(match raw {
        0 => CmpIPredicate::Eq,
        1 => CmpIPredicate::Ne,
        2 => CmpIPredicate::Slt,
        3 => CmpIPredicate::Sle,
        4 => CmpIPredicate::Sgt,
        5 => CmpIPredicate::Sge,
        6 => CmpIPredicate::Ult,
        7 => CmpIPredicate::Ule,
        8 => CmpIPredicate::Ugt,
        9 => CmpIPredicate::Uge,
        other => {
            return Err(Error::ParseError(format!(
                "unknown arith.cmpi predicate: {other}"
            )));
        }
    })
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
