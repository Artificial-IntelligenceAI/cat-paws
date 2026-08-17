# The written form

Cat Paws is a visual language. This is a way to *type* a program instead of dragging it,
for when typing is faster.

## What it is, and is not

**It makes nodes.** You type, it creates nodes on the canvas, already wired. That is all it
does.

**It is not a file format.** The graph remains the only program there is. Text is never
saved, never read back out of a graph, and never has to agree with anything.

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

## Still to decide

- How a statement ends — newline, or something written
- Arithmetic: `+ - × ÷`, and whether `×` or `*`
- Comments
- Whether `set` is a separate word from `declare`
- What happens to nodes that already exist when text is generated: added alongside, or
  replacing the canvas
