//! The written form: type a program, get nodes.
//!
//! This is an authoring aid, not a file format. It reads text and adds nodes to a graph,
//! already wired. Nothing reads text back out of a graph, nothing is saved, and it has no
//! say over where nodes sit — see `docs/written.md`.
//!
//! Because generation only goes one way, none of the round-trip problems exist: there is
//! no node identity to track across edits, no sidecar file, and nothing that can drift out
//! of step with anything else.
//!
//! ```text
//! declare 'health' = integer '20'
//!
//! if 'health' < integer '50' {
//!     print string 'low health'
//! } else {
//!     print string 'fine'
//! }
//! ```
//!
//! One line makes as many nodes as the idea needs. The first line above is three things —
//! a variable, a literal and a Set — and the `if` is seven more.

use crate::graph::{ArithOp, Graph, NodeId, NodeKind, PinRef, Side};
use crate::types::{DataType, Value};

/// Something wrong with the text, with the line it was on.
#[derive(Clone, Debug, PartialEq)]
pub struct Problem {
    pub line: usize,
    pub message: String,
    pub fix: String,
}

impl Problem {
    fn new(line: usize, message: impl Into<String>, fix: impl Into<String>) -> Problem {
        Problem {
            line,
            message: message.into(),
            fix: fix.into(),
        }
    }
}

/// Read `text` and add the nodes it describes to `graph`.
///
/// Existing nodes are left alone: what is written is added beside them, wired among
/// itself. Nothing you built by hand can be destroyed by typing.
pub fn generate(graph: &mut Graph, text: &str) -> Result<Vec<NodeId>, Vec<Problem>> {
    // Kept so a run that fails partway can put the graph back. Half a program's worth
    // of nodes appearing next to a list of problems would be worse than nothing.
    let vars_before = graph.vars.clone();
    let lines = read_lines(text);
    let mut parser = Parser { lines, at: 0 };
    let body = parser.block(0)?;

    let column = free_column(graph);
    let mut builder = Builder {
        graph,
        made: Vec::new(),
        next: column,
        row: 0.0,
        problems: Vec::new(),
    };
    // A fragment gets its own start only when the canvas has none; otherwise it is left
    // loose for the user to wire in, since adding a second Event start would break a
    // program that already compiles.
    let entry = builder.entry_point();
    let first = builder.statements(&body);
    if let (Some(entry), Some(first)) = (entry, first) {
        builder.wire(entry, 0, first, 0);
    }

    let Builder { made, problems, .. } = builder;
    if problems.is_empty() {
        Ok(made)
    } else {
        for id in made {
            graph.remove_node(id);
        }
        graph.vars = vars_before;
        Err(problems)
    }
}

// ── reading ─────────────────────────────────────────────────────────────────

/// One line of source, with the line number it came from.
struct Line {
    number: usize,
    text: String,
}

/// Strip comments and blank lines, keeping the original numbering so a problem can point
/// at the line the reader actually typed.
fn read_lines(text: &str) -> Vec<Line> {
    text.lines()
        .enumerate()
        .map(|(i, raw)| {
            let body = match raw.find('#') {
                Some(at) => &raw[..at],
                None => raw,
            };
            Line {
                number: i + 1,
                text: body.trim().to_string(),
            }
        })
        .filter(|l| !l.text.is_empty())
        .collect()
}

/// What one line says.
#[derive(Debug, PartialEq)]
enum Stmt {
    Declare { name: String, value: Expr, line: usize },
    Set { name: String, value: Expr, line: usize },
    Print { value: Expr, line: usize },
    If { condition: Expr, then: Vec<Stmt>, otherwise: Vec<Stmt>, line: usize },
    Repeat { times: Expr, body: Vec<Stmt>, line: usize },
}

impl Stmt {
    fn line(&self) -> usize {
        match self {
            Stmt::Declare { line, .. }
            | Stmt::Set { line, .. }
            | Stmt::Print { line, .. }
            | Stmt::If { line, .. }
            | Stmt::Repeat { line, .. } => *line,
        }
    }
}

/// A value, before it becomes nodes.
#[derive(Debug, PartialEq)]
enum Expr {
    Lit(Value),
    Var(String),
    Arith(ArithOp, Box<Expr>, Box<Expr>),
    LessThan(Box<Expr>, Box<Expr>),
}

struct Parser {
    lines: Vec<Line>,
    at: usize,
}

impl Parser {
    fn block(&mut self, depth: usize) -> Result<Vec<Stmt>, Vec<Problem>> {
        let mut out = Vec::new();
        let mut problems = Vec::new();

        while self.at < self.lines.len() {
            let line = &self.lines[self.at];
            if line.text == "}" || line.text.starts_with("} else") {
                if depth == 0 {
                    problems.push(Problem::new(
                        line.number,
                        "there is a closing brace here with nothing open for it to close",
                        "delete it, or add the `if` or `repeat` it was meant to close",
                    ));
                    self.at += 1;
                    continue;
                }
                break;
            }
            match self.statement() {
                Ok(stmt) => out.push(stmt),
                Err(p) => problems.push(p),
            }
        }

        if problems.is_empty() {
            Ok(out)
        } else {
            Err(problems)
        }
    }

    fn statement(&mut self) -> Result<Stmt, Problem> {
        let line = self.lines[self.at].number;
        let text = self.lines[self.at].text.clone();
        self.at += 1;

        if let Some(rest) = text.strip_prefix("declare ") {
            let (name, value) = split_assignment(line, rest)?;
            return Ok(Stmt::Declare { name, value, line });
        }
        if let Some(rest) = text.strip_prefix("set ") {
            let (name, value) = split_assignment(line, rest)?;
            return Ok(Stmt::Set { name, value, line });
        }
        if let Some(rest) = text.strip_prefix("print ") {
            return Ok(Stmt::Print {
                value: parse_expr(line, rest)?,
                line,
            });
        }
        if let Some(rest) = text.strip_prefix("if ") {
            let head = rest.trim_end().strip_suffix('{').ok_or_else(|| {
                Problem::new(
                    line,
                    "this `if` does not open a block",
                    "put a `{` at the end of the line, and a `}` on its own line after the steps",
                )
            })?;
            let condition = parse_expr(line, head)?;
            let then = self.block(1).map_err(|mut p| p.remove(0))?;
            let mut otherwise = Vec::new();
            if let Some(closer) = self.lines.get(self.at) {
                if closer.text.starts_with("} else") {
                    self.at += 1;
                    otherwise = self.block(1).map_err(|mut p| p.remove(0))?;
                }
            }
            match self.lines.get(self.at) {
                Some(l) if l.text == "}" => self.at += 1,
                _ => {
                    return Err(Problem::new(
                        line,
                        "this `if` is never closed",
                        "add a `}` on its own line after the steps inside it",
                    ))
                }
            }
            return Ok(Stmt::If {
                condition,
                then,
                otherwise,
                line,
            });
        }

        if let Some(rest) = text.strip_prefix("repeat ") {
            let head = rest.trim_end().strip_suffix('{').ok_or_else(|| {
                Problem::new(
                    line,
                    "this `repeat` does not open a block",
                    "put a `{` at the end of the line, and a `}` on its own line after the steps",
                )
            })?;
            let times = parse_expr(line, head)?;
            let body = self.block(1).map_err(|mut p| p.remove(0))?;
            match self.lines.get(self.at) {
                Some(l) if l.text == "}" => self.at += 1,
                _ => {
                    return Err(Problem::new(
                        line,
                        "this `repeat` is never closed",
                        "add a `}` on its own line after the steps inside it",
                    ))
                }
            }
            return Ok(Stmt::Repeat { times, body, line });
        }

        Err(Problem::new(
            line,
            format!("'{}' does not start with anything Cat Paws knows", first_word(&text)),
            "a line starts with declare, set, print, if or repeat",
        ))
    }
}

fn first_word(text: &str) -> &str {
    text.split_whitespace().next().unwrap_or(text)
}

fn split_assignment(line: usize, rest: &str) -> Result<(String, Expr), Problem> {
    let (name, value) = rest.split_once('=').ok_or_else(|| {
        Problem::new(
            line,
            "this line names a variable but never says what to put in it",
            "write `= ` and a value, as in declare 'health' = integer '20'",
        )
    })?;
    Ok((quoted(line, name.trim())?, parse_expr(line, value)?))
}

/// A bare quoted word is a name.
fn quoted(line: usize, text: &str) -> Result<String, Problem> {
    let trimmed = text.trim();
    trimmed
        .strip_prefix('\'')
        .and_then(|t| t.strip_suffix('\''))
        .map(|t| t.to_string())
        .ok_or_else(|| {
            Problem::new(
                line,
                format!("{trimmed} is not a name"),
                "a name goes in single quotes, as in 'health'",
            )
        })
}

/// Values, lowest-binding operator first. Comparison binds loosest, then `+ -`, then
/// `* /`, which is the order everyone already expects.
fn parse_expr(line: usize, text: &str) -> Result<Expr, Problem> {
    let text = text.trim();
    if let Some((a, _, b)) = split_level(text, &[("<", ArithOp::Add)]) {
        return Ok(Expr::LessThan(
            Box::new(parse_expr(line, a)?),
            Box::new(parse_expr(line, b)?),
        ));
    }
    // Both operators of a level are searched together. Splitting on every `*` first and
    // only then on `/` would read `a * b / c` as `a * (b / c)` — which for whole numbers
    // is a different answer, not just a different shape: 6 * 3 / 2 would be 6, not 9.
    for level in [
        [("+", ArithOp::Add), ("-", ArithOp::Subtract)].as_slice(),
        [
            ("*", ArithOp::Multiply),
            ("×", ArithOp::Multiply),
            ("/", ArithOp::Divide),
            ("÷", ArithOp::Divide),
        ]
        .as_slice(),
    ] {
        if let Some((a, op, b)) = split_level(text, level) {
            return Ok(Expr::Arith(
                op,
                Box::new(parse_expr(line, a)?),
                Box::new(parse_expr(line, b)?),
            ));
        }
    }
    parse_atom(line, text)
}

/// Split on the *last* operator of this level, so `a - b - c` groups as `(a - b) - c`.
fn split_level<'a>(
    text: &'a str,
    level: &[(&str, ArithOp)],
) -> Option<(&'a str, ArithOp, &'a str)> {
    let mut best: Option<(usize, usize, ArithOp)> = None;
    for (i, _) in text.char_indices() {
        // `i > 0` leaves a leading minus alone, and a quoted `-` belongs to the literal
        // it is part of, as in integer '-4'.
        if i == 0 || inside_quotes(text, i) {
            continue;
        }
        for (symbol, op) in level {
            if text[i..].starts_with(symbol) {
                best = Some((i, symbol.len(), *op));
            }
        }
    }
    let (at, width, op) = best?;
    Some((&text[..at], op, &text[at + width..]))
}

fn inside_quotes(text: &str, at: usize) -> bool {
    text[..at].matches('\'').count() % 2 == 1
}

/// A literal announces its type; anything else quoted is a name.
fn parse_atom(line: usize, text: &str) -> Result<Expr, Problem> {
    let text = text.trim();
    for (word, ty) in [
        ("integer", DataType::Int),
        ("float", DataType::Float),
        ("string", DataType::Str),
        ("boolean", DataType::Bool),
    ] {
        if let Some(rest) = text.strip_prefix(word) {
            let raw = quoted(line, rest)?;
            return Ok(Expr::Lit(parse_value(line, ty, &raw)?));
        }
    }
    if text.starts_with('\'') {
        return Ok(Expr::Var(quoted(line, text)?));
    }
    Err(Problem::new(
        line,
        format!("{text} is neither a name nor a value"),
        "a name is quoted, as in 'health'; a value says its type first, as in integer '20'",
    ))
}

fn parse_value(line: usize, ty: DataType, raw: &str) -> Result<Value, Problem> {
    let bad = |what: &str| {
        Problem::new(
            line,
            format!("'{raw}' is not {what}"),
            match ty {
                DataType::Int => "a whole number looks like integer '20'",
                DataType::Float => "a decimal looks like float '1.5'",
                DataType::Bool => "a boolean is boolean 'true' or boolean 'false'",
                DataType::Str => "any text will do",
            },
        )
    };
    Ok(match ty {
        DataType::Int => match raw.parse::<i64>() {
            Ok(v) => Value::Int(v),
            // Too big is a different mistake from not-a-number, and saying "'99999999999999999999'
            // is not a whole number" to someone who has just typed a whole number teaches
            // them to distrust the message rather than the number.
            Err(e)
                if matches!(
                    e.kind(),
                    std::num::IntErrorKind::PosOverflow | std::num::IntErrorKind::NegOverflow
                ) =>
            {
                return Err(Problem::new(
                    line,
                    format!("{raw} is too big to be an integer"),
                    "an integer goes from -9223372036854775808 to 9223372036854775807 — use a float for numbers larger than that",
                ))
            }
            Err(_) => return Err(bad("a whole number")),
        },
        DataType::Float => Value::Float(raw.parse().map_err(|_| bad("a decimal"))?),
        DataType::Bool => match raw {
            "true" => Value::Bool(true),
            "false" => Value::Bool(false),
            _ => return Err(bad("true or false")),
        },
        DataType::Str => Value::Str(raw.to_string()),
    })
}

/// "an integer", "a float" — a message that says "not float" reads like a machine.
fn a_an(label: &str) -> String {
    let article = if label.starts_with(['a', 'e', 'i', 'o', 'u']) {
        "an"
    } else {
        "a"
    };
    format!("{article} {label}")
}

// ── building ────────────────────────────────────────────────────────────────

/// Where to start laying out, so generated nodes do not land on existing ones.
fn free_column(graph: &Graph) -> f32 {
    graph
        .nodes()
        .map(|n| n.pos.0)
        .fold(0.0_f32, f32::max)
        + 320.0
}

struct Builder<'a> {
    graph: &'a mut Graph,
    made: Vec<NodeId>,
    next: f32,
    row: f32,
    problems: Vec<Problem>,
}

const ROW: f32 = 150.0;
const COL: f32 = 300.0;

impl<'a> Builder<'a> {
    fn add(&mut self, kind: NodeKind, column: f32) -> NodeId {
        let id = self.graph.add_node(kind, (self.next + column, self.row));
        self.made.push(id);
        id
    }

    fn wire(&mut self, from: NodeId, from_pin: usize, to: NodeId, to_pin: usize) {
        let _ = self.graph.connect(
            PinRef { node: from, side: Side::Out, index: from_pin },
            PinRef { node: to, side: Side::In, index: to_pin },
        );
    }

    /// The Event start to hang this fragment from, if the canvas has none of its own.
    fn entry_point(&mut self) -> Option<NodeId> {
        if self.graph.nodes().any(|n| n.kind == NodeKind::EventStart) {
            return None;
        }
        Some(self.add(NodeKind::EventStart, 0.0))
    }

    /// Build a run of statements, chaining their execution pins. Returns the first.
    fn statements(&mut self, body: &[Stmt]) -> Option<NodeId> {
        let mut first = None;
        let mut previous: Option<(NodeId, Option<usize>)> = None;

        for stmt in body {
            self.row += ROW;
            let (head, tail_pin) = self.statement(stmt)?;
            if first.is_none() {
                first = Some(head);
            }
            match previous {
                Some((prev, Some(pin))) => self.wire(prev, pin, head, 0),
                // A Branch has a true pin and a false pin and nothing else, so there
                // is no pin left to carry on from. Saying so beats wiring this step
                // into the true arm, which is what a naive chaining would do — and it
                // would silently delete the arm that was already there.
                Some((_, None)) => {
                    self.problems.push(Problem::new(
                        stmt.line(),
                        "nothing can follow an `if` yet, because its two paths each end where they end",
                        "put these steps inside both arms of the if, or make the if the last thing",
                    ));
                    return None;
                }
                None => {}
            }
            previous = Some((head, tail_pin));
        }
        first
    }

    /// Build one statement. Returns the node execution enters by, and which of its
    /// output pins the next statement follows — `None` when nothing may follow.
    fn statement(&mut self, stmt: &Stmt) -> Option<(NodeId, Option<usize>)> {
        match stmt {
            Stmt::Declare { name, value, line } => {
                let ty = self.type_of(value, *line)?;
                self.graph.declare_var(name.clone(), ty);
                if let Some(decl) = self.graph.vars.get_mut(name) {
                    decl.initial = ty.default_value();
                }
                let set = self.add(NodeKind::SetVar { name: name.clone(), ty }, COL);
                let source = self.value(value, *line)?;
                self.wire(source.0, source.1, set, 1);
                Some((set, Some(0)))
            }
            Stmt::Set { name, value, line } => {
                let Some(decl) = self.graph.vars.get(name) else {
                    self.problems.push(Problem::new(
                        *line,
                        format!("there is no variable called '{name}' to set"),
                        "declare it first, as in declare 'health' = integer '20'",
                    ));
                    return None;
                };
                let ty = decl.ty;
                let set = self.add(NodeKind::SetVar { name: name.clone(), ty }, COL);
                let source = self.value(value, *line)?;
                self.wire(source.0, source.1, set, 1);
                Some((set, Some(0)))
            }
            Stmt::Print { value, line } => {
                let ty = self.type_of(value, *line)?;
                let show = self.add(NodeKind::Print { ty }, COL);
                let source = self.value(value, *line)?;
                self.wire(source.0, source.1, show, 1);
                Some((show, Some(0)))
            }
            Stmt::If { condition, then, otherwise, line } => {
                let branch = self.add(NodeKind::Branch, COL);
                let cond = self.value(condition, *line)?;
                self.wire(cond.0, cond.1, branch, 1);

                if let Some(head) = self.statements(then) {
                    self.wire(branch, 0, head, 0);
                }
                if let Some(head) = self.statements(otherwise) {
                    self.wire(branch, 1, head, 0);
                }
                // Both arms are their own chains, so nothing follows the branch itself.
                Some((branch, None))
            }
            Stmt::Repeat { times, body, line } => {
                let ty = self.type_of(times, *line)?;
                if ty != DataType::Int {
                    self.problems.push(Problem::new(
                        *line,
                        format!("a repeat counts a whole number of times, not {}", a_an(ty.label())),
                        "use a whole number, as in repeat integer '10' {",
                    ));
                    return None;
                }
                let node = self.add(NodeKind::Repeat, COL);
                let count = self.value(times, *line)?;
                self.wire(count.0, count.1, node, 1);

                if let Some(head) = self.statements(body) {
                    self.wire(node, 0, head, 0);
                }
                // Unlike a Branch, a Repeat does have somewhere to carry on to.
                Some((node, Some(1)))
            }
        }
    }

    /// Build the nodes for a value. Returns the node and output pin carrying it.
    fn value(&mut self, expr: &Expr, line: usize) -> Option<(NodeId, usize)> {
        match expr {
            Expr::Lit(v) => {
                let kind = match v {
                    Value::Int(i) => NodeKind::LitInt(*i),
                    Value::Float(f) => NodeKind::LitFloat(*f),
                    Value::Bool(b) => NodeKind::LitBool(*b),
                    Value::Str(s) => NodeKind::LitStr(s.clone()),
                };
                Some((self.add(kind, 0.0), 0))
            }
            Expr::Var(name) => {
                let Some(decl) = self.graph.vars.get(name) else {
                    self.problems.push(Problem::new(
                        line,
                        format!("there is no variable called '{name}'"),
                        "declare it first, or check the spelling",
                    ));
                    return None;
                };
                let ty = decl.ty;
                Some((self.add(NodeKind::GetVar { name: name.clone(), ty }, 0.0), 0))
            }
            Expr::LessThan(a, b) => {
                let node = self.add(NodeKind::LessThan, COL / 2.0);
                let (a, b) = (self.value(a, line)?, self.value(b, line)?);
                self.wire(a.0, a.1, node, 0);
                self.wire(b.0, b.1, node, 1);
                Some((node, 0))
            }
            Expr::Arith(op, a, b) => {
                let ty = self.type_of(expr, line)?;
                let node = self.add(NodeKind::Arith { op: *op, ty }, COL / 2.0);
                let (a, b) = (self.value(a, line)?, self.value(b, line)?);
                self.wire(a.0, a.1, node, 0);
                self.wire(b.0, b.1, node, 1);
                Some((node, 0))
            }
        }
    }

    /// What type a value produces, which decides the pins on the nodes built for it.
    fn type_of(&mut self, expr: &Expr, line: usize) -> Option<DataType> {
        match expr {
            Expr::Lit(v) => Some(v.data_type()),
            Expr::Var(name) => match self.graph.vars.get(name) {
                Some(decl) => Some(decl.ty),
                None => {
                    self.problems.push(Problem::new(
                        line,
                        format!("there is no variable called '{name}'"),
                        "declare it first, or check the spelling",
                    ));
                    None
                }
            },
            Expr::LessThan(_, _) => Some(DataType::Bool),
            // Arithmetic keeps the type of what goes into it, and both sides must agree
            // for the wires to connect — the pins are typed, so a float cannot feed an
            // integer sum whatever the text says.
            Expr::Arith(_, a, b) => {
                let left = self.type_of(a, line)?;
                let right = self.type_of(b, line)?;
                if left != right {
                    self.problems.push(Problem::new(
                        line,
                        format!(
                            "this mixes {} and {}, which cannot be wired together",
                            left.label(),
                            right.label()
                        ),
                        "make both sides the same type",
                    ));
                    return None;
                }
                Some(left)
            }
        }
    }
}
