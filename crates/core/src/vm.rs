//! Runs a compiled `Program`.
//!
//! The VM never sees the graph — only instructions. A step budget keeps a
//! runaway program from freezing the browser tab.

use crate::compile::{Expr, Instr, Program};
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
            match (a.as_number(), b.as_number()) {
                (Some(x), Some(y)) => Ok(Value::Bool(x < y)),
                _ => Err("'less than' needs two numbers".to_string()),
            }
        }
    }
}
