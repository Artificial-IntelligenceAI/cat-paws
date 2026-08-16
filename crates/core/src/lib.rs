//! Cat Paws core: the graph, the compiler and the virtual machine.
//!
//! Nothing here depends on egui, so the whole language can be tested in
//! milliseconds without opening a window.

pub mod compile;
pub mod graph;
pub mod types;
pub mod vm;

pub use compile::{compile, Diagnostic, Expr, Instr, Program};
pub use graph::{Category, Graph, Link, Node, NodeId, NodeKind, Pin, PinKind, PinRef, Side};
pub use types::{DataType, Value};
pub use vm::{run, RunResult};

#[cfg(test)]
mod tests {
    use super::*;

    fn wire(g: &mut Graph, from: NodeId, fi: usize, to: NodeId, ti: usize) {
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
    fn reference_graph(health: i64) -> Graph {
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
