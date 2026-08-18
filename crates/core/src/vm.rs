//! Runs a compiled `Program`.
//!
//! The VM never sees the graph — only instructions. A step budget keeps a
//! runaway program from freezing the browser tab.

use crate::compile::{Expr, Instr, Program};
use crate::graph::ArithOp;
use crate::types::DataType;
use crate::types::Value;
use std::collections::BTreeMap;

pub const DEFAULT_STEP_LIMIT: usize = 100_000;

#[derive(Clone, Debug, Default)]
pub struct RunResult {
    pub output: Vec<String>,
    /// `Some` if the program stopped early.
    pub error: Option<String>,
    pub steps: usize,
}

pub fn run(program: &Program) -> RunResult {
    run_with_limit(program, DEFAULT_STEP_LIMIT)
}

pub fn run_with_limit(program: &Program, step_limit: usize) -> RunResult {
    let mut vars: BTreeMap<String, Value> = program.vars.clone();
    let mut result = RunResult::default();
    let mut pc = 0usize;

    while pc < program.instrs.len() {
        if result.steps >= step_limit {
            result.error = Some(format!(
                "stopped after {step_limit} steps — the program may not finish"
            ));
            return result;
        }
        result.steps += 1;

        match &program.instrs[pc] {
            Instr::Print(e) => match eval(e, &vars) {
                Ok(v) => {
                    result.output.push(v.to_string());
                    pc += 1;
                }
                Err(msg) => {
                    result.error = Some(msg);
                    return result;
                }
            },
            Instr::SetVar(name, e) => match eval(e, &vars) {
                Ok(v) => {
                    vars.insert(name.clone(), v);
                    pc += 1;
                }
                Err(msg) => {
                    result.error = Some(msg);
                    return result;
                }
            },
            Instr::JumpIfFalse(e, target) => match eval(e, &vars) {
                Ok(v) => {
                    let condition = v.as_bool().unwrap_or(false);
                    pc = if condition { pc + 1 } else { *target };
                }
                Err(msg) => {
                    result.error = Some(msg);
                    return result;
                }
            },
            Instr::Jump(target) => pc = *target,
        }
    }

    result
}

fn eval(expr: &Expr, vars: &BTreeMap<String, Value>) -> Result<Value, String> {
    match expr {
        Expr::Lit(v) => Ok(v.clone()),
        Expr::GetVar(name) => vars
            .get(name)
            .cloned()
            .ok_or_else(|| format!("variable '{name}' does not exist")),
        Expr::LessThan(a, b) => {
            let a = eval(a, vars)?;
            let b = eval(b, vars)?;
            match (&a, &b) {
                // Whole numbers compare as whole numbers.
                //
                // Going through `as_number` casts both to f64, which cannot tell apart
                // two whole numbers above 2^53 — so this answered "not less" for
                // 9223372036854775806 < 9223372036854775807 while the compiled path,
                // emitting `i64.lt_s`, answered "less". The two implementations
                // disagreed exactly where the node cautions say the limit is, which is
                // the one thing the oracle exists to make impossible.
                (Value::Int(x), Value::Int(y)) => Ok(Value::Bool(x < y)),
                _ => match (a.as_number(), b.as_number()) {
                    (Some(x), Some(y)) => Ok(Value::Bool(x < y)),
                    _ => Err("'less than' needs two numbers".to_string()),
                },
            }
        }
        Expr::Arith(op, ty, a, b) => {
            let a = eval(a, vars)?;
            let b = eval(b, vars)?;
            arith(*op, *ty, a, b)
        }
    }
}

/// Integer and float arithmetic, kept in step with what the WebAssembly backend emits.
///
/// Whole-number division rounds towards zero and dividing by zero stops the program,
/// because that is what `i64.div_s` does — the interpreter is only useful as a second
/// opinion if it agrees on the awkward cases too.
fn arith(op: ArithOp, ty: DataType, a: Value, b: Value) -> Result<Value, String> {
    match ty {
        DataType::Float => {
            let (Some(x), Some(y)) = (a.as_number(), b.as_number()) else {
                return Err(format!("'{}' needs two numbers", op.word()));
            };
            Ok(Value::Float(match op {
                ArithOp::Add => x + y,
                ArithOp::Subtract => x - y,
                ArithOp::Multiply => x * y,
                ArithOp::Divide => x / y,
            }))
        }
        _ => {
            let (Value::Int(x), Value::Int(y)) = (&a, &b) else {
                return Err(format!("'{}' needs two whole numbers", op.word()));
            };
            let (x, y) = (*x, *y);
            let out = match op {
                ArithOp::Add => x.wrapping_add(y),
                ArithOp::Subtract => x.wrapping_sub(y),
                ArithOp::Multiply => x.wrapping_mul(y),
                ArithOp::Divide => {
                    if y == 0 {
                        return Err("cannot divide a whole number by zero".to_string());
                    }
                    x.wrapping_div(y)
                }
            };
            Ok(Value::Int(out))
        }
    }
}
