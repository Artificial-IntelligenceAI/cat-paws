# Memory

## Today: nothing to manage

Cat Paws allocates nothing while a program runs.

| Kind | Where it lives |
|---|---|
| `Int`, `Float`, `Bool` | a WebAssembly local — the engine's own stack |
| `Str` | a data segment, written when the module is built |

Every string in the language comes from a literal. There is no way to build a new one
while running — no concatenation, no lists, nothing of unknown size — so the complete set
is known while compiling and is laid out in the module's data section up front. Identical
strings share one record.

That makes the current scheme **static allocation**: the same shape as a C program that
never calls `malloc`. No allocator, no free, no reference counts, no collector, because
there is nothing to manage. A Cat Paws program cannot leak, because it cannot allocate.

## When that stops being true

The first feature that forces a heap is **string concatenation** — joining two strings
makes a new one whose length is not known until it runs, and it has to go somewhere.
Lists would do the same, and sooner.

At that point the module needs a heap inside its linear memory, which is just a large byte
array the module owns, and an allocator written into the emitted code.

## The plan: a collector, written by hand

**Decided: a mark-sweep garbage collector of our own, over linear memory.** Not reference
counting, and not WasmGC.

Against **reference counting**: it means emitting retain and release around every heap
reference, which is bulk in the output and work in the inner loop. It also cannot reclaim
cycles, and once a list can hold another list, cycles are expressible.

Against **WasmGC** — the proposal where you declare struct types and the *browser's*
collector manages them: it is genuinely well supported (Chrome 119, Firefox, Safari 18.2,
and Edge follows Chrome), and it would mean no collector to write and no shadow stack.
That is exactly why it loses. Cat Paws is a language written from scratch; handing the
most interesting part to the engine is the one place that would stop being true.

Writing our own also runs on any engine that runs WebAssembly at all, since it is only
bytes and arithmetic — no version floor, and no iOS 18.2 cutoff.

### Two decisions to make before writing any of it

Both are much harder to retrofit than to design in.

**1. The shadow stack.**

A collector's hardest job is finding the roots — every reference that is still live. Inside
a WebAssembly module you **cannot scan your own locals or the value stack**. They are
invisible to the running code, so a collector has no way to discover what a function is
holding.

So every heap reference has to *also* be written somewhere the collector can walk: a
region of linear memory that the emitted code pushes to on entry and pops on exit. This
changes how every function is emitted, which is why it is a day-one decision.

(This is the problem WasmGC avoids entirely, because the engine knows its own roots.)

**2. The object header.**

Every heap object needs a few bytes in front of it saying what it is: its type, its size,
and a mark bit. Cheap to design now, painful to add once objects exist and every offset in
the emitter assumes there is no header.

### After that, the collector is small

Mark-sweep proper is the easy half: walk the shadow stack marking everything reachable,
then sweep the heap freeing everything unmarked. A few hundred lines.

## Order of work

Nothing above is worth building yet. In order:

1. A feature that can allocate — string concatenation, or lists.
2. An allocator over linear memory, with the object header decided.
3. The shadow stack, in the emitter.
4. The collector.

Building a collector before anything can allocate would mean writing something with
nothing to collect and no way to test it.
