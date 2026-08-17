//! Turns a node graph into a flat list of instructions.
//!
//! This is what the hammer button runs. Compiling is deliberately separate from
//! running: it walks the execution wires once, reports every problem it finds,
//! and produces a `Program` that the VM can execute without ever looking at the
//! graph again.

use crate::graph::{Graph, NodeId, NodeKind, PinRef, Side};
use crate::types::{DataType, Value};
use std::collections::BTreeMap;

/// Which part of the language a problem belongs to.
///
/// Named after what the user sees on screen, not after compiler phases: the grey
/// execution chain, the coloured data wires, and the variables panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Area {
    /// The grey wires — the order steps happen in.
    Flow,
    /// The coloured wires — values travelling between pins.
    Wire,
    /// Variables.
    Name,
}

impl Area {
    fn as_str(self) -> &'static str {
        match self {
            Area::Flow => "FLOW",
            Area::Wire => "WIRE",
            Area::Name => "NAME",
        }
    }
}

/// A stable name for one kind of problem, like `CP-WIRE-01`.
///
/// Stable is the point: it can be searched for, linked to, and eventually hung with an
/// explanation of the rule behind it. The wording of a message may improve; the code it
/// carries should not change under it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Code {
    pub area: Area,
    pub number: u8,
}

impl Code {
    pub const fn new(area: Area, number: u8) -> Code {
        Code { area, number }
    }

    pub fn render(self) -> String {
        format!("CP-{}-{:02}", self.area.as_str(), self.number)
    }
}

/// No Event start node, so nothing says where to begin.
pub const NO_START: Code = Code::new(Area::Flow, 1);
/// More than one Event start node.
pub const MANY_STARTS: Code = Code::new(Area::Flow, 2);
/// Execution wires lead back on themselves.
pub const EXEC_LOOP: Code = Code::new(Area::Flow, 3);
/// An Event start wired into the middle of a chain.
pub const START_IN_CHAIN: Code = Code::new(Area::Flow, 4);
/// A value node placed in the execution chain.
pub const VALUE_AS_STEP: Code = Code::new(Area::Flow, 5);
/// An input pin with nothing wired into it.
pub const EMPTY_PIN: Code = Code::new(Area::Wire, 1);
/// Data wires lead back on themselves.
pub const DATA_LOOP: Code = Code::new(Area::Wire, 2);
/// Something in a value position that produces no value.
pub const NOT_A_VALUE: Code = Code::new(Area::Wire, 3);
/// A node refers to a variable that does not exist.
pub const NO_SUCH_VAR: Code = Code::new(Area::Name, 1);

impl Code {
    /// The rule this code enforces, stated once so a reader learns the language rather
    /// than only this instance of breaking it.
    ///
    /// Written by hand, in the reader's terms. Generating these from the compiler's own
    /// conditions would produce something true and useless — the check behind an empty
    /// pin is "source_of returned None", which teaches nobody what Cat Paws expects.
    pub fn rule(self) -> &'static str {
        match (self.area, self.number) {
            (Area::Flow, 1) => "a program starts at an Event start node, so every program needs exactly one.",
            (Area::Flow, 2) => "a program has one beginning, so only one Event start may be on the canvas.",
            (Area::Flow, 3) => "the grey wires run forwards only. A step cannot lead back to one that already ran.",
            (Area::Flow, 4) => "an Event start begins a program, so nothing may lead into it.",
            (Area::Flow, 5) => "grey wires join steps and coloured wires carry values. A node that produces a value is not a step.",
            (Area::Wire, 1) => "every coloured input needs a wire. There is no default value to fall back on.",
            (Area::Wire, 2) => "a value is worked out from the wires feeding it, so it cannot be worked out from itself.",
            (Area::Wire, 3) => "only a node with a coloured output produces a value that something else can read.",
            (Area::Name, 1) => "a variable is created in the Variables panel before a node can read or write it.",
            _ => "",
        }
    }
}

/// A problem found while compiling.
///
/// Two separate sentences, deliberately. `message` says what is wrong; `fix` says what to
/// do about it. Someone who already knows the language reads the first and stops; someone
/// meeting it for the first time needs the second, and a single blended sentence tends to
/// serve neither.
///
/// `node` is the third piece — the editor highlights it and the panel entry jumps to it —
/// which is this language's equivalent of a line number.
#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub code: Code,
    pub node: Option<NodeId>,
    pub message: String,
    pub fix: String,
}

impl Diagnostic {
    pub fn at(
        code: Code,
        node: NodeId,
        message: impl Into<String>,
        fix: impl Into<String>,
    ) -> Diagnostic {
        Diagnostic {
            code,
            node: Some(node),
            message: message.into(),
            fix: fix.into(),
        }
    }

    pub fn global(code: Code, message: impl Into<String>, fix: impl Into<String>) -> Diagnostic {
        Diagnostic {
            code,
            node: None,
            message: message.into(),
            fix: fix.into(),
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
                NO_START,
                "there is no Event start node, so nothing says where the program begins",
                "right-click the canvas and add an Event start, then wire it to the first step",
            )])
        }
        [one] => *one,
        many => {
            return Err(many
                .iter()
                .map(|id| {
                    Diagnostic::at(
                MANY_STARTS,
                        *id,
                        "there is more than one Event start node, so it is unclear which one begins the program",
                        "delete all but one of them",
                    )
                })
                .collect())
        }
    };

    let mut c = Compiler {
        graph,
        instrs: Vec::new(),
        diags: Vec::new(),
        exec_path: Vec::new(),
        values: Values::new(graph),
    };

    c.emit_after(PinRef {
        node: start,
        side: Side::Out,
        index: 0,
    });

    c.diags.append(&mut c.values.diags);
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
    values: Values<'a>,
}

/// Turns data wires into `Expr` trees.
///
/// Separate from the execution walk so that any backend can reuse it. There is one
/// definition of what a data wire means, and the WASM emitter shares it rather than
/// keeping a second copy that could drift.
pub(crate) struct Values<'a> {
    graph: &'a Graph,
    pub(crate) diags: Vec<Diagnostic>,
    /// Nodes on the current data path, used to catch circular values.
    data_path: Vec<NodeId>,
}

impl<'a> Values<'a> {
    pub(crate) fn new(graph: &'a Graph) -> Values<'a> {
        Values {
            graph,
            diags: Vec::new(),
            data_path: Vec::new(),
        }
    }
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
                EXEC_LOOP,
                id,
                "the execution wires lead back to a node they already passed through, so this program would never finish",
                "break the loop by unplugging one of the grey wires — repeating a step is not supported yet",
            ));
            return;
        }
        let Some(kind) = self.graph.kind_of(id).cloned() else {
            return;
        };
        self.exec_path.push(id);

        match kind {
            NodeKind::Print => {
                let text = self.values.input_expr(id, 1, DataType::Str);
                self.instrs.push(Instr::Print(text));
                self.emit_after(out(id, 0));
            }
            NodeKind::SetVar { ref name, ty } => {
                let value = self.values.input_expr(id, 1, ty);
                self.instrs.push(Instr::SetVar(name.clone(), value));
                self.emit_after(out(id, 0));
            }
            NodeKind::Branch => {
                let cond = self.values.input_expr(id, 1, DataType::Bool);

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
                START_IN_CHAIN,
                    id,
                    "an Event start is where the program begins, so it cannot also be a step in the middle of one",
                    "unplug the grey wire going into this node",
                ));
            }
            other => {
                self.diags.push(Diagnostic::at(
                VALUE_AS_STEP,
                    id,
                    format!(
                        "{} produces a value, so it cannot sit in the grey execution chain",
                        other.title()
                    ),
                    "wire it into a coloured input pin instead of a grey one",
                ));
            }
        }

        self.exec_path.pop();
    }

}

impl<'a> Values<'a> {
    /// Compiles whatever feeds input pin `index` of `node`.
    pub(crate) fn input_expr(&mut self, node: NodeId, index: usize, expected: DataType) -> Expr {
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
                EMPTY_PIN,
                node,
                format!("'{pin_name}' on {title} has nothing wired into it, so there is no value to use"),
                format!(
                    "wire something into '{pin_name}' — it takes {}",
                    a_an(expected.label())
                ),
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
                DATA_LOOP,
                id,
                "this value is worked out from itself, going round in a circle for ever",
                "unplug one of the coloured wires in the loop",
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
                        .push(Diagnostic::at(
                NO_SUCH_VAR,
                            id,
                            format!("there is no variable called '{name}'"),
                            "add it in the Variables panel, or pick a different one on this node",
                        ));
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
                NOT_A_VALUE,
                    id,
                    format!("{} does not produce a value, so nothing can read from it", other.title()),
                    "wire a value node into this pin instead",
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

/// "an integer", "a float" — small, but a diagnostic that says "it takes integer" reads
/// like a machine wrote it.
fn a_an(label: &str) -> String {
    let article = if label.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
    format!("{article} {label}")
}
