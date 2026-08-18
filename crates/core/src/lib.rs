//! Cat Paws core: the graph, the compiler and the virtual machine.
//!
//! Nothing here depends on egui, so the whole language can be tested in
//! milliseconds without opening a window.

pub mod compile;
pub mod graph;
pub mod types;
pub mod vm;
pub mod wasm;
pub mod written;

pub use compile::{compile, Area, Code, Diagnostic, Expr, Instr, Program};
pub use graph::{Category, Graph, Link, Node, NodeId, NodeKind, Pin, PinKind, PinRef, Side};
pub use types::{DataType, Value};
pub use vm::{run, RunResult};
pub use wasm::{emit, text};
pub use written::{generate, Problem};

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn wire(g: &mut Graph, from: NodeId, fi: usize, to: NodeId, ti: usize) {
        g.connect(
            PinRef {
                node: from,
                side: Side::Out,
                index: fi,
            },
            PinRef {
                node: to,
                side: Side::In,
                index: ti,
            },
        )
        .expect("connection should be legal");
    }

    /// Builds the graph from the reference image: start -> branch on
    /// (health < 50), printing a different line on each side.
    pub(crate) fn reference_graph(health: i64) -> Graph {
        let mut g = Graph::new();
        g.declare_var("Health".to_string(), DataType::Int);
        g.vars.get_mut("Health").unwrap().initial = Value::Int(health);

        let start = g.add_node(NodeKind::EventStart, (0.0, 0.0));
        let branch = g.add_node(NodeKind::Branch, (200.0, 0.0));
        let get = g.add_node(
            NodeKind::GetVar {
                name: "Health".to_string(),
                ty: DataType::Int,
            },
            (0.0, 200.0),
        );
        let fifty = g.add_node(NodeKind::LitInt(50), (0.0, 300.0));
        let less = g.add_node(NodeKind::LessThan, (200.0, 200.0));
        let low = g.add_node(NodeKind::Print { ty: DataType::Str }, (420.0, -40.0));
        let fine = g.add_node(NodeKind::Print { ty: DataType::Str }, (420.0, 80.0));
        let low_text = g.add_node(NodeKind::LitStr("low health".to_string()), (420.0, 300.0));
        let fine_text = g.add_node(NodeKind::LitStr("fine".to_string()), (420.0, 380.0));

        wire(&mut g, start, 0, branch, 0);
        wire(&mut g, get, 0, less, 0);
        wire(&mut g, fifty, 0, less, 1);
        wire(&mut g, less, 0, branch, 1);
        wire(&mut g, branch, 0, low, 0);
        wire(&mut g, branch, 1, fine, 0);
        wire(&mut g, low_text, 0, low, 1);
        wire(&mut g, fine_text, 0, fine, 1);
        g
    }

    #[test]
    fn reference_graph_takes_the_true_branch() {
        let program = compile(&reference_graph(20)).expect("should compile");
        let result = run(&program);
        assert_eq!(result.output, vec!["low health".to_string()]);
        assert!(result.error.is_none());
    }

    #[test]
    fn reference_graph_takes_the_false_branch() {
        let program = compile(&reference_graph(90)).expect("should compile");
        let result = run(&program);
        assert_eq!(result.output, vec!["fine".to_string()]);
    }

    #[test]
    fn mismatched_types_cannot_be_wired() {
        let mut g = Graph::new();
        let text = g.add_node(NodeKind::LitStr("hi".to_string()), (0.0, 0.0));
        let branch = g.add_node(NodeKind::Branch, (200.0, 0.0));
        // A string into a boolean condition must be refused.
        let err = g.connect(
            PinRef {
                node: text,
                side: Side::Out,
                index: 0,
            },
            PinRef {
                node: branch,
                side: Side::In,
                index: 1,
            },
        );
        assert!(err.is_err());
    }

    #[test]
    fn exec_and_data_pins_cannot_be_joined() {
        let mut g = Graph::new();
        let start = g.add_node(NodeKind::EventStart, (0.0, 0.0));
        let less = g.add_node(NodeKind::LessThan, (200.0, 0.0));
        let err = g.connect(
            PinRef {
                node: start,
                side: Side::Out,
                index: 0,
            },
            PinRef {
                node: less,
                side: Side::In,
                index: 0,
            },
        );
        assert!(err.is_err());
    }

    #[test]
    fn missing_start_node_is_reported() {
        let g = Graph::new();
        let diags = compile(&g).expect_err("empty graph should not compile");
        assert!(diags[0].message.contains("Event start"));
    }

    #[test]
    fn unconnected_input_is_reported() {
        let mut g = Graph::new();
        let start = g.add_node(NodeKind::EventStart, (0.0, 0.0));
        let print = g.add_node(NodeKind::Print { ty: DataType::Str }, (200.0, 0.0));
        wire(&mut g, start, 0, print, 0);
        let diags = compile(&g).expect_err("print with no text should not compile");
        assert!(diags[0].message.contains("nothing wired into it"));
    }

    #[test]
    fn a_data_input_only_accepts_one_wire() {
        let mut g = Graph::new();
        let a = g.add_node(NodeKind::LitStr("a".to_string()), (0.0, 0.0));
        let b = g.add_node(NodeKind::LitStr("b".to_string()), (0.0, 100.0));
        let print = g.add_node(NodeKind::Print { ty: DataType::Str }, (200.0, 0.0));
        wire(&mut g, a, 0, print, 1);
        wire(&mut g, b, 0, print, 1);
        assert_eq!(g.links().len(), 1, "second wire should replace the first");
    }
}

/// Running compiled WebAssembly, in a real engine.
///
/// These are the tests that matter for the compiler: not "did we produce bytes" but
/// "does an engine accept them and does the program then do the right thing". Wasmtime
/// validates a module exactly as a browser does, so a module that runs here runs there.
#[cfg(test)]
pub(crate) mod wasm_tests {
    use super::tests::{reference_graph, wire};
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Instantiate a module, run `main`, and collect whatever it printed.
    pub(crate) fn run_wasm(bytes: &[u8]) -> Vec<String> {
        let engine = wasmtime::Engine::default();
        let module = wasmtime::Module::new(&engine, bytes).expect("the engine should accept it");
        let printed: Arc<Mutex<Vec<String>>> = Arc::default();

        let mut store = wasmtime::Store::new(&engine, Arc::clone(&printed));
        let mut linker = wasmtime::Linker::new(&engine);
        linker
            .func_wrap(
                "env",
                "print",
                |mut caller: wasmtime::Caller<'_, Arc<Mutex<Vec<String>>>>, ptr: i32| {
                    // Exactly what a browser host does: read the length prefix, then
                    // that many UTF-8 bytes out of the module's exported memory.
                    let memory = caller
                        .get_export("memory")
                        .and_then(|e| e.into_memory())
                        .expect("the module should export its memory");
                    let data = memory.data(&caller);
                    let at = ptr as usize;
                    let len = u32::from_le_bytes(data[at..at + 4].try_into().unwrap()) as usize;
                    let text = String::from_utf8_lossy(&data[at + 4..at + 4 + len]).into_owned();
                    caller.data().lock().unwrap().push(text);
                },
            )
            .expect("print should link");
        linker
            .func_wrap(
                "env",
                "print_int",
                |caller: wasmtime::Caller<'_, Arc<Mutex<Vec<String>>>>, v: i64| {
                    caller.data().lock().unwrap().push(v.to_string());
                },
            )
            .expect("print_int should link");
        linker
            .func_wrap(
                "env",
                "print_float",
                |caller: wasmtime::Caller<'_, Arc<Mutex<Vec<String>>>>, v: f64| {
                    caller.data().lock().unwrap().push(v.to_string());
                },
            )
            .expect("print_float should link");
        linker
            .func_wrap(
                "env",
                "print_bool",
                |caller: wasmtime::Caller<'_, Arc<Mutex<Vec<String>>>>, v: i32| {
                    caller
                        .data()
                        .lock()
                        .unwrap()
                        .push(if v != 0 { "true" } else { "false" }.to_string());
                },
            )
            .expect("print_bool should link");

        let instance = linker
            .instantiate(&mut store, &module)
            .expect("the module should instantiate");
        instance
            .get_typed_func::<(), ()>(&mut store, "main")
            .expect("main should be exported")
            .call(&mut store, ())
            .expect("main should run");

        let out = printed.lock().unwrap().clone();
        out
    }

    #[test]
    fn a_branch_becomes_structured_control_flow() {
        // The same graph both ways. WebAssembly has no jump-to-address, so this proves
        // the branch really did become a nested `if`/`else`.
        assert_eq!(run_wasm(&emit(&reference_graph(20)).unwrap()), vec!["low health"]);
        assert_eq!(run_wasm(&emit(&reference_graph(90)).unwrap()), vec!["fine"]);
    }

    #[test]
    fn the_interpreter_and_the_compiler_agree() {
        // The old bytecode VM is kept as a second opinion. Two implementations written
        // separately will not usually be wrong in the same way, so a disagreement is a
        // real bug in one of them.
        for health in [-5, 0, 1, 49, 50, 51, 90, 1000] {
            let graph = reference_graph(health);
            let interpreted = run(&compile(&graph).expect("bytecode should compile")).output;
            let compiled = run_wasm(&emit(&graph).expect("wasm should compile"));
            assert_eq!(
                interpreted, compiled,
                "health {health}: interpreter said {interpreted:?}, wasm said {compiled:?}"
            );
        }
    }

    #[test]
    fn variables_round_trip_through_locals() {
        let mut g = Graph::new();
        g.declare_var("Message".to_string(), DataType::Str);
        let start = g.add_node(NodeKind::EventStart, (0.0, 0.0));
        let set = g.add_node(
            NodeKind::SetVar {
                name: "Message".to_string(),
                ty: DataType::Str,
            },
            (200.0, 0.0),
        );
        let text = g.add_node(NodeKind::LitStr("hello paws".to_string()), (0.0, 200.0));
        let show = g.add_node(NodeKind::Print { ty: DataType::Str }, (400.0, 0.0));
        let get = g.add_node(
            NodeKind::GetVar {
                name: "Message".to_string(),
                ty: DataType::Str,
            },
            (200.0, 200.0),
        );
        wire(&mut g, start, 0, set, 0);
        wire(&mut g, text, 0, set, 1);
        wire(&mut g, set, 0, show, 0);
        wire(&mut g, get, 0, show, 1);

        assert_eq!(run_wasm(&emit(&g).unwrap()), vec!["hello paws"]);
    }

    /// Writes the reference program out so it can be inspected or run elsewhere.
    #[test]
    fn dump_a_module_for_inspection() {
        let bytes = emit(&reference_graph(20)).unwrap();
        let path = std::env::temp_dir().join("cat-paws-reference.wasm");
        std::fs::write(&path, &bytes).unwrap();
        eprintln!("wrote {} ({} bytes)", path.display(), bytes.len());
    }

    #[test]
    fn a_graph_with_no_start_is_refused() {
        let g = Graph::new();
        let diags = emit(&g).expect_err("should not compile");
        assert!(diags[0].message.contains("Event start"), "{diags:?}");
    }
}

/// Every diagnostic should be able to stand in front of a beginner.
#[cfg(test)]
mod diagnostic_tests {
    use super::tests::{reference_graph, wire};
    use super::*;

    /// Every code the compiler can produce. Adding one here is what makes the tests
    /// below cover it.
    const ALL_CODES: [Code; 9] = [
        compile::NO_START,
        compile::MANY_STARTS,
        compile::EXEC_LOOP,
        compile::START_IN_CHAIN,
        compile::VALUE_AS_STEP,
        compile::EMPTY_PIN,
        compile::DATA_LOOP,
        compile::NOT_A_VALUE,
        compile::NO_SUCH_VAR,
    ];

    /// Compile something broken and hand back what the user would read.
    fn problems(build: impl Fn(&mut Graph)) -> Vec<Diagnostic> {
        let mut g = reference_graph(20);
        build(&mut g);
        match compile(&g) {
            Ok(_) => Vec::new(),
            Err(diags) => diags,
        }
    }

    #[test]
    fn every_problem_says_what_to_do_about_it() {
        // A message that only names the fault leaves a beginner stuck. Whatever we
        // report, there is a second sentence telling them what to change.
        let cases: Vec<Vec<Diagnostic>> = vec![
            problems(|g| {
                // Nothing wired into a Print's text pin.
                let extra = g.add_node(NodeKind::Print { ty: DataType::Str }, (900.0, 0.0));
                let start = g
                    .nodes()
                    .find(|n| n.kind == NodeKind::EventStart)
                    .map(|n| n.id)
                    .unwrap();
                g.disconnect_pin(PinRef {
                    node: start,
                    side: Side::Out,
                    index: 0,
                });
                wire(g, start, 0, extra, 0);
            }),
            problems(|g| {
                g.add_node(NodeKind::EventStart, (900.0, 900.0));
            }),
            match compile(&Graph::new()) {
                Ok(_) => Vec::new(),
                Err(d) => d,
            },
        ];

        for diags in cases {
            assert!(!diags.is_empty(), "this case should have failed to compile");
            for d in diags {
                assert!(!d.fix.trim().is_empty(), "no fix offered for: {}", d.message);
                assert!(
                    !d.message.trim().is_empty(),
                    "a diagnostic with no message is no use"
                );
                // Jargon a beginner has no way to look up.
                for word in ["expr", "pin index", "NodeId", "unwrap", "panic"] {
                    assert!(
                        !d.message.contains(word) && !d.fix.contains(word),
                        "internal wording leaked into a message: {} / {}",
                        d.message,
                        d.fix
                    );
                }
            }
        }
    }

    #[test]
    fn a_code_reads_the_way_it_is_meant_to() {
        assert_eq!(compile::EMPTY_PIN.render(), "CP-WIRE-01");
        assert_eq!(compile::NO_START.render(), "CP-FLOW-01");
        assert_eq!(compile::NO_SUCH_VAR.render(), "CP-NAME-01");
    }

    #[test]
    fn every_code_is_used_by_exactly_one_kind_of_problem() {
        // Two problems sharing a code would make the code useless as a handle: someone
        // searching for it would find an explanation of the wrong thing.
        let all = ALL_CODES;
        let _ = [
            compile::NO_START,
            compile::MANY_STARTS,
            compile::EXEC_LOOP,
            compile::START_IN_CHAIN,
            compile::VALUE_AS_STEP,
            compile::EMPTY_PIN,
            compile::DATA_LOOP,
            compile::NOT_A_VALUE,
            compile::NO_SUCH_VAR,
        ];
        let mut seen: Vec<String> = all.iter().map(|c| c.render()).collect();
        seen.sort();
        let count = seen.len();
        seen.dedup();
        assert_eq!(seen.len(), count, "two problems share a code");
    }

    #[test]
    fn every_code_states_its_rule() {
        // A code with no rule renders without the line, silently. Listing them here
        // means adding a code without explaining it fails loudly instead.
        for code in ALL_CODES {
            let rule = code.rule();
            assert!(
                !rule.is_empty(),
                "{} has no rule written for it",
                code.render()
            );
            assert!(
                rule.ends_with('.') && rule.len() > 25,
                "{} reads oddly: {rule:?}",
                code.render()
            );
            // The rule describes the language, so it must not talk about one instance.
            // Matched as whole words: "There is no default" is fine, "here" is not.
            let words: Vec<String> = rule
                .split(|c: char| !c.is_alphanumeric())
                .map(|w| w.to_lowercase())
                .collect();
            for banned in ["here", "this", "expr", "nodeid"] {
                assert!(
                    !words.iter().any(|w| w == banned),
                    "{} states an instance, not a rule: {rule:?}",
                    code.render()
                );
            }
        }
    }

    #[test]
    fn both_backends_report_the_same_problem() {
        // The bytecode compiler and the WebAssembly compiler each walk the graph, so a
        // broken program has to be refused by both — not compiled by one of them.
        let g = Graph::new();
        let bytecode = compile(&g).err().expect("no start node");
        let wasm = wasm::emit(&g).err().expect("no start node");
        assert_eq!(bytecode.len(), wasm.len());
        assert_eq!(bytecode[0].message, wasm[0].message);
        assert_eq!(bytecode[0].fix, wasm[0].fix);
        assert_eq!(bytecode[0].code, wasm[0].code);
    }
}



/// Arithmetic, checked in a real engine and against the interpreter.
#[cfg(test)]
pub(crate) mod arith_tests {
    use super::tests::wire;
    use super::*;
    use graph::ArithOp;

    /// start -> print( a <op> b ), as a graph.
    fn maths(op: ArithOp, ty: DataType, a: Value, b: Value) -> Graph {
        let mut g = Graph::new();
        g.declare_var("Answer".to_string(), ty);
        let start = g.add_node(NodeKind::EventStart, (0.0, 0.0));
        let set = g.add_node(
            NodeKind::SetVar {
                name: "Answer".to_string(),
                ty,
            },
            (200.0, 0.0),
        );
        let node = g.add_node(NodeKind::Arith { op, ty }, (0.0, 200.0));
        let lit_a = g.add_node(lit(a), (-200.0, 150.0));
        let lit_b = g.add_node(lit(b), (-200.0, 250.0));
        wire(&mut g, start, 0, set, 0);
        wire(&mut g, lit_a, 0, node, 0);
        wire(&mut g, lit_b, 0, node, 1);
        wire(&mut g, node, 0, set, 1);
        g
    }

    fn lit(v: Value) -> NodeKind {
        match v {
            Value::Int(i) => NodeKind::LitInt(i),
            Value::Float(f) => NodeKind::LitFloat(f),
            Value::Bool(b) => NodeKind::LitBool(b),
            Value::Str(s) => NodeKind::LitStr(s),
        }
    }

    /// start -> print( a <op> b ), so the answer can actually be observed.
    pub(crate) fn shown(op: ArithOp, ty: DataType, a: Value, b: Value) -> Graph {
        let mut g = Graph::new();
        let start = g.add_node(NodeKind::EventStart, (0.0, 0.0));
        let show = g.add_node(NodeKind::Print { ty }, (300.0, 0.0));
        let node = g.add_node(NodeKind::Arith { op, ty }, (0.0, 200.0));
        let lit_a = g.add_node(lit(a), (-200.0, 150.0));
        let lit_b = g.add_node(lit(b), (-200.0, 250.0));
        wire(&mut g, start, 0, show, 0);
        wire(&mut g, lit_a, 0, node, 0);
        wire(&mut g, lit_b, 0, node, 1);
        wire(&mut g, node, 0, show, 1);
        g
    }

    #[test]
    fn whole_number_arithmetic_agrees_with_the_interpreter() {
        for (op, a, b) in [
            (ArithOp::Add, 7, 5),
            (ArithOp::Subtract, 7, 5),
            (ArithOp::Multiply, 7, 5),
            (ArithOp::Divide, 7, 5),
            (ArithOp::Divide, -7, 5),
            (ArithOp::Subtract, 5, 7),
            (ArithOp::Multiply, -3, -4),
        ] {
            let g = maths(op, DataType::Int, Value::Int(a), Value::Int(b));
            let program = compile(&g).expect("should compile");
            let interpreted = vm::run(&program);
            assert!(interpreted.error.is_none(), "{op:?} {a} {b}: {:?}", interpreted.error);
            // The WebAssembly module must at least build and validate for every case.
            wasm::emit(&g).expect("should emit");
        }
    }

    #[test]
    fn dividing_a_whole_number_by_zero_is_refused_while_compiling() {
        // This used to compile and then trap part-way through running, after anything
        // before it had already printed. Both sides are known here, so the answer can be
        // asked for — and refused — before the program ever starts.
        let g = maths(ArithOp::Divide, DataType::Int, Value::Int(1), Value::Int(0));
        let diags = compile(&g).expect_err("dividing by zero should be refused");
        assert_eq!(diags[0].code, compile::DIVIDE_BY_ZERO);
        assert!(diags[0].message.contains("zero"), "unhelpful: {}", diags[0].message);
        wasm::emit(&g).expect_err("the compiled path should refuse it too");
    }

    #[test]
    fn dividing_by_a_zero_that_only_appears_at_run_time_still_stops_the_program() {
        // The compile-time check only reaches sums whose operands are already known. A
        // divisor that arrives through a variable is not, so `i64.div_s` trapping — and
        // the interpreter refusing to match it — is still what protects this case.
        let mut g = Graph::new();
        written::generate(
            &mut g,
            "declare 'd' = integer '0'\nprint integer '1' / 'd'",
        )
        .expect("should read");
        let program = compile(&g).expect("should compile — the zero is not known yet");
        let message = vm::run(&program).error.expect("dividing by zero should stop it");
        assert!(message.contains("zero"), "unhelpful message: {message}");
    }

    #[test]
    fn float_division_by_zero_is_allowed() {
        // Floats have infinity, so this is a value rather than a fault — and f64.div
        // does not trap. The two paths agree because both follow IEEE.
        let g = maths(ArithOp::Divide, DataType::Float, Value::Float(1.0), Value::Float(0.0));
        let program = compile(&g).expect("should compile");
        assert!(vm::run(&program).error.is_none());
        wasm::emit(&g).expect("should emit");
    }

    #[test]
    fn arithmetic_pins_only_accept_their_own_type() {
        // The point of typing the node rather than inferring it: a float cannot be
        // dragged into an integer sum.
        let mut g = Graph::new();
        let add = g.add_node(
            NodeKind::Arith {
                op: ArithOp::Add,
                ty: DataType::Int,
            },
            (0.0, 0.0),
        );
        let f = g.add_node(NodeKind::LitFloat(1.5), (0.0, 200.0));
        let joined = g.connect(
            PinRef { node: f, side: Side::Out, index: 0 },
            PinRef { node: add, side: Side::In, index: 0 },
        );
        assert!(joined.is_err(), "a float should not fit an integer pin");
    }
}

/// Arithmetic, compared across both backends now that a number can be printed.
#[cfg(test)]
mod arith_agreement {
    use super::arith_tests::shown;
    use super::wasm_tests::run_wasm;
    use super::*;
    use graph::ArithOp;

    #[test]
    fn both_backends_compute_the_same_answers() {
        // Until Print could take a number, arithmetic could be computed but not seen,
        // so the two implementations could not be compared on it at all.
        for (op, a, b) in [
            (ArithOp::Add, 7, 5),
            (ArithOp::Subtract, 7, 5),
            (ArithOp::Subtract, 5, 7),
            (ArithOp::Multiply, 7, 5),
            (ArithOp::Multiply, -3, -4),
            (ArithOp::Divide, 7, 5),
            (ArithOp::Divide, -7, 5),
            (ArithOp::Divide, 7, -5),
            (ArithOp::Add, i64::MAX, 0),
            (ArithOp::Multiply, 1_000_000, 1_000_000),
        ] {
            let g = shown(op, DataType::Int, Value::Int(a), Value::Int(b));
            let interpreted = vm::run(&compile(&g).expect("bytecode")).output;
            let compiled = run_wasm(&wasm::emit(&g).expect("wasm"));
            assert_eq!(
                interpreted, compiled,
                "{a} {} {b}: interpreter said {interpreted:?}, wasm said {compiled:?}",
                op.symbol()
            );
        }
    }

    #[test]
    fn a_printed_number_reads_the_way_a_person_writes_it() {
        let g = shown(ArithOp::Add, DataType::Int, Value::Int(2), Value::Int(3));
        assert_eq!(run_wasm(&wasm::emit(&g).unwrap()), vec!["5"]);
    }
}


/// The written form, judged by whether what it builds actually runs.
#[cfg(test)]
mod written_tests {
    use super::wasm_tests::run_wasm;
    use super::*;

    /// Type a program, then run whatever appeared on the canvas.
    fn typed(text: &str) -> Vec<String> {
        let mut g = Graph::new();
        written::generate(&mut g, text).unwrap_or_else(|p| panic!("should read: {p:#?}"));
        run_wasm(&wasm::emit(&g).unwrap_or_else(|d| panic!("should compile: {d:#?}")))
    }

    #[test]
    fn the_reference_program_can_be_typed() {
        // The health check, in six lines instead of ten dragged nodes.
        let out = typed(
            "declare 'health' = integer '20'\n\
             if 'health' < integer '50' {\n\
                 print string 'low health'\n\
             } else {\n\
                 print string 'fine'\n\
             }",
        );
        assert_eq!(out, vec!["low health"]);
    }

    #[test]
    fn the_other_branch_runs_too() {
        let out = typed(
            "declare 'health' = integer '90'\n\
             if 'health' < integer '50' {\n\
                 print string 'low health'\n\
             } else {\n\
                 print string 'fine'\n\
             }",
        );
        assert_eq!(out, vec!["fine"]);
    }

    #[test]
    fn arithmetic_and_printing_a_number() {
        assert_eq!(typed("print integer '2' + integer '3'"), vec!["5"]);
        assert_eq!(typed("print integer '10' - integer '4' * integer '2'"), vec!["2"]);
        assert_eq!(typed("print float '7.5' / float '2.5'"), vec!["3"]);
    }

    #[test]
    fn a_variable_can_be_declared_then_set_then_read() {
        let out = typed(
            "declare 'n' = integer '1'\n\
             set 'n' = 'n' + integer '41'\n\
             print 'n'",
        );
        assert_eq!(out, vec!["42"]);
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let out = typed(
            "# the answer\n\
             \n\
             print integer '42'   # shown here\n",
        );
        assert_eq!(out, vec!["42"]);
    }

    #[test]
    fn one_line_makes_as_many_nodes_as_it_needs() {
        let mut g = Graph::new();
        let made = written::generate(&mut g, "declare 'x' = integer '20'").unwrap();
        // A variable, a literal and a Set — plus the Event start, since the canvas had
        // none of its own.
        assert!(g.vars.contains_key("x"), "the variable should exist");
        assert_eq!(made.len(), 3, "expected start, literal and set: {made:?}");
    }

    #[test]
    fn what_is_already_on_the_canvas_is_left_alone() {
        let mut g = Graph::new();
        let kept = g.add_node(NodeKind::LitInt(7), (0.0, 0.0));
        written::generate(&mut g, "print integer '1'").unwrap();
        assert!(g.node(kept).is_some(), "the existing node should still be there");
        // The canvas already had no Event start, so one was made.
        assert_eq!(
            g.nodes().filter(|n| n.kind == NodeKind::EventStart).count(),
            1
        );
    }

    #[test]
    fn a_second_start_is_not_added_to_a_canvas_that_has_one() {
        let mut g = Graph::new();
        g.add_node(NodeKind::EventStart, (0.0, 0.0));
        written::generate(&mut g, "print integer '1'").unwrap();
        assert_eq!(
            g.nodes().filter(|n| n.kind == NodeKind::EventStart).count(),
            1,
            "a second start would stop the program compiling"
        );
    }

    #[test]
    fn problems_name_the_line_and_say_what_to_do() {
        let cases = [
            ("print integer '20'\nwibble 'x'", 2),
            ("declare 'x' integer '1'", 1),
            ("print 'nothere'", 1),
            ("print integer '1' + float '1.5'", 1),
            ("if 'x' < integer '1' {\nprint integer '2'", 1),
        ];
        for (text, line) in cases {
            let mut g = Graph::new();
            let problems = written::generate(&mut g, text)
                .err()
                .unwrap_or_else(|| panic!("should have been refused: {text}"));
            assert_eq!(problems[0].line, line, "wrong line for: {text}");
            assert!(!problems[0].fix.is_empty(), "no fix offered for: {text}");
        }
    }
}

/// Repeat: the first thing in Cat Paws that can run a step more than once.
///
/// Every test here runs the program and reads what it printed. A loop that emits
/// plausible-looking instructions and counts wrong would pass an inspection test and
/// fail these.
#[cfg(test)]
mod repeat_tests {
    use super::wasm_tests::run_wasm;
    use super::*;

    fn typed(text: &str) -> Graph {
        let mut g = Graph::new();
        written::generate(&mut g, text).unwrap_or_else(|p| panic!("should read: {p:#?}"));
        g
    }

    fn ran(text: &str) -> Vec<String> {
        let g = typed(text);
        run_wasm(&wasm::emit(&g).unwrap_or_else(|d| panic!("should compile: {d:#?}")))
    }

    #[test]
    fn a_body_runs_once_per_count() {
        assert_eq!(ran("repeat integer '3' {\nprint string 'meow'\n}"), ["meow"; 3]);
    }

    #[test]
    fn a_count_of_zero_runs_the_body_no_times() {
        // Tested before the body rather than after, so this is zero passes and not one.
        assert!(ran("repeat integer '0' {\nprint string 'meow'\n}").is_empty());
    }

    #[test]
    fn a_negative_count_runs_the_body_no_times() {
        assert!(ran("repeat integer '-4' {\nprint string 'meow'\n}").is_empty());
    }

    #[test]
    fn steps_after_a_repeat_still_run() {
        // The whole point of the `then` pin: a Branch has nowhere to carry on to, and
        // a Repeat does.
        assert_eq!(
            ran("repeat integer '2' {\nprint string 'meow'\n}\nprint string 'done'"),
            ["meow", "meow", "done"]
        );
    }

    #[test]
    fn a_loop_can_count() {
        assert_eq!(
            ran("declare 'n' = integer '0'\n\
                 repeat integer '5' {\n\
                     set 'n' = 'n' + integer '1'\n\
                 }\n\
                 print 'n'"),
            ["5"]
        );
    }

    #[test]
    fn nested_loops_do_not_share_a_counter() {
        // Each Repeat gets its own local. Sharing one would make the inner loop eat the
        // outer loop's count and run the body three times instead of nine.
        assert_eq!(
            ran("declare 'n' = integer '0'\n\
                 repeat integer '3' {\n\
                     repeat integer '3' {\n\
                         set 'n' = 'n' + integer '1'\n\
                     }\n\
                 }\n\
                 print 'n'"),
            ["9"]
        );
    }

    #[test]
    fn the_count_is_read_once_and_not_again() {
        // `times` is wired to a variable the body then changes. Scratch reads its count
        // once too, and a loop that re-read it here would run four times, not two.
        assert_eq!(
            ran("declare 'limit' = integer '2'\n\
                 declare 'n' = integer '0'\n\
                 repeat 'limit' {\n\
                     set 'limit' = integer '4'\n\
                     set 'n' = 'n' + integer '1'\n\
                 }\n\
                 print 'n'"),
            ["2"]
        );
    }

    #[test]
    fn a_branch_inside_a_loop_works() {
        // The loop's own `br` instructions are written outside the body, so an `if`
        // nested in it must not shift what they branch to.
        assert_eq!(
            ran("declare 'n' = integer '0'\n\
                 repeat integer '4' {\n\
                     set 'n' = 'n' + integer '1'\n\
                     if 'n' < integer '3' {\n\
                         print string 'early'\n\
                     } else {\n\
                         print string 'late'\n\
                     }\n\
                 }"),
            ["early", "early", "late", "late"]
        );
    }

    #[test]
    fn the_interpreter_and_the_compiler_agree_about_loops() {
        // Two implementations written from different materials — one a flat list with
        // jump indices, one WebAssembly's structured blocks. Agreement is the evidence.
        for text in [
            "repeat integer '3' {\nprint string 'meow'\n}",
            "repeat integer '0' {\nprint string 'meow'\n}",
            "declare 'n' = integer '0'\nrepeat integer '7' {\nset 'n' = 'n' + integer '2'\n}\nprint 'n'",
            "declare 'n' = integer '0'\nrepeat integer '3' {\nrepeat integer '4' {\nset 'n' = 'n' + integer '1'\n}\n}\nprint 'n'",
            "repeat integer '2' {\nprint string 'a'\n}\nprint string 'b'",
        ] {
            let g = typed(text);
            let interpreted = vm::run_with_limit(&compile(&g).expect("bytecode"), 1_000_000).output;
            let compiled = run_wasm(&wasm::emit(&g).expect("wasm"));
            assert_eq!(interpreted, compiled, "disagreement on:\n{text}");
        }
    }

    #[test]
    fn a_loop_drawn_by_hand_compiles_the_same_way() {
        // Nothing here came from the written form: the nodes are placed and wired
        // directly, the way the canvas does it.
        let mut g = Graph::new();
        let start = g.add_node(NodeKind::EventStart, (0.0, 0.0));
        let count = g.add_node(NodeKind::LitInt(3), (0.0, 100.0));
        let repeat = g.add_node(NodeKind::Repeat, (200.0, 0.0));
        let text = g.add_node(NodeKind::LitStr("meow".into()), (200.0, 200.0));
        let print = g.add_node(NodeKind::Print { ty: DataType::Str }, (400.0, 0.0));

        let out = |node, index| PinRef { node, side: Side::Out, index };
        let inp = |node, index| PinRef { node, side: Side::In, index };
        g.connect(out(start, 0), inp(repeat, 0)).unwrap();
        g.connect(out(count, 0), inp(repeat, 1)).unwrap();
        g.connect(out(repeat, 0), inp(print, 0)).unwrap();
        g.connect(out(text, 0), inp(print, 1)).unwrap();

        assert_eq!(run_wasm(&wasm::emit(&g).unwrap()), ["meow"; 3]);
    }

    #[test]
    fn a_body_wired_back_into_its_own_loop_unplugs_it_instead_of_cycling() {
        // Repeat repeats by *holding* its body, not by the wires going round, so this
        // was written expecting CP-FLOW-03. It cannot happen, and not because of
        // Repeat: every node has exactly one execution input, and a wire into an
        // occupied input replaces what was there. So the back edge steals the input the
        // forward chain was using, and the loop falls off the program instead of
        // closing. (Data wires *can* cycle — a value node has several input pins, so a
        // back edge has a free one to land on. That is why CP-WIRE-02 is reachable and
        // CP-FLOW-03 is not.)
        let mut g = Graph::new();
        let start = g.add_node(NodeKind::EventStart, (0.0, 0.0));
        let count = g.add_node(NodeKind::LitInt(3), (0.0, 100.0));
        let repeat = g.add_node(NodeKind::Repeat, (200.0, 0.0));
        let text = g.add_node(NodeKind::LitStr("meow".into()), (200.0, 200.0));
        let print = g.add_node(NodeKind::Print { ty: DataType::Str }, (400.0, 0.0));

        let out = |node, index| PinRef { node, side: Side::Out, index };
        let inp = |node, index| PinRef { node, side: Side::In, index };
        g.connect(out(start, 0), inp(repeat, 0)).unwrap();
        g.connect(out(count, 0), inp(repeat, 1)).unwrap();
        g.connect(out(repeat, 0), inp(print, 0)).unwrap();
        g.connect(out(text, 0), inp(print, 1)).unwrap();
        assert_eq!(run_wasm(&wasm::emit(&g).unwrap()), ["meow"; 3]);

        // Now wire the print back into the Repeat, which is how someone would try to
        // draw a loop by hand.
        g.connect(out(print, 0), inp(repeat, 0)).unwrap();
        assert!(
            g.target_of(out(start, 0)).is_none(),
            "the Event start should have lost its wire to the Repeat"
        );
        assert!(
            run_wasm(&wasm::emit(&g).unwrap()).is_empty(),
            "nothing is reachable from the start any more"
        );
    }

    #[test]
    fn a_repeat_counting_something_that_is_not_a_whole_number_is_refused() {
        let mut g = Graph::new();
        let problems = written::generate(&mut g, "repeat float '2.5' {\nprint string 'meow'\n}")
            .expect_err("a float count should be refused");
        assert_eq!(problems[0].message, "a repeat counts a whole number of times, not a float");
    }

    #[test]
    fn a_failed_generation_leaves_the_canvas_as_it_was() {
        // Half a program appearing beside a list of problems is worse than nothing.
        let mut g = Graph::new();
        written::generate(&mut g, "declare 'health' = integer '20'").unwrap();
        let before = g.node_ids();

        written::generate(&mut g, "declare 'x' = integer '1'\nrepeat float '2.5' {\nprint 'x'\n}")
            .expect_err("should be refused");

        assert_eq!(g.node_ids(), before, "nodes were left behind");
        assert!(!g.vars.contains_key("x"), "a variable was left behind");
    }

    #[test]
    fn steps_after_an_if_are_reported_rather_than_silently_rewired() {
        // A Branch's only outputs are its two arms. Chaining the next step onto pin 0
        // would replace the true arm — the program would compile and be wrong.
        let mut g = Graph::new();
        let problems = written::generate(
            &mut g,
            "if integer '1' < integer '2' {\nprint string 'yes'\n}\nprint string 'after'",
        )
        .expect_err("should be refused");
        assert_eq!(problems[0].line, 4);
        assert!(problems[0].message.contains("nothing can follow an `if`"));
    }
}

#[cfg(test)]
mod precedence_tests {
    use super::wasm_tests::run_wasm;
    use super::*;

    fn ran(text: &str) -> Vec<String> {
        let mut g = Graph::new();
        written::generate(&mut g, text).unwrap_or_else(|p| panic!("should read: {p:#?}"));
        run_wasm(&wasm::emit(&g).unwrap_or_else(|d| panic!("should compile: {d:#?}")))
    }

    /// Whole-number division truncates, so grouping is not cosmetic here: reading this
    /// right to left gives 6 * (3 / 2) = 6, and left to right gives (6 * 3) / 2 = 9.
    #[test]
    fn multiply_and_divide_group_left_to_right() {
        assert_eq!(ran("print integer '6' * integer '3' / integer '2'"), ["9"]);
        assert_eq!(ran("print integer '6' / integer '2' * integer '3'"), ["9"]);
    }

    #[test]
    fn add_and_subtract_group_left_to_right() {
        assert_eq!(ran("print integer '10' - integer '4' + integer '3'"), ["9"]);
        assert_eq!(ran("print integer '10' - integer '4' - integer '3'"), ["3"]);
    }

    #[test]
    fn multiplying_binds_tighter_than_adding() {
        assert_eq!(ran("print integer '10' - integer '4' * integer '2'"), ["2"]);
    }

    #[test]
    fn a_negative_literal_is_not_a_subtraction() {
        assert_eq!(ran("print integer '-4' + integer '1'"), ["-3"]);
    }

    /// The reduction step the benchmark kernel leans on: x - (x / m) * m.
    #[test]
    fn the_remainder_idiom_groups_correctly() {
        assert_eq!(
            ran("print integer '1000000007' - integer '1000000007' / integer '1000003' * integer '1000003'"),
            ["997010"]
        );
    }
}

/// Overflow is refused while compiling, not wrapped around while running.
///
/// The machine wraps: `i64.add` turns 9223372036854775807 + 1 into a large negative
/// number and reports nothing. Someone learning to program has no reason to suspect the
/// machine rather than themselves, so a wrong answer they believe is worse than an error
/// they can read. Every sum whose operands are already known is therefore worked out
/// while compiling, and refused there if it has no answer.
#[cfg(test)]
mod overflow_tests {
    use super::wasm_tests::run_wasm;
    use super::*;

    fn refused(text: &str) -> Vec<compile::Diagnostic> {
        let mut g = Graph::new();
        written::generate(&mut g, text).unwrap_or_else(|p| panic!("should read: {p:#?}"));
        wasm::emit(&g).expect_err("this should not compile")
    }

    fn ran(text: &str) -> Vec<String> {
        let mut g = Graph::new();
        written::generate(&mut g, text).unwrap_or_else(|p| panic!("should read: {p:#?}"));
        run_wasm(&wasm::emit(&g).unwrap_or_else(|d| panic!("should compile: {d:#?}")))
    }

    #[test]
    fn adding_past_the_top_is_refused() {
        let d = refused("print integer '9223372036854775807' + integer '1'");
        assert_eq!(d[0].code, compile::TOO_BIG);
        assert!(d[0].message.contains("bigger than an integer can hold"), "{:?}", d[0].message);
    }

    #[test]
    fn subtracting_past_the_bottom_is_refused() {
        let d = refused("print integer '-9223372036854775808' - integer '1'");
        assert_eq!(d[0].code, compile::TOO_BIG);
    }

    #[test]
    fn multiplying_past_the_top_is_refused() {
        let d = refused("print integer '4000000000' * integer '4000000000'");
        assert_eq!(d[0].code, compile::TOO_BIG);
    }

    #[test]
    fn dividing_by_zero_is_refused_before_it_runs() {
        // Previously this compiled and the program trapped part-way through, after
        // whatever came before it had already printed.
        let d = refused("print integer '5' / integer '0'");
        assert_eq!(d[0].code, compile::DIVIDE_BY_ZERO);
        assert!(d[0].message.contains("no answer"), "{:?}", d[0].message);
    }

    #[test]
    fn a_literal_too_big_to_be_an_integer_says_so() {
        let mut g = Graph::new();
        let p = written::generate(&mut g, "print integer '99999999999999999999'")
            .expect_err("should be refused");
        assert!(p[0].message.contains("too big to be an integer"), "{:?}", p[0].message);
    }

    #[test]
    fn sums_that_do_fit_are_untouched() {
        assert_eq!(ran("print integer '9223372036854775806' + integer '1'"), ["9223372036854775807"]);
        assert_eq!(ran("print integer '-9223372036854775807' - integer '1'"), ["-9223372036854775808"]);
    }

    #[test]
    fn nested_sums_are_checked_all_the_way_down() {
        // The inner multiply overflows; folding upward has to notice rather than
        // carrying a wrapped value into the outer sum.
        let d = refused("print integer '4000000000' * integer '4000000000' + integer '1'");
        assert_eq!(d[0].code, compile::TOO_BIG);
    }

    #[test]
    fn the_two_backends_still_agree_on_sums_that_fit() {
        for text in [
            "print integer '9223372036854775806' + integer '1'",
            "print integer '-9223372036854775807' - integer '1'",
            "print integer '7' / integer '2'",
            "print integer '-7' / integer '2'",
        ] {
            let mut g = Graph::new();
            written::generate(&mut g, text).unwrap();
            let interpreted = vm::run(&compile(&g).expect("bytecode")).output;
            let compiled = run_wasm(&wasm::emit(&g).expect("wasm"));
            assert_eq!(interpreted, compiled, "disagreement on:\n{text}");
        }
    }

    /// Overflow that only shows up at run time is still silent — worth pinning so the
    /// gap is a recorded fact rather than a surprise.
    #[test]
    fn overflow_through_a_variable_is_not_caught_yet() {
        let out = ran(
            "declare 'x' = integer '9223372036854775807'\n\
             set 'x' = 'x' + integer '1'\n\
             print 'x'",
        );
        assert_eq!(out, ["-9223372036854775808"], "still wraps once a variable is involved");
    }
}

/// Reading a finished module back as text.
///
/// Cat Paws has no intermediate language of its own — `compile.rs` builds the instruction
/// list the *interpreter* runs, which is a second opinion rather than a stage on the way
/// to WebAssembly. So the module itself is the only honest answer to "what did my program
/// become", and it has to be readable.
#[cfg(test)]
mod wasm_text {
    use super::*;

    fn wat_of(source: &str) -> String {
        let mut g = Graph::new();
        written::generate(&mut g, source).expect("should read");
        let bytes = wasm::emit(&g).expect("should compile");
        wasm::text(&bytes).expect("should read back as text")
    }

    #[test]
    fn a_module_reads_back_as_webassembly_text() {
        let wat = wat_of("print string 'hello'");
        assert!(wat.starts_with("(module"), "not a module:\n{wat}");
        assert!(wat.contains("(func"), "no functions:\n{wat}");
        assert!(wat.contains("\"main\""), "main is not exported:\n{wat}");
    }

    /// The arithmetic a person wrote should be findable in what they are shown, or the
    /// view teaches nothing.
    #[test]
    fn the_arithmetic_is_visible_in_it() {
        let wat = wat_of("declare 'x' = integer '2'\nset 'x' = 'x' + 'x'\nprint 'x'");
        assert!(wat.contains("i64.add"), "the addition is missing:\n{wat}");
        assert!(wat.contains("local.set") || wat.contains("local.tee"), "{wat}");
    }

    #[test]
    fn a_loop_shows_the_block_and_loop_pair() {
        // The idiom from wasm.rs: `loop` only jumps backwards, so leaving needs an
        // enclosing `block`. Someone reading this should be able to see that.
        let wat = wat_of("repeat integer '3' {\nprint string 'meow'\n}");
        assert!(wat.contains("loop"), "no loop:\n{wat}");
        assert!(wat.contains("block"), "no block:\n{wat}");
        assert!(wat.contains("br_if"), "no exit branch:\n{wat}");
    }

    #[test]
    fn rubbish_is_refused_rather_than_shown_as_text() {
        assert!(wasm::text(b"not a wasm module at all").is_err());
    }
}

/// Declaring a variable that already exists must not throw its value away.
#[cfg(test)]
mod declare_keeps_the_value {
    use super::*;

    /// The bug as reported: a starting value typed into the panel silently became 0 the
    /// moment a written program mentioning that variable was generated.
    #[test]
    fn generating_a_declare_leaves_an_existing_start_alone() {
        let mut g = Graph::new();
        g.declare_var("Health".into(), DataType::Int);
        g.vars.get_mut("Health").unwrap().initial = Value::Int(i64::MAX);

        written::generate(&mut g, "declare 'Health' = integer '20'").expect("should read");

        assert_eq!(
            g.vars["Health"].initial,
            Value::Int(i64::MAX),
            "declaring an existing variable wiped its starting value"
        );
    }

    #[test]
    fn a_variable_that_did_not_exist_still_gets_made() {
        let mut g = Graph::new();
        written::generate(&mut g, "declare 'fresh' = integer '7'").expect("should read");
        assert!(g.vars.contains_key("fresh"));
        assert_eq!(g.vars["fresh"].initial, Value::Int(0), "a new one starts at the default");
    }

    /// Changing the type is a different variable in all but name, so the old starting
    /// value cannot be kept — it is not even the right kind of value any more.
    #[test]
    fn a_different_type_replaces_it() {
        let mut g = Graph::new();
        g.declare_var("x".into(), DataType::Int);
        g.vars.get_mut("x").unwrap().initial = Value::Int(99);

        written::generate(&mut g, "declare 'x' = string 'hello'").expect("should read");
        assert_eq!(g.vars["x"].ty, DataType::Str);
        assert_eq!(g.vars["x"].initial, Value::Str(String::new()));
    }

    /// `set` never touched it, and still must not.
    #[test]
    fn setting_leaves_the_start_alone_too() {
        let mut g = Graph::new();
        g.declare_var("n".into(), DataType::Int);
        g.vars.get_mut("n").unwrap().initial = Value::Int(41);
        written::generate(&mut g, "set 'n' = integer '5'").expect("should read");
        assert_eq!(g.vars["n"].initial, Value::Int(41));
    }
}

/// Comparing whole numbers the two backends must agree on.
///
/// The compiled path emits `i64.lt_s`; the interpreter used to go through
/// `Value::as_number`, which casts to `f64`. Above 2^53 an `f64` cannot tell neighbouring
/// whole numbers apart, so the two answered differently — and a divergence between the
/// oracle and the thing it checks is the one failure that makes every other test weaker.
#[cfg(test)]
mod comparing_large_numbers {
    use super::wasm_tests::run_wasm;
    use super::*;

    fn both_ways(a: i64, b: i64) -> (Vec<String>, Vec<String>) {
        let src = format!(
            "declare 'x' = integer '{a}'\n\
             if 'x' < integer '{b}' {{\n\
                 print string 'less'\n\
             }} else {{\n\
                 print string 'not less'\n\
             }}"
        );
        let mut g = Graph::new();
        written::generate(&mut g, &src).expect("should read");
        (
            vm::run(&compile(&g).expect("bytecode")).output,
            run_wasm(&wasm::emit(&g).expect("wasm")),
        )
    }

    #[test]
    fn neighbours_past_the_reach_of_a_float_still_compare() {
        for (a, b) in [
            (9_007_199_254_740_993_i64, 9_007_199_254_740_994_i64), // just past 2^53
            (922_337_203_685_477_580, 922_337_203_685_477_581),
            (i64::MAX - 1, i64::MAX),
            (i64::MIN, i64::MIN + 1),
        ] {
            let (interpreted, compiled) = both_ways(a, b);
            assert_eq!(interpreted, compiled, "the two backends disagree on {a} < {b}");
            assert_eq!(interpreted, vec!["less"], "{a} < {b} should be true");
        }
    }

    #[test]
    fn the_larger_number_is_not_less() {
        for (a, b) in [(i64::MAX, i64::MAX - 1), (9_007_199_254_740_994_i64, 9_007_199_254_740_993)] {
            let (interpreted, compiled) = both_ways(a, b);
            assert_eq!(interpreted, compiled, "disagreement on {a} < {b}");
            assert_eq!(interpreted, vec!["not less"]);
        }
    }

    /// Equal is not less, at any size — the boundary the comparison is named after.
    #[test]
    fn a_number_is_not_less_than_itself() {
        for v in [0_i64, 50, i64::MAX, i64::MIN, 9_007_199_254_740_993] {
            let (interpreted, compiled) = both_ways(v, v);
            assert_eq!(interpreted, compiled, "disagreement on {v} < {v}");
            assert_eq!(interpreted, vec!["not less"]);
        }
    }
}

/// A quoted word that names no variable should say what it looks like you meant.
///
/// Quotes alone mean *the thing called this*, and a value announces its type first. The
/// rule earns its keep — it is the only thing separating `print 'health'` from
/// `print string 'health'` — but it turns a beginner's `= '10000'` into "there is no
/// variable called '10000'", which is true and useless.
#[cfg(test)]
mod naming_something_that_is_not_there {
    use super::*;

    fn refused(text: &str) -> Vec<written::Problem> {
        let mut g = Graph::new();
        written::generate(&mut g, text).expect_err("should be refused")
    }

    #[test]
    fn a_number_in_quotes_is_told_how_to_be_a_number() {
        let p = refused("declare 'x' = '10000'");
        assert!(
            p[0].message.contains("read as the name of a variable"),
            "unhelpful: {}", p[0].message
        );
        assert!(p[0].fix.contains("integer '10000'"), "no spelling offered: {}", p[0].fix);
    }

    #[test]
    fn a_decimal_and_a_boolean_are_told_too() {
        assert!(refused("declare 'x' = '1.5'")[0].fix.contains("float '1.5'"));
        assert!(refused("declare 'x' = 'true'")[0].fix.contains("boolean 'true'"));
        assert!(refused("declare 'x' = '-42'")[0].fix.contains("integer '-42'"));
    }

    /// A word that is not a value is probably a typo or a missing declaration — but it
    /// might be text, so that spelling is offered as well.
    #[test]
    fn a_word_keeps_the_old_advice_and_gains_one() {
        let p = refused("declare 'x' = 'helth'");
        assert!(p[0].message.contains("there is no variable called 'helth'"));
        assert!(p[0].fix.contains("spelling"));
        assert!(p[0].fix.contains("string 'helth'"), "text was not offered: {}", p[0].fix);
    }

    /// A variable that really does exist still just works.
    #[test]
    fn naming_a_real_variable_is_untouched() {
        let mut g = Graph::new();
        written::generate(&mut g, "declare 'a' = integer '3'\ndeclare 'b' = 'a'")
            .expect("naming an existing variable should work");
        assert!(g.vars.contains_key("b"));
    }

    /// Reading a number is the point, so it must still reach the program.
    #[test]
    fn the_suggested_spelling_actually_works() {
        let mut g = Graph::new();
        written::generate(&mut g, "declare 'x' = integer '10000'\nprint 'x'").expect("should read");
        let out = super::wasm_tests::run_wasm(&wasm::emit(&g).expect("wasm"));
        assert_eq!(out, vec!["10000"]);
    }
}

/// Advice has to name a gesture the editor actually has.
///
/// `CP-FLOW-01` — the first error anybody meets, since it is what you get the moment you
/// delete the Event start — said "right-click the canvas and add an Event start". There is
/// no context menu in Cat Paws and there never was: right-drag pans, and `index.html`
/// suppresses the browser's own menu so the pan is not interrupted. The advice could not
/// be followed, which is worse than no advice, because it teaches that the messages cannot
/// be trusted.
#[cfg(test)]
mod advice_names_a_real_gesture {
    use super::*;

    /// Everything a diagnostic can tell you to do, gathered by making each error happen.
    fn every_fix() -> Vec<(String, String)> {
        let mut out = Vec::new();

        // No Event start.
        let g = Graph::new();
        for d in compile(&g).unwrap_err() {
            out.push((d.code.render(), d.fix));
        }

        // Two Event starts.
        let mut g = Graph::new();
        g.add_node(NodeKind::EventStart, (0.0, 0.0));
        g.add_node(NodeKind::EventStart, (200.0, 0.0));
        for d in compile(&g).unwrap_err() {
            out.push((d.code.render(), d.fix));
        }

        // An empty input pin, and a divide by zero, and an overflow.
        for src in [
            "print integer '1' + integer '1'",
            "print integer '5' / integer '0'",
            "print integer '9223372036854775807' + integer '1'",
        ] {
            let mut g = Graph::new();
            if written::generate(&mut g, src).is_ok() {
                if let Err(ds) = compile(&g) {
                    for d in ds {
                        out.push((d.code.render(), d.fix));
                    }
                }
            }
        }

        // A pin left empty, built by hand.
        let mut g = Graph::new();
        let start = g.add_node(NodeKind::EventStart, (0.0, 0.0));
        let print = g.add_node(NodeKind::Print { ty: DataType::Str }, (200.0, 0.0));
        g.connect(
            PinRef { node: start, side: Side::Out, index: 0 },
            PinRef { node: print, side: Side::In, index: 0 },
        )
        .unwrap();
        for d in compile(&g).unwrap_err() {
            out.push((d.code.render(), d.fix));
        }

        out
    }

    /// The editor has no context menu. Nothing may suggest one.
    #[test]
    fn nothing_tells_you_to_right_click() {
        for (code, fix) in every_fix() {
            assert!(
                !fix.to_lowercase().contains("right-click"),
                "{code} tells you to right-click, which pans the canvas: {fix}"
            );
        }
    }

    /// No advice may name a gesture this editor does not have.
    ///
    /// Stated as a prohibition rather than a requirement, because plenty of good advice
    /// names no gesture at all — "change the second number to anything other than zero"
    /// is about the value, not about clicking. What goes wrong is inventing a gesture,
    /// which is exactly what happened with right-click.
    #[test]
    fn no_advice_invents_a_gesture() {
        // Things other editors have and this one does not.
        let absent = [
            "right-click",
            "right click",
            "context menu",
            "double-click",
            "double click",
            "middle-click",
            "menu bar",
            "toolbar button",
            "press escape",
        ];
        for (code, fix) in every_fix() {
            let lower = fix.to_lowercase();
            for gesture in absent {
                assert!(
                    !lower.contains(gesture),
                    "{code} tells you to {gesture}, which Cat Paws has no such thing as: {fix}"
                );
            }
        }
    }

    /// The first error anybody meets should point at the palette, which is where nodes
    /// actually come from.
    #[test]
    fn a_missing_start_points_at_the_palette() {
        let fix = &compile(&Graph::new()).unwrap_err()[0].fix;
        assert!(fix.contains("ADD NODE"), "it does not say where to find one: {fix}");
    }
}
