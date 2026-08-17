# Error codes

Every problem the compiler reports carries a stable code, like `CP-WIRE-01`.

The code is a *handle*: something to search for, link to, and eventually hang an
explanation of the rule on. The wording of a message may improve over time; the code it
carries should not change under it.

## What a diagnostic is made of

```
the rule: every coloured input needs a wire. There is no default value to fall back on.
• 'text' on Print has nothing wired into it, so there is no value to use
     try: wire something into 'text' — it takes a string
     CP-WIRE-01
```

- **the rule** — what Cat Paws expects, in general. Stated so a reader learns the language
  and not only this mistake.
- **what is wrong**, in the reader's terms
- **what to change** — someone meeting the language for the first time needs this more
  than the second line, so it is always present
- **where** — the node, outlined on the canvas and one click from the panel entry, which
  centres the view on it. This is the visual language's equivalent of a line number.

The code sits last and quiet. A beginner reads the sentences; the code is there for when
they want to go and find out more.

### The rule appears once per code

The first diagnostic with a given code carries its rule; later ones with the same code do
not. Meeting a rule once teaches it, and eight copies of the same paragraph is a wall to
scroll past.

The rules are **written by hand**. Generating them from the compiler's own conditions would
give something true and useless — the check behind an empty pin is "source_of returned
None", which teaches nobody what Cat Paws expects. A test requires every code to have one,
so a new code cannot quietly ship without it, and requires each to describe the language
rather than an instance: a rule containing "this" or "here" is rejected.

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

## The rules

| Code | The rule |
|---|---|
| `CP-FLOW-01` | a program starts at an Event start node, so every program needs exactly one. |
| `CP-FLOW-02` | a program has one beginning, so only one Event start may be on the canvas. |
| `CP-FLOW-03` | the grey wires run forwards only. A step cannot lead back to one that already ran. |
| `CP-FLOW-04` | an Event start begins a program, so nothing may lead into it. |
| `CP-FLOW-05` | grey wires join steps and coloured wires carry values. A node that produces a value is not a step. |
| `CP-WIRE-01` | every coloured input needs a wire. There is no default value to fall back on. |
| `CP-WIRE-02` | a value is worked out from the wires feeding it, so it cannot be worked out from itself. |
| `CP-WIRE-03` | only a node with a coloured output produces a value that something else can read. |
| `CP-NAME-01` | a variable is created in the Variables panel before a node can read or write it. |

Read together they are close to a summary of what Cat Paws expects, which is the test of
whether they are pulling their weight.
