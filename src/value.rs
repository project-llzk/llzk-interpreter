use std::{
    cell::RefCell,
    collections::BTreeMap,
    fmt,
    ops::{Add, Div, Mul, Neg, Sub},
    rc::Rc,
};

use num_bigint::{BigInt, BigUint, Sign};
use num_traits::{One, Zero};

/// Shared pointer to a mutable struct instance.
pub type StructRef = Rc<RefCell<StructInstance>>;
/// Shared pointer to a mutable array instance.
pub type ArrayRef = Rc<RefCell<ArrayInstance>>;

/// Concrete BN254 field element used by the toy interpreter.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct Felt {
    value: BigUint,
}

impl Felt {
    /// Constructs a field element from a canonical non-negative integer.
    pub fn new(value: BigUint) -> Self {
        Self {
            value: value % modulus(),
        }
    }

    /// Constructs a field element from a small integer.
    pub fn from_u64(value: u64) -> Self {
        Self::new(BigUint::from(value))
    }

    /// Returns the additive identity (zero).
    pub fn zero() -> Self {
        Self::from_u64(0)
    }

    /// Constructs a field element from decimal digits.
    pub fn from_decimal(value: &str) -> Result<Self, String> {
        let bigint = BigUint::parse_bytes(value.as_bytes(), 10)
            .ok_or_else(|| format!("invalid decimal felt: {value}"))?;
        Ok(Self::new(bigint))
    }

    /// Returns the canonical integer representative.
    pub fn as_biguint(&self) -> &BigUint {
        &self.value
    }

    /// Bitwise AND (operating on canonical integer representatives).
    pub fn bit_and(&self, rhs: &Self) -> Self {
        Self::new(&self.value & &rhs.value)
    }

    /// Bitwise OR (operating on canonical integer representatives).
    pub fn bit_or(&self, rhs: &Self) -> Self {
        Self::new(&self.value | &rhs.value)
    }

    /// Bitwise XOR (operating on canonical integer representatives).
    pub fn bit_xor(&self, rhs: &Self) -> Self {
        Self::new(&self.value ^ &rhs.value)
    }

    /// Left shift by `amount` bits, reduced modulo p.
    pub fn shl(&self, amount: u32) -> Self {
        Self::new(&self.value << amount)
    }

    /// Right shift by `amount` bits (unsigned, no modular reduction needed).
    pub fn shr(&self, amount: u32) -> Self {
        Self::new(&self.value >> amount)
    }

    /// Modular exponentiation: self^exp mod p.
    pub fn pow(&self, exp: &Self) -> Self {
        Self::new(self.value.modpow(&exp.value, modulus()))
    }

    fn inverse(&self) -> Result<Self, String> {
        if self.value.is_zero() {
            return Err("division by zero".into());
        }

        let modulus_bigint = BigInt::from_biguint(Sign::Plus, modulus().clone());
        let value_bigint = BigInt::from_biguint(Sign::Plus, self.value.clone());
        let (gcd, x, _) = extended_gcd(value_bigint, modulus_bigint.clone());
        if gcd != BigInt::one() {
            return Err("value has no inverse".into());
        }

        let normalized = ((x % &modulus_bigint) + &modulus_bigint) % &modulus_bigint;
        let result = normalized
            .to_biguint()
            .ok_or_else(|| "failed to normalize inverse".to_string())?;
        Ok(Self::new(result))
    }
}

impl Add for Felt {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self::new(self.value + rhs.value)
    }
}

impl Sub for Felt {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self::Output {
        let modulus = modulus();
        if self.value >= rhs.value {
            Self::new(self.value - rhs.value)
        } else {
            Self::new((&self.value + modulus) - rhs.value)
        }
    }
}

impl Mul for Felt {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        Self::new(self.value * rhs.value)
    }
}

impl Div for Felt {
    type Output = Self;

    #[allow(clippy::suspicious_arithmetic_impl)]
    fn div(self, rhs: Self) -> Self::Output {
        let inv = rhs
            .inverse()
            .expect("division by zero in llzk_interpreter felt");
        Self::new(self.value * inv.value)
    }
}

impl Neg for Felt {
    type Output = Self;

    fn neg(self) -> Self::Output {
        if self.value.is_zero() {
            self
        } else {
            Self::new(modulus() - self.value)
        }
    }
}

/// Fixed-width two's-complement integer value.
///
/// `value` is the canonical unsigned representation in `[0, 2^width)`.
/// Operations interpret bits as signed or unsigned per the operation's
/// semantics (e.g. `arith.divsi` vs `arith.divui`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IntValue {
    value: BigUint,
    width: u32,
}

impl IntValue {
    /// Creates an integer value, masking to the given bit width.
    pub fn new(value: BigUint, width: u32) -> Self {
        if width == 0 {
            return Self {
                value: BigUint::zero(),
                width,
            };
        }
        let modulus = BigUint::one() << width;
        Self {
            value: value % modulus,
            width,
        }
    }

    /// Creates an integer value from a (possibly negative) signed integer,
    /// reducing to the canonical unsigned representation.
    pub fn from_signed(value: BigInt, width: u32) -> Self {
        if width == 0 {
            return Self {
                value: BigUint::zero(),
                width,
            };
        }
        let modulus = BigInt::from_biguint(Sign::Plus, BigUint::one() << width);
        let normalized = ((value % &modulus) + &modulus) % &modulus;
        Self {
            value: normalized
                .to_biguint()
                .expect("normalized integer is non-negative"),
            width,
        }
    }

    /// Creates an integer value from a small signed integer.
    pub fn from_i64(value: i64, width: u32) -> Self {
        Self::from_signed(BigInt::from(value), width)
    }

    /// Bit width.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Canonical unsigned representation.
    pub fn as_unsigned(&self) -> &BigUint {
        &self.value
    }

    /// Two's-complement signed interpretation.
    pub fn as_signed(&self) -> BigInt {
        if self.width == 0 {
            return BigInt::zero();
        }
        let half = BigUint::one() << (self.width - 1);
        if self.value >= half {
            let modulus = BigUint::one() << self.width;
            BigInt::from_biguint(Sign::Plus, self.value.clone())
                - BigInt::from_biguint(Sign::Plus, modulus)
        } else {
            BigInt::from_biguint(Sign::Plus, self.value.clone())
        }
    }
}

impl fmt::Display for IntValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "i{}({})", self.width, self.value)
    }
}

/// Concrete runtime values supported by the toy interpreter.
#[derive(Clone, Debug)]
pub enum Value {
    /// A field element.
    Felt(Felt),
    /// A boolean (i1) value.
    Bool(bool),
    /// An `index`-typed value.
    Index(usize),
    /// A fixed-width integer (`iN` for N >= 2).
    Int(IntValue),
    /// A mutable array instance.
    Array(ArrayRef),
    /// A mutable struct instance.
    Struct(StructRef),
}

/// Runtime state for one LLZK array value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ArrayInstance {
    /// Current elements keyed by their linearized index tuple.
    pub elements: BTreeMap<Vec<usize>, Value>,
}

/// Runtime state for one LLZK struct value.
#[derive(Clone, Debug, Default)]
pub struct StructInstance {
    /// The struct symbol name.
    pub type_name: String,
    /// Current member values keyed by member name.
    pub members: BTreeMap<String, Value>,
}

impl StructInstance {
    /// Creates an empty struct instance of the given type.
    pub fn new(type_name: impl Into<String>) -> Self {
        Self {
            type_name: type_name.into(),
            members: BTreeMap::new(),
        }
    }
}

impl ArrayInstance {
    /// Creates an empty array instance.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates a 1D array initialized from a flat value list.
    pub fn from_values(values: Vec<Value>) -> Self {
        let elements = values
            .into_iter()
            .enumerate()
            .map(|(index, value)| (vec![index], value))
            .collect();
        Self { elements }
    }

    /// Returns the value at `indices`, if one has been written.
    pub fn read(&self, indices: &[usize]) -> Option<Value> {
        self.elements.get(indices).cloned()
    }

    /// Writes `value` at `indices`.
    pub fn write(&mut self, indices: &[usize], value: Value) {
        self.elements.insert(indices.to_vec(), value);
    }
}

impl Value {
    /// Returns the inner felt value or an explanatory error string.
    pub fn as_felt(&self) -> Result<Felt, String> {
        match self {
            Self::Felt(value) => Ok(value.clone()),
            other => Err(format!("expected felt, got {other}")),
        }
    }

    /// Returns the inner bool value or an explanatory error string.
    pub fn as_bool(&self) -> Result<bool, String> {
        match self {
            Self::Bool(value) => Ok(*value),
            other => Err(format!("expected bool, got {other}")),
        }
    }

    /// Returns the inner index value or an explanatory error string.
    pub fn as_index(&self) -> Result<usize, String> {
        match self {
            Self::Index(value) => Ok(*value),
            other => Err(format!("expected index, got {other}")),
        }
    }

    /// Returns the inner fixed-width integer or an explanatory error string.
    pub fn as_int(&self) -> Result<IntValue, String> {
        match self {
            Self::Int(value) => Ok(value.clone()),
            other => Err(format!("expected int, got {other}")),
        }
    }

    /// Returns the inner array reference or an explanatory error string.
    pub fn as_array(&self) -> Result<ArrayRef, String> {
        match self {
            Self::Array(value) => Ok(value.clone()),
            other => Err(format!("expected array, got {other}")),
        }
    }

    /// Returns the inner struct reference or an explanatory error string.
    pub fn as_struct(&self) -> Result<StructRef, String> {
        match self {
            Self::Struct(value) => Ok(value.clone()),
            other => Err(format!("expected struct, got {other}")),
        }
    }
}

impl PartialEq for StructInstance {
    fn eq(&self, other: &Self) -> bool {
        self.type_name == other.type_name && self.members == other.members
    }
}

impl Eq for StructInstance {}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Felt(lhs), Self::Felt(rhs)) => lhs == rhs,
            (Self::Bool(lhs), Self::Bool(rhs)) => lhs == rhs,
            (Self::Index(lhs), Self::Index(rhs)) => lhs == rhs,
            (Self::Int(lhs), Self::Int(rhs)) => lhs == rhs,
            (Self::Array(lhs), Self::Array(rhs)) => *lhs.borrow() == *rhs.borrow(),
            (Self::Struct(lhs), Self::Struct(rhs)) => *lhs.borrow() == *rhs.borrow(),
            _ => false,
        }
    }
}

impl Eq for Value {}

impl fmt::Display for StructInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {:?}", self.type_name, self.members)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Felt(value) => write!(f, "felt({value})"),
            Self::Bool(value) => write!(f, "bool({value})"),
            Self::Index(value) => write!(f, "index({value})"),
            Self::Int(value) => write!(f, "int({value})"),
            Self::Array(value) => write!(f, "array({:?})", value.borrow().elements),
            Self::Struct(value) => write!(f, "struct({})", value.borrow()),
        }
    }
}

impl fmt::Display for Felt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

fn modulus() -> &'static BigUint {
    static MODULUS: std::sync::LazyLock<BigUint> = std::sync::LazyLock::new(|| {
        BigUint::parse_bytes(
            b"21888242871839275222246405745257275088548364400416034343698204186575808495617",
            10,
        )
        .expect("valid bn254 modulus")
    });
    &MODULUS
}

fn extended_gcd(a: BigInt, b: BigInt) -> (BigInt, BigInt, BigInt) {
    if b.is_zero() {
        (a, BigInt::one(), BigInt::zero())
    } else {
        let (gcd, x1, y1) = extended_gcd(b.clone(), a.clone() % b.clone());
        let x = y1.clone();
        let y = x1 - (a / b) * y1;
        (gcd, x, y)
    }
}
