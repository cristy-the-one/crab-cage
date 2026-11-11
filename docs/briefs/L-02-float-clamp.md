# Brief L-02: `f64::clamp` and `f32::clamp` in lust

For: Grok. Repo: `lust-lang`, branch from `next`. One pull request.
Written 2026-11-11 against lust-lang `31e1d4da`. Line numbers are from that commit; if they have moved, search for the quoted text.

## The task in one sentence

Make lust accept `x.clamp(LO, HI)` on a float **when `LO` and `HI` are float literals with `LO <= HI`**, and reject every other float `clamp` with a clear message.

## Why this shape

Today `3.5f64.clamp(0.0, 1.0)` fails with `L0104 no method clamp on type f64`. Crab Cage's emitted controller uses it, and so will code that models write.

In rustc, `clamp` panics when `min > max` or a bound is NaN, and the panic text contains both bounds and a location inside core. Matching that byte for byte needs runtime work that is out of scope here. lust's rule: *what it cannot match, it rejects*. With literal bounds that the checker has verified, the panic cannot happen, so lust's output is equal to rustc's by construction. Non-literal bounds stay rejected (stage 2, not yours).

## Read first

- `AGENTS.md` (the whole file; it is short). Rule zero and rule 2 govern this change.
- `scripts/difftest.sh`, the header comment: how `tests/pass` and `tests/reject` cases are judged.

## What rustc does (the behaviour to match)

```rust
// core, simplified
if self < min { self = min; }
if self > max { self = max; }
self
```

So: NaN input returns NaN. `(-0.0).clamp(0.0, 1.0)` returns `-0.0` (because `-0.0 < 0.0` is false). In C: `a < lo ? lo : (a > hi ? hi : a)`.

## Changes, file by file

Use `copysign` as the pattern to copy: it is a float method with one argument. Search for `FloatCopysign` and `copysign` to see every site; there are nine. `clamp` needs the same sites, with two arguments, like `FloatMulAdd`.

1. **`src/intern.rs`** — the list of known names (near line 398, `copysign,`). Add `clamp,` the same way. Check first that it is not already there under another section.

2. **`src/check/mod.rs`, `enum Builtin`** (near line 258, `FloatCopysign,`). Add `FloatClamp,`.

3. **`src/check/mod.rs`, the big `Builtin::…` list** (near line 1144, contains `Builtin::FloatCopysign`). Add `Builtin::FloatClamp`. Keep whatever ordering the list uses.

4. **`src/check/mod.rs`, `const FLOAT_METHODS`** (line 1132). Add `clamp` after `max`. This string is shown to users in the L0104 hint.

5. **`src/check/mod.rs`, `fn float_method`** (line 18362). After the `copysign` arm add:
   ```rust
   if n == k.clamp {
       return Some((B::FloatClamp, Autoref::None, vec![t, t], t));
   }
   ```

6. **The literal-bounds rule.** `float_method` does not see the argument expressions; its caller does. In `fn method_call` (line 16751) the arm `(None, TyKind::Float(f)) => self.float_method(name, base, f)` is near line 17646, and `args` is in scope. After the builtin is resolved to `B::FloatClamp`, check the two argument expressions:
   - each must be a float literal, or a unary minus applied to a float literal (search `ExprKind::Lit` and `ExprKind::Unary` in this file to see how literals are matched);
   - parse both values; require `lo <= hi` (this also excludes NaN, which a literal cannot be anyway).

   If the check fails, report and return `ERROR`:
   ```rust
   self.err(
       "L9001",
       name_span,
       "`clamp` on a float with bounds that are not float literals".to_string(),
       "Write the bounds as literals with min <= max, such as `x.clamp(0.0, 1.0)`, or use `x.max(lo).min(hi)`.",
   );
   ```
   If `lo > hi` with literals, use the same code with the message `` `clamp` on a float with min > max (rustc panics here) ``.

   There is a second caller of `float_method` near line 4416 (path calls such as `f64::clamp(x, 0.0, 1.0)`). If the arguments are not easy to reach there, make that path return `None` for `FloatClamp` so the path form stays rejected as it is today, and say so in the pull request. Do not guess.

7. **`src/check/constfn.rs`** (near line 219). Do **not** add `FloatClamp`. `clamp` in a `const` context stays rejected. A reject is fine.

8. **`src/cgen.rs`, the `B::Float…` or-pattern** (near line 7655, `| B::FloatCopysign`). Add `| B::FloatClamp`.

9. **`src/cgen.rs`, `fn float_method_c`** (line 9985). Add:
   ```rust
   B::FloatClamp => format!("lust_{w}_clamp({a}, {}, {})", vals[1], vals[2]),
   ```

10. **Runtime.** Copy how `lust_f64_min` is done, in all three places:
    - `src/runtime/float.c` (near line 1538): `double lust_f64_clamp(double a, double lo, double hi) { return a < lo ? lo : (a > hi ? hi : a); }` and the `float` version `lust_f32_clamp`.
    - `src/runtime/float.h` (near line 171): the two prototypes.
    - `src/runtime/inl.h` (near line 541): the two `static inline` versions and the two `#define`s, same style as `lust_inl_f64_min_`.

11. **`src/capabilities.rs`** — two places (near lines 184 and 886). Add `Builtin::FloatClamp,` to the list and
    `B::FloatClamp => (&["floats"], vec![k.clamp], Call::Method),` to the match.

After step 11, build. The compiler will name any `match` on `Builtin` that is now missing an arm; add `FloatClamp` next to `FloatCopysign` in each.

## Tests to add

**`tests/pass/float_clamp_literal_bounds.rs`** (rustc and lust must print the same):

```rust
// f64::clamp and f32::clamp with literal bounds: below, inside, above,
// NaN (stays NaN), infinities, and the sign of zero.
fn main() {
    let xs = [-5.0f64, -0.0, 0.0, 0.25, 1.0, 7.5, f64::NAN, f64::INFINITY, f64::NEG_INFINITY];
    for &x in xs.iter() {
        println!("{:?} -> {:?} {:?} {:?}", x, x.clamp(0.0, 1.0), x.clamp(-120.0, 120.0), x.clamp(-1.5, -0.5));
    }
    let fs = [-5.0f32, -0.0, 0.5, 9.0, f32::NAN];
    for &x in fs.iter() {
        println!("f32 {:?} -> {:?} {:?}", x, x.clamp(0.0, 1.0), x.clamp(2.5, 2.5));
    }
    let mut acc = 0.0f64;
    for i in 0..10 {
        acc = (acc + i as f64 * 0.7).clamp(-3.0, 3.0);
    }
    println!("{:?}", acc);
}
```

**`tests/reject/float_clamp_variable_bounds.rs`** (valid Rust that lust rejects on purpose):

```rust
// expect: L9001
// Valid Rust. With bounds that are not literals rustc may panic with a
// message lust does not reproduce yet, so lust rejects the call.
fn main() {
    let lo = 0.0f64;
    let hi = 1.0f64;
    println!("{:?}", 3.5f64.clamp(lo, hi));
}
```

**`tests/reject/float_clamp_min_above_max.rs`**: the same shape with `3.5f64.clamp(2.0, 1.0)` and `// expect: L9001`. rustc compiles this program (it panics only when run), so it is a valid reject case.

Integer `clamp` and `Ord::clamp` already work. Run `tests/pass` cases whose name contains `clamp` to confirm you did not change them.

## Build and check

```sh
scripts/build.sh
LUST_GATE_FILTER=clamp bash scripts/ci.sh
LUST_GATE_FILTER=float bash scripts/ci.sh
scripts/test-capabilities.py
```

On Windows run these from Git Bash, after `scripts/build-tcc.ps1` has been run once and with `LUST_TCC_HOME` set (README, "Windows").

The oracle is the `rustc` on `PATH`. lust is defined against rustc 1.99. If the installed rustc is a different version, say which one you used.

## Done when

- `lust check` accepts `fn main() { let x: f64 = 3.5; println!("{}", x.clamp(0.0, 1.0)); }` and `lust run` prints `1`.
- Both new test files pass under the filtered gate, and no existing `float` or `clamp` case changed result.
- `lust capabilities --summary` no longer suggests that float `clamp` is missing, and the L0104 hint lists `clamp`.

## Not in this task

- Non-literal bounds and the panic path (needs a runtime helper with a core source location via `std_loc`, and float `Debug` formatting inside the panic message).
- `clamp` in `const` items.
- Anything in `library/`.

## Report back

The commit hash; the list of files changed; the output of the two filtered gate runs; which rustc version was the oracle; and whether the path form `f64::clamp(x, a, b)` is accepted or still rejected.
