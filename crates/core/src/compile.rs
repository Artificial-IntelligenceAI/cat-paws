//! Turns a node graph into a flat list of instructions.
//!
//! This is what the hammer button runs. Compiling is deliberately separate from
//! running: it walks the execution wires once, reports every problem it finds,
//! and produces a `Program` that the VM can execute without ever looking at the
//! graph again.

use crate::graph::{Graph, NodeId, NodeKind, PinRef, Side};
use crate::types::{DataType, Value};
use std::collections::BTreeMap;

/// A problem found while compiling. `node` lets the editor highlight the culprit.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub node: Option<NodeId>,
    pub message: String,
}

impl Diagnostic {
    fn at(node: NodeId, message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            node: Some(node),
            message: message.into(),
        }
    }

    fn global(message: impl Into<String>) -> Diagnostic {
        Diagnostic {
            node: None,
            message: message.into(),
        }
    }
}

/// A value computed at runtime. Data wires compile into a tree of these.
#[derive(Clone, Debug)]
pub enum Expr {
    Lit(Value),
    GetVar(String),
    LessThan(Box<Expr>, Box<Expr>),
}

/// One step of the compiled program. Jumps hold an absolute instruction index.
#[derive(Clone, Debug)]
pub enum Instr {
    Print(Expr),
    SetVar(String, Expr),
    JumpIfFalse(Expr, usize),
    Jump(usize),
}

#[derive(Clone, Debug, Default)]
pub struct Program {
    pub instrs: Vec<Instr>,
    /// Starting value of every variable, captured at compile time.
    pub vars: BTreeMap<String, Value>,
}

impl Program {
    /// A plain-text listing, so the user can see what the hammer produced.
    pub fn listing(&self) -> Vec<String> {
        self.instrs
            .iter()
            .enumerate()
            .map(|(i, instr)| {
                let body = match instr {
                    Instr::Print(e) => format!("print {}", show(e)),
                    Instr::SetVar(name, e) => format!("set {name} = {}", show(e)),
                    Instr::JumpIfFalse(e, t) => format!("jump_if_false {} -> {t}", show(e)),
                    Instr::Jump(t) => format!("jump -> {t}"),
                };
                format!("{i:>3}  {body}")
            })
            .collect()
    }
}

fn show(e: &Expr) -> String {
    match e {
        Expr::Lit(Value::Str(s)) => format!("{s:?}"),
        Expr::Lit(v) => v.to_string(),
        Expr::GetVar(name) => name.clone(),
        Expr::LessThan(a, b) => format!("({} < {})", show(a), show(b)),
    }
}

/// Compiles a graph, or returns every problem that stopped it.
pub fn compile(graph: &Graph) -> Result<Program, Vec<Diagnostic>> {
    let starts: Vec<NodeId> = graph
        .nodes()
        .filter(|n| n.kind == NodeKind::EventStart)
        .map(|n| n.id)
        .collect();

    let start = match starts.as_slice() {
        [] => {
            return Err(vec![Diagnostic::global(
                "no Event start node — add one to say where the program begins",
            )])
        }
        [one] => *one,
        many => {
            return Err(many
                .iter()
                .map(|id| Diagnostic::at(*id, "more than one Event start node; keep only one"))
                .collect())
        }
    };

    let mut c = Compiler {
        graph,
        instrs: Vec::new(),
        diags: Vec::new(),
        exec_path: Vec::new(),
        data_path: Vec::new(),
    };

    c.emit_after(PinRef {
        node: start,
        side: Side::Out,
        index: 0,
    });

    if c.diags.is_empty() {
        let vars = graph
            .vars
            .iter()
            .map(|(name, decl)| (name.clone(), decl.initial.clone()))
            .collect();
        Ok(Program {
            instrs: c.instrs,
            vars,
        })
    } else {
        Err(c.diags)
    }
}

struct Compiler<'a> {
    graph: &'a Graph,
    instrs: Vec<Instr>,
    diags: Vec<Diagnostic>,
    /// Nodes on the current execution path, used to catch loops.
    exec_path: Vec<NodeId>,
    /// Nodes on the current data path, used to catch circular values.
    data_path: Vec<NodeId>,
}

impl<'a> Compiler<'a> {
    /// Emits whatever this execution output pin leads to. An unconnected exec
    /// output simply ends that branch, which is not an error.
    fn emit_after(&mut self, out_pin: PinRef) {
        if let Some(target) = self.graph.target_of(out_pin) {
            self.emit_node(target.node);
        }
    }

    fn emit_node(&mut self, id: NodeId) {
        if self.exec_path.contains(&id) {
            self.diags.push(Diagnostic::at(
                id,
                "execution wires form a loop; loops are not supported yet",
            ));
            return;
        }
        let Some(kind) = self.graph.kind_of(id).cloned() else {
            return;
        };
        self.exec_path.push(id);

        match kind {
            NodeKind::Print => {
                let text = self.input_expr(id, 1, DataType::Str);
                self.instrs.push(Instr::Print(text));
                self.emit_after(out(id, 0));
            }
            NodeKind::SetVar { ref name, ty } => {
                let value = self.input_expr(id, 1, ty);
                self.instrs.push(Instr::SetVar(name.clone(), value));
                self.emit_after(out(id, 0));
            }
            NodeKind::Branch => {
                let cond = self.input_expr(id, 1, DataType::Bool);

                // Reserve the conditional jump, fill in its target once we know
                // where the false branch starts.
                let jump_if_false = self.instrs.len();
                self.instrs.push(Instr::JumpIfFalse(cond, usize::MAX));

                self.emit_after(out(id, 0)); // true path

                // At the end of the true path, skip over the false path.
                let jump_to_end = self.instrs.len();
                self.instrs.push(Instr::Jump(usize::MAX));

                let false_start = self.instrs.len();
                if let Instr::JumpIfFalse(_, target) = &mut self.instrs[jump_if_false] {
                    *target = false_start;
                }

                self.emit_after(out(id, 1)); // false path

                let end = self.instrs.len();
                if let Instr::Jump(target) = &mut self.instrs[jump_to_end] {
                    *target = end;
                }
            }
            NodeKind::EventStart => {
                self.diags.push(Diagnostic::at(
                    id,
                    "Event start cannot appear in the middle of an execution chain",
                ));
            }
            other => {
                self.diags.push(Diagnostic::at(
                    id,
                    format!("{} is a value node and cannot be run as a step", other.title()),
                ));
            }
        }

        self.exec_path.pop();
    }

    /// Compiles whatever feeds input pin `index` of `node`.
    fn input_expr(&mut self, node: NodeId, index: usize, expected: DataType) -> Expr {
        let pin = PinRef {
            node,
            side: Side::In,
            index,
        };
        let pin_name = self
            .graph
            .node(node)
            .and_then(|n| n.pin(Side::In, index))
            .map(|p| p.name)
            .unwrap_or_else(|| index.to_string());

        let Some(source) = self.graph.source_of(pin) else {
            let title = self
                .graph
                .kind_of(node)
                .map(|k| k.title())
                .unwrap_or_default();
            self.diags.push(Diagnostic::at(
                node,
                format!("'{pin_name}' on {title} has nothing wired into it"),
            ));
            return Expr::Lit(expected.default_value());
        };

        self.output_expr(source, expected)
    }

    /// Compiles the value produced by an output pin.
    fn output_expr(&mut self, source: PinRef, expected: DataType) -> Expr {
        let id = source.node;
        if self.data_path.contains(&id) {
            self.diags.push(Diagnostic::at(
                id,
                "data wires form a loop; a value cannot depend on itself",
            ));
            return Expr::Lit(expected.default_value());
        }
        let Some(kind) = self.graph.kind_of(id).cloned() else {
            return Expr::Lit(expected.default_value());
        };
        self.data_path.push(id);

        let expr = match kind {
            NodeKind::LitInt(v) => Expr::Lit(Value::Int(v)),
            NodeKind::LitFloat(v) => Expr::Lit(Value::Float(v)),
            NodeKind::LitBool(v) => Expr::Lit(Value::Bool(v)),
            NodeKind::LitStr(ref v) => Expr::Lit(Value::Str(v.clone())),
            NodeKind::GetVar { ref name, .. } => {
                if !self.graph.vars.contains_key(name) {
                    self.diags
                        .push(Diagnostic::at(id, format!("no variable named '{name}'")));
                }
                Expr::GetVar(name.clone())
            }
            NodeKind::LessThan => {
                let a = self.input_expr(id, 0, DataType::Int);
                let b = self.input_expr(id, 1, DataType::Int);
                Expr::LessThan(Box::new(a), Box::new(b))
            }
            other => {
                self.diags.push(Diagnostic::at(
                    id,
                    format!("{} does not produce a value", other.title()),
                ));
                Expr::Lit(expected.default_value())
            }
        };

        self.data_path.pop();
        expr
    }
}

fn out(node: NodeId, index: usize) -> PinRef {
    PinRef {
        node,
        side: Side::Out,
        index,
    }
}
