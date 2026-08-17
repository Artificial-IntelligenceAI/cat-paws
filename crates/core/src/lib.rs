//! Cat Paws core: the graph, the compiler and the virtual machine.
//!
//! Nothing here depends on egui, so the whole language can be tested in
//! milliseconds without opening a window.

pub mod compile;
pub mod graph;
pub mod types;
pub mod vm;
pub mod wasm;

pub use compile::{compile, Area, Code, Diagnostic, Expr, Instr, Program};
pub use graph::{Category, Graph, Link, Node, NodeId, NodeKind, Pin, PinKind, PinRef, Side};
pub use types::{DataType, Value};
pub use vm::{run, RunResult};
pub use wasm::emit;

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
        let low = g.add_node(NodeKind::Print, (420.0, -40.0));
        let fine = g.add_node(NodeKind::Print, (420.0, 80.0));
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
        let print = g.add_node(NodeKind::Print, (200.0, 0.0));
        wire(&mut g, start, 0, print, 0);
        let diags = compile(&g).expect_err("print with no text should not compile");
        assert!(diags[0].message.contains("nothing wired into it"));
    }

    #[test]
    fn a_data_input_only_accepts_one_wire() {
        let mut g = Graph::new();
        let a = g.add_node(NodeKind::LitStr("a".to_string()), (0.0, 0.0));
        let b = g.add_node(NodeKind::LitStr("b".to_string()), (0.0, 100.0));
        let print = g.add_node(NodeKind::Print, (200.0, 0.0));
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
mod wasm_tests {
    use super::tests::{reference_graph, wire};
    use super::*;
    use std::sync::{Arc, Mutex};

    /// Instantiate a module, run `main`, and collect whatever it printed.
    fn run_wasm(bytes: &[u8]) -> Vec<String> {
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
        let show = g.add_node(NodeKind::Print, (400.0, 0.0));
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
                let extra = g.add_node(NodeKind::Print, (900.0, 0.0));
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
        let all = [
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

