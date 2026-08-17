//! The graph the user builds: nodes, pins and the wires between them.
//!
//! This module knows nothing about drawing. Node positions are plain `(f32, f32)`
//! so that the editor can decide what a "position" means on screen.

use crate::types::{DataType, Value};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug, Hash)]
pub struct NodeId(pub u64);

/// Whether a pin sits on the left of a node (an input) or the right (an output).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Side {
    In,
    Out,
}

/// A pin is either part of the execution chain (grey wires, "what happens next")
/// or carries a value of some type (coloured wires).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PinKind {
    Exec,
    Data(DataType),
}

#[derive(Clone, Debug)]
pub struct Pin {
    pub name: String,
    pub kind: PinKind,
}

impl Pin {
    fn exec(name: &str) -> Pin {
        Pin {
            name: name.to_string(),
            kind: PinKind::Exec,
        }
    }

    fn data(name: &str, ty: DataType) -> Pin {
        Pin {
            name: name.to_string(),
            kind: PinKind::Data(ty),
        }
    }
}

/// Points at one specific pin on one specific node.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct PinRef {
    pub node: NodeId,
    pub side: Side,
    pub index: usize,
}

/// A wire. `from` is always an output pin, `to` is always an input pin.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Link {
    pub from: PinRef,
    pub to: PinRef,
}

/// Node categories exist only so the editor can colour node headers consistently.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Category {
    Event,
    Flow,
    Action,
    Pure,
    Variable,
}

/// The arithmetic a node can do. Kept separate from `NodeKind` so one variant covers
/// all four operators rather than four near-identical ones.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ArithOp {
    Add,
    Subtract,
    Multiply,
    Divide,
}

impl ArithOp {
    pub fn symbol(self) -> &'static str {
        match self {
            ArithOp::Add => "+",
            ArithOp::Subtract => "−",
            ArithOp::Multiply => "×",
            ArithOp::Divide => "÷",
        }
    }

    pub fn word(self) -> &'static str {
        match self {
            ArithOp::Add => "add",
            ArithOp::Subtract => "subtract",
            ArithOp::Multiply => "multiply",
            ArithOp::Divide => "divide",
        }
    }

    pub const ALL: [ArithOp; 4] = [
        ArithOp::Add,
        ArithOp::Subtract,
        ArithOp::Multiply,
        ArithOp::Divide,
    ];
}

/// Every kind of node Cat Paws can execute. Adding a language feature means
/// adding a variant here, then handling it in `pins`, `compile` and `vm`.
#[derive(Clone, Debug, PartialEq)]
pub enum NodeKind {
    /// Where execution begins. Exactly one is required to compile.
    EventStart,
    /// Sends execution down one of two paths depending on a boolean.
    Branch,
    /// Runs the chain on its `body` pin a fixed number of times, then carries on
    /// down `then`.
    ///
    /// The body is an ordinary chain that ends, exactly like a Branch arm — so
    /// execution wires still never form a cycle, and the compiler's rule against
    /// cycles stays a flat rule rather than a conditional one. It is also the shape
    /// a Scratch user already knows: blocks sit *inside* the loop's mouth.
    ///
    /// `times` is read once, before the first pass. Changing the variable that fed
    /// it while the loop runs does not lengthen or shorten the loop, which is what
    /// Scratch does too.
    Repeat,
    /// Writes a line to the output console.
    ///
    /// Carries the type it prints, so a number can be shown without first being turned
    /// into text. Building text at runtime would need a heap, and nothing in the
    /// language needs one yet — see docs/memory.md.
    Print { ty: DataType },
    /// Pure comparison: true when a < b.
    LessThan,
    /// Arithmetic on two numbers of the same type.
    ///
    /// The type is part of the node rather than worked out from what is wired in:
    /// a pin whose type is fixed can refuse a wrong wire while it is being dragged,
    /// which is the whole reason the pins are typed.
    Arith { op: ArithOp, ty: DataType },
    /// Reads the current value of a variable.
    GetVar { name: String, ty: DataType },
    /// Writes a value into a variable.
    SetVar { name: String, ty: DataType },
    LitInt(i64),
    LitFloat(f64),
    LitBool(bool),
    LitStr(String),
}

impl NodeKind {
    pub fn title(&self) -> String {
        match self {
            NodeKind::EventStart => "Event start".to_string(),
            NodeKind::Branch => "Branch".to_string(),
            NodeKind::Repeat => "Repeat".to_string(),
            NodeKind::Print { ty } => match ty {
                DataType::Str => "Print".to_string(),
                _ => format!("Print {}", ty.label()),
            },
            NodeKind::LessThan => "Less than".to_string(),
            NodeKind::Arith { op, .. } => format!("{} {}", op.word(), op.symbol()),
            NodeKind::GetVar { name, .. } => name.clone(),
            NodeKind::SetVar { name, .. } => format!("Set {name}"),
            NodeKind::LitInt(v) => v.to_string(),
            NodeKind::LitFloat(v) => v.to_string(),
            NodeKind::LitBool(v) => v.to_string(),
            NodeKind::LitStr(v) => format!("\"{v}\""),
        }
    }

    /// The small grey line under the title.
    pub fn subtitle(&self) -> String {
        match self {
            NodeKind::EventStart => "entry point".to_string(),
            NodeKind::Branch => "true / false".to_string(),
            NodeKind::Repeat => "do it n times".to_string(),
            NodeKind::Print { .. } => "write a line".to_string(),
            NodeKind::LessThan => "number compare".to_string(),
            NodeKind::Arith { ty, .. } => format!("{} maths", ty.label()),
            NodeKind::GetVar { ty, .. } => format!("{} variable", ty.label()),
            NodeKind::SetVar { ty, .. } => format!("set {}", ty.label()),
            NodeKind::LitInt(_) => "integer".to_string(),
            NodeKind::LitFloat(_) => "float".to_string(),
            NodeKind::LitBool(_) => "boolean".to_string(),
            NodeKind::LitStr(_) => "string".to_string(),
        }
    }

    pub fn category(&self) -> Category {
        match self {
            NodeKind::EventStart => Category::Event,
            NodeKind::Branch | NodeKind::Repeat => Category::Flow,
            NodeKind::Print { .. } | NodeKind::SetVar { .. } => Category::Action,
            NodeKind::LessThan | NodeKind::Arith { .. } => Category::Pure,
            NodeKind::GetVar { .. }
            | NodeKind::LitInt(_)
            | NodeKind::LitFloat(_)
            | NodeKind::LitBool(_)
            | NodeKind::LitStr(_) => Category::Variable,
        }
    }

    pub fn inputs(&self) -> Vec<Pin> {
        match self {
            NodeKind::EventStart => vec![],
            NodeKind::Branch => vec![
                Pin::exec("in"),
                Pin::data("condition", DataType::Bool),
            ],
            NodeKind::Repeat => vec![Pin::exec("in"), Pin::data("times", DataType::Int)],
            NodeKind::Print { ty } => vec![
                Pin::exec("in"),
                Pin::data(if *ty == DataType::Str { "text" } else { "value" }, *ty),
            ],
            NodeKind::LessThan => vec![
                Pin::data("a", DataType::Int),
                Pin::data("b", DataType::Int),
            ],
            NodeKind::Arith { ty, .. } => vec![Pin::data("a", *ty), Pin::data("b", *ty)],
            NodeKind::GetVar { .. } => vec![],
            NodeKind::SetVar { ty, .. } => vec![Pin::exec("in"), Pin::data("value", *ty)],
            NodeKind::LitInt(_)
            | NodeKind::LitFloat(_)
            | NodeKind::LitBool(_)
            | NodeKind::LitStr(_) => vec![],
        }
    }

    pub fn outputs(&self) -> Vec<Pin> {
        match self {
            NodeKind::EventStart => vec![Pin::exec("then")],
            NodeKind::Branch => vec![Pin::exec("true"), Pin::exec("false")],
            // "body" is the chain that repeats; "then" is what happens once it has
            // finished repeating. Two execution outputs, like Branch, so the editor
            // needs nothing new to draw it.
            NodeKind::Repeat => vec![Pin::exec("body"), Pin::exec("then")],
            NodeKind::Print { .. } => vec![Pin::exec("then")],
            NodeKind::LessThan => vec![Pin::data("result", DataType::Bool)],
            NodeKind::Arith { ty, .. } => vec![Pin::data("result", *ty)],
            NodeKind::GetVar { ty, .. } => vec![Pin::data("value", *ty)],
            NodeKind::SetVar { .. } => vec![Pin::exec("then")],
            NodeKind::LitInt(_) => vec![Pin::data("value", DataType::Int)],
            NodeKind::LitFloat(_) => vec![Pin::data("value", DataType::Float)],
            NodeKind::LitBool(_) => vec![Pin::data("value", DataType::Bool)],
            NodeKind::LitStr(_) => vec![Pin::data("value", DataType::Str)],
        }
    }
}

#[derive(Clone, Debug)]
pub struct Node {
    pub id: NodeId,
    pub kind: NodeKind,
    pub pos: (f32, f32),
}

impl Node {
    pub fn pin(&self, side: Side, index: usize) -> Option<Pin> {
        let pins = match side {
            Side::In => self.kind.inputs(),
            Side::Out => self.kind.outputs(),
        };
        pins.get(index).cloned()
    }
}

/// A variable declared on the graph, with the value it starts each run with.
#[derive(Clone, Debug)]
pub struct VarDecl {
    pub ty: DataType,
    pub initial: Value,
}

#[derive(Clone, Debug, Default)]
pub struct Graph {
    nodes: BTreeMap<NodeId, Node>,
    links: Vec<Link>,
    pub vars: BTreeMap<String, VarDecl>,
    next_id: u64,
}

impl Graph {
    pub fn new() -> Graph {
        Graph::default()
    }

    pub fn add_node(&mut self, kind: NodeKind, pos: (f32, f32)) -> NodeId {
        let id = NodeId(self.next_id);
        self.next_id += 1;
        self.nodes.insert(id, Node { id, kind, pos });
        id
    }

    pub fn remove_node(&mut self, id: NodeId) {
        self.nodes.remove(&id);
        self.links.retain(|l| l.from.node != id && l.to.node != id);
    }

    pub fn node(&self, id: NodeId) -> Option<&Node> {
        self.nodes.get(&id)
    }

    pub fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        self.nodes.get_mut(&id)
    }

    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.nodes.values()
    }

    pub fn node_ids(&self) -> Vec<NodeId> {
        self.nodes.keys().copied().collect()
    }

    pub fn links(&self) -> &[Link] {
        &self.links
    }

    pub fn kind_of(&self, id: NodeId) -> Option<&NodeKind> {
        self.nodes.get(&id).map(|n| &n.kind)
    }

    pub fn pin_kind(&self, r: PinRef) -> Option<PinKind> {
        self.nodes.get(&r.node)?.pin(r.side, r.index).map(|p| p.kind)
    }

    /// Whether a wire from `from` to `to` is legal. This is the single source of
    /// truth for connection validity — the editor calls it while a wire is being
    /// dragged so an illegal connection simply cannot be dropped.
    pub fn can_connect(&self, from: PinRef, to: PinRef) -> Result<(), &'static str> {
        if from.node == to.node {
            return Err("a node cannot wire into itself");
        }
        if from.side != Side::Out || to.side != Side::In {
            return Err("wires run from an output pin to an input pin");
        }
        let from_kind = self.pin_kind(from).ok_or("that output pin does not exist")?;
        let to_kind = self.pin_kind(to).ok_or("that input pin does not exist")?;
        match (from_kind, to_kind) {
            (PinKind::Exec, PinKind::Exec) => Ok(()),
            (PinKind::Data(a), PinKind::Data(b)) if a == b => Ok(()),
            (PinKind::Data(a), PinKind::Data(b)) => {
                // Leaked as a &'static str for simplicity; the editor shows its
                // own richer message using the two pin types.
                let _ = (a, b);
                Err("those two pins are different types")
            }
            _ => Err("an execution pin cannot join a data pin"),
        }
    }

    /// Adds a wire, replacing any wire this would conflict with.
    ///
    /// An execution output may only lead to one place, and a data input may only
    /// be fed by one wire — so connecting to an occupied pin replaces what was
    /// there rather than failing. That matches how Blueprints behave.
    pub fn connect(&mut self, from: PinRef, to: PinRef) -> Result<(), &'static str> {
        self.can_connect(from, to)?;
        let from_is_exec = matches!(self.pin_kind(from), Some(PinKind::Exec));
        self.links.retain(|l| {
            let replaced_output = from_is_exec && l.from == from;
            let replaced_input = l.to == to;
            !(replaced_output || replaced_input)
        });
        self.links.push(Link { from, to });
        Ok(())
    }

    pub fn disconnect_pin(&mut self, pin: PinRef) {
        self.links.retain(|l| l.from != pin && l.to != pin);
    }

    /// The node and pin feeding this input pin, if anything does.
    pub fn source_of(&self, input: PinRef) -> Option<PinRef> {
        self.links.iter().find(|l| l.to == input).map(|l| l.from)
    }

    /// The input pin an execution output leads to, if any.
    pub fn target_of(&self, output: PinRef) -> Option<PinRef> {
        self.links.iter().find(|l| l.from == output).map(|l| l.to)
    }

    pub fn declare_var(&mut self, name: String, ty: DataType) {
        self.vars.insert(
            name,
            VarDecl {
                ty,
                initial: ty.default_value(),
            },
        );
    }
}
