//! The type system: what kinds of value can travel along a data wire.

use std::fmt;

/// The type of a data pin. A wire may only join two data pins of the same type,
/// which is what makes an invalid connection impossible to draw in the editor.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum DataType {
    Int,
    Float,
    Bool,
    Str,
}

impl DataType {
    pub fn label(self) -> &'static str {
        match self {
            DataType::Int => "integer",
            DataType::Float => "float",
            DataType::Bool => "boolean",
            DataType::Str => "string",
        }
    }

    /// Every type needs a starting value, used when a variable is created.
    pub fn default_value(self) -> Value {
        match self {
            DataType::Int => Value::Int(0),
            DataType::Float => Value::Float(0.0),
            DataType::Bool => Value::Bool(false),
            DataType::Str => Value::Str(String::new()),
        }
    }

    pub const ALL: [DataType; 4] = [
        DataType::Int,
        DataType::Float,
        DataType::Bool,
        DataType::Str,
    ];
}

/// A runtime value. `DataType` is the compile-time counterpart.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

impl Value {
    pub fn data_type(&self) -> DataType {
        match self {
            Value::Int(_) => DataType::Int,
            Value::Float(_) => DataType::Float,
            Value::Bool(_) => DataType::Bool,
            Value::Str(_) => DataType::Str,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Ints and floats are both comparable as numbers, so ordering nodes can
    /// accept either without caring which it got.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Bool(b) => write!(f, "{b}"),
            Value::Str(s) => write!(f, "{s}"),
        }
    }
}
