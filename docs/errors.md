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

Every node a person can place carries one:

| Node | What it warns about |
|---|---|
| `Event start` | anything not joined to this chain never runs, silently |
| whole-number `+ − ×` | going past the ceiling makes the answer wrong with no warning |
| whole-number `÷` | the remainder is thrown away; dividing by zero stops the program |
| decimal arithmetic | approximate, and stops counting whole numbers exactly above 2⁵³ |
| `Repeat` | the count is read once, before the first pass |
| `Branch` | nothing can follow it — each arm ends where it ends |
| `Less than` | whole numbers only, and strictly less than |
| `Print` | writes to the output panel, not onto the canvas |
| `Set` | writes **once**; it is not a rule that keeps holding |
| a variable | reads at the moment execution passes, so it can differ each time round |
| a number | the range, and that arriving past it makes the answer wrong |
| a decimal | some values change the moment they are typed |
| a boolean | true or false, not 1 and 0 |
| text | `"5"` is the character five, not the number 5 |

This began the other way round, with six kinds carrying nothing on the reasoning that **an
icon on every node is an icon nobody reads**. The reasoning was sound and the application
of it was not. `Set` writing once rather than staying true — *"I set total from score, why
didn't total change when score did?"* — is the single most common thing a beginner gets
wrong about programming anywhere, and it had no mark at all. When every node genuinely has
a sharp edge, every node earns one; the icon is a "what should I know here", not a hazard
sign.

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

## Nothing in the compiler substitutes a value in silence

Every fallback in `compile.rs` reports before it substitutes. That was almost true and is
now actually true: `Values::output_expr` returned `Value::Int(0)` without a word when a
wire's source node was missing from the graph, and both execution walks ended the chain
just as quietly.

None of the three should be reachable — `Graph::remove_node` drops every link touching a
node, and `connect` validates both pin indices before storing a link — so this is not a fix
for a bug anyone can hit today. It is the rule made uniform. A program that compiles clean
and runs wrong is the single outcome this language exists to refuse, and a fallback that
stays quiet is how that outcome arrives.

## `Less than` compares whole numbers as whole numbers

Worth recording because it is the only time the two implementations have ever disagreed.

The compiled path emits `i64.lt_s`. The interpreter went through `Value::as_number`, which
casts to `f64` — and an `f64` cannot tell apart two whole numbers above 2^53. So:

```
9223372036854775806 < 9223372036854775807
   interpreter:  not less      ← wrong
   WebAssembly:  less          ← right
```

`as_number` existed so one comparison could serve both integers and decimals, but the pins
on `Less than` are both integers: there was never a decimal case to serve. Integers now
compare as integers, and the divergence is covered by tests at four sizes, including either
side of 2^53 and both ends of the range.

A disagreement between the oracle and the thing it checks is worse than an ordinary bug: it
makes every other test weaker, because agreement stops meaning what it is supposed to mean.

## A paused tab says so

Not a compiler message, but the same principle, and it cost a full afternoon of
misdiagnosis before it was written down.

The editor draws from `requestAnimationFrame`, and a browser gives none to a page that is
not in front. So a backgrounded tab paints **nothing** — not a stale frame, not a partial
one. The canvas stays cleared.

`index.html` paints the body the same colour as the canvas on purpose, so there is no white
flash while the WebAssembly loads. The consequence is that a paused tab and a working one
are *pixel-identical*: a flat rectangle either way, with no way to tell whether the program
is fine, still loading, or broken.

It now says which:

```
Cat Paws is paused
A browser stops drawing a tab that is not in front,
so the canvas is blank rather than broken.
Bring this tab to the front and it carries on.
```

Plain page text rather than anything drawn on the canvas — so it appears precisely when the
canvas cannot, including in a screenshot.

## Advice must name a gesture the editor has

`CP-FLOW-01` said **"right-click the canvas and add an Event start"**. Cat Paws has no
context menu and never has: right-drag pans, and `index.html` suppresses the browser's own
menu so that pan is not interrupted. The advice could not be followed at all.

It is the first error anybody meets — deleting the Event start is the quickest way to reach
it — so the first thing a new user was told to do was a thing that does nothing. That is
worse than saying nothing, because it teaches that the messages are not to be trusted, and
every message after it is then read with that in mind.

The same fault, quieter, ran through the rest: **"unplug the wire"** is not a gesture
anybody can guess. The editor's word is *alt-click a pin to cut*, and that is what the
advice says now.

```
before   right-click the canvas and add an Event start, then wire it to the first step
after    click Event start in the ADD NODE list, then drag a wire from its 'then' pin
         to the first step

before   break the loop by unplugging one of the grey wires
after    alt-click one of the grey pins to cut that wire
```

A test now makes every error happen and reads its advice back, forbidding any gesture the
editor does not have — right-click, double-click, a context menu, a menu bar. It is written
as a prohibition rather than a requirement on purpose: plenty of good advice names no
gesture at all, because it is about the value rather than about clicking.
