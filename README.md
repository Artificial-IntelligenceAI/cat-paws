# Cat Paws

A visual programming editor in the spirit of Scratch, but built around an
Unreal-Blueprint-style **node graph** instead of snapping blocks — nodes float on
a canvas and you wire them together.

Written in Rust, runs in the browser via WebAssembly, and natively on the desktop
from the same code.

## Two kinds of wire

- **Execution wires** (grey, arrow pins) decide *what happens next*. `Branch` has
  a true and a false output, so execution goes one way or the other.
- **Data wires** (coloured, round pins) carry *values* into pins. Wire colour is
  the type: cyan integer, green float, red boolean, pink string.

A pin only accepts a wire of a type it can take, so an invalid connection cannot
be drawn in the first place. Illegal drags turn red before you release.

## Compile, then run

The two toolbar buttons are deliberately separate:

- **Compile** (hammer) walks the execution wires, reports every problem it finds,
  and lowers the graph into a flat list of instructions.
- **Compile & Run** (play) does that, then executes the instructions.

Compiling is not just validation — it produces a real program. Tick
*Show compiled code* to see the instructions the hammer generated.

## Memory

A program allocates nothing while it runs: numbers live in WebAssembly locals and every
string is a literal written into the module up front. So there is no allocator and nothing
can leak, because nothing can be allocated.

That changes the moment a value can be built whose size is not known until it runs — string
concatenation, or lists. The plan from there is a mark-sweep collector of our own, and the
two decisions that have to come first are written down in
[docs/memory.md](docs/memory.md).

## Running it

In the browser:

```bash
cd crates/app && trunk serve
```

Then open <http://127.0.0.1:8080>.

Natively:

```bash
cargo run -p cat-paws-app
```

Tests for the language itself (no window needed):

```bash
cargo test -p cat-paws-core
```

## Layout

| Path | What lives there |
| --- | --- |
| `crates/core` | The graph model, type system, compiler and VM. No UI dependencies, so it tests in milliseconds. |
| `crates/app` | The editor: hand-built canvas, Solarized theme, hand-painted toolbar icons. |

The split is the important part: adding a language feature means adding a
`NodeKind` in `core` and handling it in `compile` and `vm`. The canvas derives its
layout from each node's pin list, so it needs no changes to draw or wire it.

## Controls

| | |
| --- | --- |
| Left-drag a node | Move it |
| Right-drag | Pan the workspace |
| Scroll | Zoom |
| Drag a pin | Draw a wire |
| Alt-click a pin | Cut its wires |
| Delete / Backspace | Remove the selected node |
| Cmd+Z / Ctrl+Z | Undo |

The two mouse buttons are kept separate on purpose: the left button acts on the
graph (move nodes, pull wires) and the right button moves the view. Panning works
from anywhere, including on top of a node, so you never have to find empty canvas
to move around.

## Theme

Solarized, light and dark. Solarized keeps its eight accent colours identical
across both modes and swaps only the greys — so wire colours stay meaningful when
you flip the theme.

## License

Licensed under the [Apache License, Version 2.0](LICENSE).
