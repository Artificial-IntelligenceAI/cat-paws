# The written form

Cat Paws is a visual language. This is a way to *type* a program instead of dragging it,
for when typing is faster.

## Where it lives

Its own tab, beside the canvas. It began as a five-row box wedged into the side panel,
which was enough to show the idea worked and far too small to write a program in. It
authors whole programs, so it gets the room the canvas has.

Pressing **Create nodes** switches back to the canvas, because what you wanted was the
nodes, and leaving you looking at an empty box makes you go and check.

While the Write tab is showing, the palette and the selected-node inspector are hidden —
dragging a node out has nowhere to land, and "selected node" is a canvas idea. The
variables stay, because knowing what already exists is exactly what you want while typing.

## What it is, and is not

**It makes nodes.** You type, it creates nodes on the canvas, already wired. That is all it
does.

**It is not a file format.** The graph remains the only program there is. Text is never
saved, never read back out of a graph, and never has to agree with anything.

**It can be undone.** Everything a program generates comes back in a *single* step, not one
node at a time — ten lines and a dozen nodes are one thing you did, so they are one thing to
take back. A program that is refused leaves no undo step at all, since it changed nothing
and an undo that does nothing reads as broken.

**It does not touch layout.** Where nodes sit is the canvas's business. The written form
has no way to say where something goes, and moving a node in the editor cannot change any
text, because there is no text to change.

Everything below follows from those three sentences. There is no node identity to track, no
sidecar file, no round-trip, and nothing to keep in sync — because generation only goes one
way.

## One line makes as many nodes as it needs

A line says what you mean; the generator builds whatever that takes.

```
declare 'health' = integer '20'
```

produces three things: the variable `health` in the panel, a literal node holding `20`, and
a Set node, wired together.

```
if 'health' < integer '50' {
    print string 'low health'
} else {
    print string 'fine'
}
```

produces seven more: a getter, a literal, a Less than, a Branch, two Prints and two string
literals — with the true and false arms wired from the shape of the braces.

The alternative was one node per line, which is faithful to the canvas and almost as slow
as dragging: thirteen lines for that program instead of six, every node needing a name
invented for it so the next line could refer to it, and two extra lines at the end just to
wire execution.

The cost of the choice, stated plainly: **the text is not a listing of the canvas.** One
line can become four nodes, so reading it does not tell you exactly what will appear.

## A bare name is a name; a value says its type

Quotes alone mean *the thing called this*. A literal always announces its type first.

```
print 'health'              the variable
print string 'health'       the six letters
```

Without the rule those two lines are the same text meaning different things.

The rule has one sharp edge, so the error covers it. `declare 'x' = '10000'` reads `'10000'`
as *the variable called 10000* and finds none — which is correct, and useless on its own:

```
line 1: '10000' is read as the name of a variable here, not as a value
     try: a value says its type first — write integer '10000'
```

A quoted word that names nothing and looks like a value now says which spelling it wanted:
`integer`, `float` or `boolean` as appropriate. A word that looks like a word keeps the old
advice about spelling, and gains `string 'that'` in case text was what was meant.

## The types are the ones on the canvas

| Written | Pin |
|---|---|
| `integer` | integer |
| `float` | float |
| `string` | string |
| `boolean` | boolean |

The same words the pins already show, so the two views never disagree about what something
is called. Friendlier words were considered — `number`, `whole`, `yes-no` — and dropped:
a beginner who learns `yes-no` here then reads `boolean` on the pin has been taught two
names for one thing.

`integer` and `float` are separate because they are separate pins. A whole number will not
wire into a float input, and the written form cannot hide that.

## The rest of the syntax

**A newline ends a statement.** Nothing is written to end one. This is a typing aid, not a
file format, so there is no reason to make the reader type punctuation.

**`declare` and `set` are different words** because they do different things: `declare`
adds a variable to the panel *and* writes to it; `set` only writes to one that exists.
Setting something undeclared is refused rather than quietly declaring it.

**Arithmetic is `+ - * /`**, with `×` and `÷` also accepted. The node titles show `×` and
`÷`, but neither is on a keyboard, and a written form nobody can type has missed the point.

**`#` starts a comment**, to the end of the line.

Precedence is the usual one: comparison binds loosest, then `+ -`, then `* /`. So
`integer '10' - integer '4' * integer '2'` is 2, not 12.

## Generated nodes are added, never substituted

What is written appears beside what is already on the canvas, wired among itself. Nothing
built by hand can be destroyed by typing. Generating twice leaves two copies, and the way
to remove one is to delete it.

A fragment is given its own `Event start` **only if the canvas has none**. Adding a second
would stop a working program compiling, so an existing program is left for the reader to
wire the new piece into.

## Repeating

```
repeat integer '10' {
    print string 'meow'
}
print string 'done'
```

`repeat` takes a whole number and a block. The count is read **once**, before the first
pass, so changing what fed it from inside does not lengthen the loop — the same rule
Scratch uses. A count of zero, or a negative one, runs the body no times.

Unlike `if`, something may follow a `repeat`. A Branch's only outputs are its two arms, so
there is nowhere for a following step to attach; a Repeat has a `then` pin for exactly
that. Writing a step after an `if` is reported rather than quietly wired into the true arm.

## What it does not do yet

Functions, since the graph has none. Loops that stop on a condition rather than a count.
And there is no way to write a node that has no place in a statement — a value node
sitting on its own — which is fine, because that is what dragging is for.

