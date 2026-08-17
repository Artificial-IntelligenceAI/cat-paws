# Error codes

Every problem the compiler reports carries a stable code, like `CP-WIRE-01`.

The code is a *handle*: something to search for, link to, and eventually hang an
explanation of the rule on. The wording of a message may improve over time; the code it
carries should not change under it.

## What a diagnostic is made of

Three pieces, and the panel shows all three:

```
• 'text' on Print has nothing wired into it, so there is no value to use
     try: wire something into 'text' — it takes a string
     CP-WIRE-01
```

- **what is wrong**, in the reader's terms
- **what to change** — someone meeting the language for the first time needs this more
  than the first line, so it is always present
- **where** — the node, outlined on the canvas and one click from the panel entry, which
  centres the view on it. This is the visual language's equivalent of a line number.

The code sits last and quiet. A beginner reads the sentences; the code is there for when
they want to go and find out more.

## The areas

Named after what is on screen, not after compiler phases.

| Area | What it covers |
|---|---|
| `FLOW` | the grey wires — the order steps happen in |
| `WIRE` | the coloured wires — values travelling between pins |
| `NAME` | variables |

## The codes

| Code | Problem |
|---|---|
| `CP-FLOW-01` | No Event start node, so nothing says where the program begins |
| `CP-FLOW-02` | More than one Event start node |
| `CP-FLOW-03` | Execution wires lead back to a node they already passed through |
| `CP-FLOW-04` | An Event start wired into the middle of a chain |
| `CP-FLOW-05` | A value node placed in the execution chain |
| `CP-WIRE-01` | An input pin with nothing wired into it |
| `CP-WIRE-02` | Data wires lead back on themselves — a value worked out from itself |
| `CP-WIRE-03` | Something in a value position that produces no value |
| `CP-NAME-01` | A node refers to a variable that does not exist |

## Rules

- A code belongs to **one** kind of problem. Two sharing a code would make the code
  useless as a handle, since searching it would explain the wrong thing. A test enforces
  this.
- **Both backends report the same code** for the same problem. The bytecode compiler and
  the WebAssembly compiler each walk the graph, so a broken program must be refused by
  both, with the same wording and the same code. A test enforces this too.
- Codes are **append-only**. Retire one by leaving a gap rather than renumbering, or a
  link written down somewhere will quietly start pointing at a different problem.

## Not built yet

A `rule conditions:` line — a sentence stating the rule the code enforces, rather than this
instance of breaking it, so a reader learns the language and not just this mistake. The
codes exist so that line has something to hang on. AHPCL shows its rule the first time a
code appears in a run and not on repeats, which is probably right here too.
