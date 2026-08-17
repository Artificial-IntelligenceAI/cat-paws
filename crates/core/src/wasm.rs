//! Turns a node graph into a real WebAssembly module.
//!
//! This is the compiler proper. `compile.rs` produces our own bytecode for the
//! interpreter in `vm.rs`; this produces a `.wasm` binary that a browser runs directly,
//! with no interpreter of ours anywhere in the picture.
//!
//! # Why this walks the graph rather than the bytecode
//!
//! The bytecode is a flat list with absolute jump targets. WebAssembly has no jumps to
//! an address — its control flow is *structured*: nested `block`, `loop` and `if`, and a
//! branch may only break outward to an enclosing one. Recovering that structure from
//! flat jumps is a real algorithm (the "relooper" problem that Emscripten had to solve).
//!
//! The graph never lost the structure in the first place: a branch node has a true path
//! and a false path, and execution wires cannot form a cycle. So the graph maps onto
//! WASM's `if`/`else` directly, and going via the bytecode would mean throwing that
//! structure away and paying to rebuild it.
//!
//! # How values are represented
//!
//! | Cat Paws | WebAssembly |
//! |---|---|
//! | `Int` | `i64` |
//! | `Float` | `f64` |
//! | `Bool` | `i32`, 0 or 1 |
//! | `Str` | `i32` pointer into linear memory |
//!
//! A string is a `[len: u32][utf8 bytes]` record in a data segment. Every string in the
//! language comes from a literal — there is no way to build a new one at runtime yet —
//! so they are all known while compiling and can simply be laid out in memory up front.
//!
//! Printing is an *imported* function. WebAssembly has no I/O at all: a module can only
//! call what the host gives it. The host supplies `env.print`, which is handed a pointer
//! and reads the string out of the module's exported memory.

use std::collections::BTreeMap;

use wasm_encoder::{
    BlockType, CodeSection, ConstExpr, DataSection, EntityType, ExportKind, ExportSection,
    Function, FunctionSection, ImportSection, Instruction, MemArg, MemorySection, MemoryType,
    Module, TypeSection, ValType,
};

use crate::compile::{
    Diagnostic, Expr, Values, EXEC_LOOP, MANY_STARTS, NO_START, NO_SUCH_VAR, START_IN_CHAIN,
    VALUE_AS_STEP,
};
use crate::graph::{ArithOp, Graph, NodeId, NodeKind, PinRef, Side};
use crate::types::{DataType, Value};

/// Where the string data starts. Offset 0 is left alone so that a null pointer is never
/// a valid string.
const STRING_BASE: u32 = 8;

/// Compile a graph to a WebAssembly module.
///
/// The bytes are a complete `.wasm` file: hand them to `WebAssembly.instantiate` with an
/// `env.print` function and call the exported `main`.
pub fn emit(graph: &Graph) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let start = entry_point(graph)?;

    let mut e = Emitter {
        graph,
        values: Values::new(graph),
        diags: Vec::new(),
        locals: BTreeMap::new(),
        strings: StringTable::default(),
        body: Vec::new(),
        exec_path: Vec::new(),
    };

    // Variables become function locals, in a fixed order so an index means one thing.
    let mut local_types = Vec::new();
    for (index, (name, decl)) in graph.vars.iter().enumerate() {
        e.locals.insert(name.clone(), index as u32);
        local_types.push(val_type(decl.ty));
    }

    // Each run starts from the declared initial values.
    for (name, decl) in graph.vars.iter() {
        let index = e.locals[name];
        e.push_const(&decl.initial);
        e.body.push(Instruction::LocalSet(index));
    }

    e.walk_from(PinRef {
        node: start,
        side: Side::Out,
        index: 0,
    });

    e.diags.append(&mut e.values.diags);
    if !e.diags.is_empty() {
        return Err(e.diags);
    }

    Ok(e.finish(local_types))
}

/// Every string literal in the program, laid out in linear memory.
#[derive(Default)]
struct StringTable {
    bytes: Vec<u8>,
    offsets: BTreeMap<String, u32>,
}

impl StringTable {
    /// The address of this string, adding it to the data segment the first time it is
    /// seen. Identical strings share one record.
    fn intern(&mut self, text: &str) -> u32 {
        if let Some(offset) = self.offsets.get(text) {
            return *offset;
        }
        let offset = STRING_BASE + self.bytes.len() as u32;
        self.bytes.extend_from_slice(&(text.len() as u32).to_le_bytes());
        self.bytes.extend_from_slice(text.as_bytes());
        // Keep every record 4-byte aligned so the length prefix is always readable.
        while self.bytes.len() % 4 != 0 {
            self.bytes.push(0);
        }
        self.offsets.insert(text.to_string(), offset);
        offset
    }
}

struct Emitter<'a> {
    graph: &'a Graph,
    values: Values<'a>,
    diags: Vec<Diagnostic>,
    /// Variable name to local index.
    locals: BTreeMap<String, u32>,
    strings: StringTable,
    body: Vec<Instruction<'static>>,
    /// Nodes on the current execution path, so a cycle is reported rather than hung on.
    exec_path: Vec<NodeId>,
}

impl<'a> Emitter<'a> {
    /// Emit whatever this execution output leads to. An unconnected output simply ends
    /// that path, which is not an error.
    fn walk_from(&mut self, out_pin: PinRef) {
        if let Some(target) = self.graph.target_of(out_pin) {
            self.walk_node(target.node);
        }
    }

    fn walk_node(&mut self, id: NodeId) {
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
                self.push_expr(&text);
                // Import 0: the host's print.
                self.body.push(Instruction::Call(0));
                self.walk_from(out(id, 0));
            }
            NodeKind::SetVar { ref name, ty } => {
                let value = self.values.input_expr(id, 1, ty);
                self.push_expr(&value);
                match self.locals.get(name) {
                    Some(index) => self.body.push(Instruction::LocalSet(*index)),
                    None => {
                        self.diags.push(Diagnostic::at(
                NO_SUCH_VAR,
                            id,
                            format!("there is no variable called '{name}'"),
                            "add it in the Variables panel, or pick a different one on this node",
                        ));
                        self.body.push(Instruction::Drop);
                    }
                }
                self.walk_from(out(id, 0));
            }
            NodeKind::Branch => {
                let cond = self.values.input_expr(id, 1, DataType::Bool);
                self.push_expr(&cond);
                // Straight from the graph's own shape: the true path is the `if` arm and
                // the false path is the `else` arm. No jump targets to patch.
                self.body.push(Instruction::If(BlockType::Empty));
                self.walk_from(out(id, 0));
                self.body.push(Instruction::Else);
                self.walk_from(out(id, 1));
                self.body.push(Instruction::End);
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

    fn push_const(&mut self, value: &Value) {
        match value {
            Value::Int(v) => self.body.push(Instruction::I64Const(*v)),
            Value::Float(v) => self.body.push(Instruction::F64Const((*v).into())),
            Value::Bool(v) => self.body.push(Instruction::I32Const(*v as i32)),
            Value::Str(text) => {
                let offset = self.strings.intern(text);
                self.body.push(Instruction::I32Const(offset as i32));
            }
        }
    }

    /// Push the value of an expression onto the WebAssembly stack.
    ///
    /// WASM is a stack machine, so a postfix walk of the expression tree *is* the
    /// instruction sequence: operands first, then the operator.
    fn push_expr(&mut self, expr: &Expr) {
        match expr {
            Expr::Lit(value) => self.push_const(value),
            Expr::GetVar(name) => match self.locals.get(name) {
                Some(index) => self.body.push(Instruction::LocalGet(*index)),
                None => {
                    self.diags.push(Diagnostic::global(
                NO_SUCH_VAR,
                        format!("there is no variable called '{name}'"),
                        "add it in the Variables panel, or pick a different one on this node",
                    ));
                    self.body.push(Instruction::I32Const(0));
                }
            },
            Expr::LessThan(a, b) => {
                self.push_expr(a);
                self.push_expr(b);
                // Both pins on Less than are integers, so this is the signed 64-bit
                // comparison. It leaves an i32 0 or 1, which is what `if` wants.
                self.body.push(Instruction::I64LtS);
            }
            Expr::Arith(op, ty, a, b) => {
                self.push_expr(a);
                self.push_expr(b);
                // One instruction per operator and type. Whole-number division is
                // signed, and `i64.div_s` traps on a zero divisor — the program stops
                // rather than inventing an answer, which is what the interpreter does
                // too.
                self.body.push(match (ty, op) {
                    (DataType::Float, ArithOp::Add) => Instruction::F64Add,
                    (DataType::Float, ArithOp::Subtract) => Instruction::F64Sub,
                    (DataType::Float, ArithOp::Multiply) => Instruction::F64Mul,
                    (DataType::Float, ArithOp::Divide) => Instruction::F64Div,
                    (_, ArithOp::Add) => Instruction::I64Add,
                    (_, ArithOp::Subtract) => Instruction::I64Sub,
                    (_, ArithOp::Multiply) => Instruction::I64Mul,
                    (_, ArithOp::Divide) => Instruction::I64DivS,
                });
            }
        }
    }

    /// Assemble the sections into a finished module.
    fn finish(mut self, local_types: Vec<ValType>) -> Vec<u8> {
        let mut module = Module::new();

        // Two signatures: the host's print takes a pointer, and main takes nothing.
        let mut types = TypeSection::new();
        types.ty().function([ValType::I32], []);
        types.ty().function([], []);
        module.section(&types);

        let mut imports = ImportSection::new();
        imports.import("env", "print", EntityType::Function(0));
        module.section(&imports);

        let mut functions = FunctionSection::new();
        functions.function(1);
        module.section(&functions);

        // One page is 64KiB, which is far more than the string table needs.
        let mut memories = MemorySection::new();
        memories.memory(MemoryType {
            minimum: 1,
            maximum: None,
            memory64: false,
            shared: false,
            page_size_log2: None,
        });
        module.section(&memories);

        let mut exports = ExportSection::new();
        // Function 0 is the import, so ours is 1.
        exports.export("main", ExportKind::Func, 1);
        // The host reads printed strings straight out of this.
        exports.export("memory", ExportKind::Memory, 0);
        module.section(&exports);

        let mut code = CodeSection::new();
        let mut f = Function::new(local_types.into_iter().map(|t| (1, t)));
        for instruction in &self.body {
            f.instruction(instruction);
        }
        f.instruction(&Instruction::End);
        code.function(&f);
        module.section(&code);

        if !self.strings.bytes.is_empty() {
            let mut data = DataSection::new();
            data.active(
                0,
                &ConstExpr::i32_const(STRING_BASE as i32),
                std::mem::take(&mut self.strings.bytes),
            );
            module.section(&data);
        }

        module.finish()
    }
}

/// The single `Event start` a program must have.
fn entry_point(graph: &Graph) -> Result<NodeId, Vec<Diagnostic>> {
    let starts: Vec<NodeId> = graph
        .nodes()
        .filter(|n| n.kind == NodeKind::EventStart)
        .map(|n| n.id)
        .collect();

    match starts.as_slice() {
        [] => Err(vec![Diagnostic::global(
                NO_START,
            "there is no Event start node, so nothing says where the program begins",
            "right-click the canvas and add an Event start, then wire it to the first step",
        )]),
        [one] => Ok(*one),
        many => Err(many
            .iter()
            .map(|id| {
                Diagnostic::at(
                MANY_STARTS,
                    *id,
                    "there is more than one Event start node, so it is unclear which one begins the program",
                    "delete all but one of them",
                )
            })
            .collect()),
    }
}

fn val_type(ty: DataType) -> ValType {
    match ty {
        DataType::Int => ValType::I64,
        DataType::Float => ValType::F64,
        // A boolean and a string pointer are both i32 to WebAssembly, which has no
        // narrower integer and no notion of a pointer.
        DataType::Bool | DataType::Str => ValType::I32,
    }
}

fn out(node: NodeId, index: usize) -> PinRef {
    PinRef {
        node,
        side: Side::Out,
        index,
    }
}

/// Silence the unused-import warning when the data section is empty.
#[allow(unused)]
fn _memarg_is_used(_: MemArg) {}
