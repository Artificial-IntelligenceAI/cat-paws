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

## `CP-MATH-01` and `CP-MATH-02` — arithmetic with no answer

A fourth area, added because these are neither a wiring mistake nor a naming one: the
wires are right and the sum has no answer.

**`CP-MATH-01`** — a result outside what an integer holds.

```
print integer '9223372036854775807' + integer '1'

    9223372036854775807 + 1 is bigger than an integer can hold
    try: an integer goes up to 9223372036854775807 — use smaller numbers, or
         floats, which reach much further
```

The machine does not do this on its own. `i64.add` **wraps**: it turns that sum into
−9223372036854775808 and reports nothing, which is what C, Java, Go and Rust-in-release all
do. That is a defensible default for someone who knows to expect it, and a trap for someone
who does not. A beginner has no reason to suspect the machine rather than themselves, so a
wrong answer they believe is worse than an error they can read.

**`CP-MATH-02`** — dividing by zero.

```
print integer '5' / integer '0'

    this divides by zero, which has no answer
    try: change the second number to anything other than zero
```

Previously this compiled, ran, and trapped part-way through — after everything before it had
already printed, so the output stopped mid-way with no explanation.

### What this reaches, and what it does not

Every sum whose two sides are **already known while compiling** is worked out then, and
refused there if it has no answer. That covers literals, and nested sums built only from
literals, all the way down.

It does **not** reach a value that arrives through a variable:

```
declare 'x' = integer '9223372036854775807'
set 'x' = 'x' + integer '1'
print 'x'                            → -9223372036854775808, silently
```

Catching that means checking every addition while the program runs — in WebAssembly, an
`i64.add` followed by a comparison and a branch, on every operation. Cheap individually,
and the benchmark says Cat Paws currently keeps pace with native Rust precisely because it
does none of it. That is a real trade and it has not been made yet; a test pins the current
behaviour so the gap is a recorded fact rather than a surprise.

## Cautions — the `ⓘ` on a node

Separate from errors, and deliberately so. An error means *this program will not compile*.
A caution means *this node behaves in a way that catches people out*, and the program is
perfectly fine.

They live on `NodeKind::caution` in the core rather than in the editor, because they are
facts about the language and a second copy in the drawing code would drift away from the
first.

Only nodes with a genuine sharp edge carry one:

Each one is in two halves: what the node does, then **what that means for you**. The first
half alone is not much use — "whole numbers wrap around" tells a beginner what happens,
not that their counter will silently go negative and every number after it will be wrong.
A test enforces that both halves are present, and that they come in that order.

| Node | What it warns about |
|---|---|
| whole-number `+ − ×` | the ceiling, and that going past it makes the answer wrong with no warning |
| whole-number `÷` | the remainder is thrown away, and dividing by zero stops the program |
| decimal arithmetic | decimals are approximate, and stop counting whole numbers exactly above 2⁵³ |
| `Repeat` | the count is read once, before the first pass |
| `Branch` | nothing can follow it — each arm ends where it ends |
| `Less than` | whole numbers only, and strictly less than |
| an integer literal | the range it can hold |
| a decimal literal | some values change the moment they are typed |

`Event start`, `Print`, `Set`, a variable, a boolean and a string carry none. **An icon on
every node is an icon nobody reads**, so the mark has to mean something when it appears.

The icon sits at the right of the header and stays dim until the pointer is near it. A
permanent bright mark on half the canvas reads as a fault, and none of these are faults.

### Floats are not the way round the integer ceiling

Worth stating on its own, because this project got it wrong first time and the mistake is
an easy one to make:

```
floats stop counting whole numbers exactly above   9,007,199,254,740,992   (2^53)
an integer runs out at                             9,223,372,036,854,775,807
```

A float reaches numbers a thousand times larger — but it stops being **exact** a thousand
times *sooner*. Add 1 to 9,007,199,254,740,992 as a float and you get the same number back,
silently. So "use a float instead" sends someone who has hit the integer ceiling somewhere
strictly worse: integers at least announce nothing and wrap to an obviously wrong negative,
while a float quietly stops moving.

`CP-MATH-01` used to end with *"or floats, which reach much further"*. It no longer does,
and a test now checks that anywhere the integer ceiling is named, the float one is named
beside it.
