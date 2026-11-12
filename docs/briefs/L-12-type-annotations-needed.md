# Brief L-12: say "type annotations needed" for a field of an unknown type

For: Grok. Repo: `lust-lang`, branch from `next`. One pull request.
Written 2026-11-12 against lust-lang `10148c4d`. Line numbers are from that commit; if they have moved, search for the quoted text.

## The task in one sentence

When a program reads a field (`x.name` or `x.0`) of a value whose type lust has not inferred yet, report **L0610 "type annotations needed"**, not **L0103 "no field"**.

This changes a message only. No program that lust accepts today is affected, and no program it rejects today becomes accepted.

## Why

In the model ladder of 2026-11-12 (crab-cage, `docs/WORK-PACKAGES.md`, "The model ladder on a GPU machine"), `qwen2.5-coder:14b` lost all three runs of the `merge-ranges` nugget to this one message. It wrote:

```rust
let mut merged = Vec::new();
if let Some(last) = merged.last_mut() {
    last.1 = 2;          // lust: L0103 no field `1` on type `&mut _`
}
merged.push((1usize, 2usize));
```

rustc rejects this program too, so lust is right to reject it. But rustc says what to do:

```
error[E0282]: type annotations needed for `Vec<_>`
 --> a.rs:2:9
```

lust says `no field 1 on type &mut _` with the fix "Check the field index". The model then rewrote the field access 32 times and never touched the `let`. 96 of the 104 diagnostics in the whole experiment were this one.

## What lust does today (checked with lust 0.3.0, build `1ac5e64c`)

| Program | lust today | rustc |
|---|---|---|
| A. tuple field, type unknown: `last.1` as above | `L0103 no field 1 on type &mut _` | `E0282 type annotations needed for Vec<_>` |
| B. named field, type unknown: `last.x` where `v = Vec::new()` and `v.push(P { x: 1 })` comes later | `L0103 no field x on type &_` | `E0282 type annotations needed for Vec<_>` |
| C. method, type unknown: `last.len()` in the same shape | `L0610 type annotations needed: the receiver type must be known here` | `E0282` |
| D. really no such field: `let t = (1, 2); t.5` | `L0103 no field 5 on type ({integer}, {integer})` | `E0609 no field 5` |

C and D are already right. **A and B are the task.** D must not change.

The four programs are at the end of this brief.

## Read first

- `AGENTS.md` (the whole file). A diagnostic's code and text are a stated interface, so the pull request must say that two messages changed.
- `scripts/difftest.sh`, the header comment: how `tests/reject` cases are judged.
- `scripts/check-codes.sh`: what it checks about L-codes.

## Where

`src/check/mod.rs`, two places. Both report L0103 after auto-dereferencing the receiver into a local called `inner`:

1. Named field, near line 9769:
   ```rust
   if !matches!(self.kind(inner), TyKind::Error) && !self.tainted_lookup(bt) {
       let tn = self.tn(bt);
       self.err("L0103", span, format!("no field `{}` on type `{tn}`", self.name(*name)), "Check the field name.");
   }
   ```
2. Tuple field, near line 10627:
   ```rust
   if !matches!(self.kind(inner), TyKind::Error) {
       let tn = self.tn(bt);
       self.err("L0103", span, format!("no field `{index}` on type `{tn}`"), "Check the field index.");
   }
   ```

Do not touch the other L0103 sites (`offset_of!`, struct literals, enum variants, patterns).

## What to change

In both places, before reporting L0103, test whether `inner` (the receiver after auto-deref, with inference variables resolved as far as they are known) is still an **unresolved inference variable**. If it is, report instead:

- code: `L0610`
- message: ``type annotations needed: the type of this value must be known to read its field``
- fix: ``Annotate the binding it comes from, for example `let mut v: Vec<(usize, usize)> = Vec::new();`.``

Otherwise keep the L0103 message exactly as it is.

How to tell "still an inference variable": find how case C does it. Search `src/check` for `the receiver type must be known here`; the test that guards that message is the one to reuse. Use the same helper or the same `TyKind` match. Do not write a new way to ask the question.

Only a general type variable counts. `{integer}` and `{float}` (case D) are numeric variables and must keep L0103.

## Do not

- Do not make these programs compile. rustc rejects them.
- Do not change the span: keep it on the field access. (Pointing at the `let`, as rustc does, would be better, but it needs the origin of the variable; that is a later step.)
- Do not change the text of L0103 or of any other L0610 message.
- Do not add a new L-code.

## Tests

Add two cases to `tests/reject/`, named like the files already there, each with the expected code the way its neighbours state it:

- `field_of_uninferred_tuple.rs`: program A. Expect L0610.
- `field_of_uninferred_struct.rs`: program B. Expect L0610.

Check that a case for D exists (search `tests/reject` and `tests/capabilities` for `no field`). If none does, add `field_index_out_of_range.rs` with program D, expecting L0103, so the old message is pinned.

If `lust explain L0610` or the README lists the L0610 messages, add the new one there.

## Done when

- `lust check` on A and B prints `error[L0610]: type annotations needed: the type of this value must be known to read its field`.
- `lust check` on C and D prints what the table above shows.
- `scripts/difftest.sh` passes, `scripts/check-codes.sh` passes, and `cargo fmt --check` reports nothing new in the lines you changed.
- The pull request description has a section "Interface change" that names the two messages.

## If you get stuck

- If `inner` is not an inference variable at that point for program A (for example it is already `ERROR`, or the variable is wrapped in something), stop and say what `self.kind(inner)` is. Do not widen the test to "any type that prints with `_`".
- If reusing case C's test changes the outcome of any existing `tests/reject` or `tests/pass` case, stop and report which.

## The four programs

A:
```rust
fn main() {
    let mut merged = Vec::new();
    if let Some(last) = merged.last_mut() {
        last.1 = 2;
    }
    merged.push((1usize, 2usize));
    println!("{}", merged.len());
}
```

B:
```rust
struct P { x: i32 }
fn main() {
    let mut v = Vec::new();
    if let Some(last) = v.last() {
        println!("{}", last.x);
    }
    v.push(P { x: 1 });
}
```

C:
```rust
fn main() {
    let mut v = Vec::new();
    if let Some(last) = v.last() {
        println!("{}", last.len());
    }
    v.push(String::new());
}
```

D:
```rust
fn main() {
    let t = (1, 2);
    println!("{}", t.5);
}
```
