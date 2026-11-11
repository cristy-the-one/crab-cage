# Delta kernels

v0.03 is fixed: 2 KB sectors, LOAD / ADD / STORE, sector-local addresses, and the correctness rules in the spec. A kernel chooses the LOAD and ADD commands for one sector. It does not pack bits and it does not grade itself. The host packs the command list, decodes it onto the old image, and accepts the score only when every new real byte matches and the CRC-32 of each new region matches. A kernel that fails that check has no score.

The denominator below is the sum of real bytes in the new images, **1358**. The 0xFF-filled span of the same pairs is 5390; ratios use the real bytes. Lower is better. "Gap" is bytes above the v0.03 optimum.

## Totals

| Kernel | Bytes | Ratio | Gap | Cage run |
|---|---:|---:|---:|---:|
| Add only | 1530 | 1.127 | +905 | 86 ms |
| Strategy A | 731 | 0.538 | +106 | 86 ms |
| Strategy B, sector-local | 860 | 0.633 | +235 | 87 ms |
| Best of A and B, and the spec reference | 626 | 0.461 | +1 | 86 ms |
| Greedy family search | 860 | 0.633 | +235 | 87 ms |
| Optimal parse | 625 | 0.460 | 0 | 87 ms |
| Cross-sector LOAD, not a v0.03 file | 447 | 0.329 | — | host |

The cage run column is the sandboxed run time for all eight pairs at `-C opt-level=2`, from `crab-cage delta eval`. It does not include rustc. Every proven kernel finished far inside the nugget limit (20 s and 128 MB per case).

Add-only is larger than the firmware on this corpus because of the file header, the per-sector STORE, and the ADD headers. That is the right comparison: the delta has to win against shipping the new bytes, and headers can make a tiny change larger than the change itself.

## Per pair

Bytes are the packed v0.03 file. The last column is the cross-sector proposal and is not a legal v0.03 delta.

| Pair | New bytes | Add only | A | B | Best / reference | Family | Optimal | Cross-sector |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| 01-bugfix | 30 | 54 | 29 | 33 | 29 | 33 | 29 | 29 |
| 02-table | 292 | 316 | 74 | 149 | 74 | 149 | 74 | 74 |
| 03-gaps | 64 | 56 | 29 | 33 | 29 | 33 | 29 | 29 |
| 04-insert | 72 | 96 | 84 | 62 | 62 | 62 | 61 | 61 |
| 05-shift-inside | 128 | 152 | 109 | 36 | 36 | 36 | 36 | 36 |
| 06-scattered | 512 | 536 | 119 | 265 | 119 | 265 | 119 | 119 |
| 07-move-earlier | 212 | 248 | 258 | 248 | 248 | 248 | 248 | 70 |
| 08-unaligned | 48 | 72 | 29 | 34 | 29 | 34 | 29 | 29 |
| **Total** | **1358** | **1530** | **731** | **860** | **626** | **860** | **625** | **447** |

`corpus/results.json` is this table. `cargo test` recomputes it from the SREC files.

On seven pairs the optimal parse and the better of A and B are the same size. `04-insert` is the one-byte exception (61 against 62). `07-move-earlier` is the pair v0.03 cannot compress: the moved block is optimal as literals, 248 bytes for 212 bytes of new firmware.

## What won

**Copy the old sector, then patch.** Strategy A emits one LOAD per old real interval and ADDs only the bytes that still differ, merging two ADD runs when the gap between them is shorter than an ADD header. A 0xFF byte that erased RAM already holds is not emitted. That is the winning plan for a bugfix (`01`, one immediate, 29 bytes), a changed table (`02`, 74 bytes for 16 changed entries inside 292), a linker gap (`03`), scattered flips (`06`, 119 bytes for 512), and the unaligned one-byte change (`08`). The one-byte patches use a 2-byte ADD header. The generator tries `add_pat_size_bits` from 1 to 11 and packs with the smallest width that can encode the ADD lengths it actually emitted, so a one-byte patch is not costed at the 4-byte worst-case header.

**A hash match wins when the bytes moved inside the sector.** Strategy B (window 8, stride 4, FNV-1a 32, at most 64 candidates, `MIN_MATCH` 8, then demote a LOAD that costs at least as much as an ADD of the same bytes) is worse than A on the edit pairs, because it never stands back and copies the whole old interval. It is better when a block moved and the identity copy would be wrong: `05-shift-inside` (36 against A's 109) and `04-insert` (62 against A's 84). The inserted `worker()` is 30 identical bytes at a higher offset in the same sector, and B loads them.

**Best of A and B, per sector, per width.** The kernel costs both plans at the width it was given and keeps the cheaper legal one. On this corpus that matches the spec's reference generator on every pair (626 bytes). The spec costs A against a full-image B at the worst-case ADD header and tunes the width once; B here never needed a cross-sector match that survived as a LOAD, so the two procedures land on the same files.

**The optimum allows a LOAD to be overwritten.** A first parse that treated each new byte as covered by exactly one command lost to strategy A on `06-scattered` (the exact-match runs are short, while one LOAD of the whole interval plus a few ADD patches is cheap). The parse that is measured here is a shortest path over the new-real offsets: a LOAD copies a slice of one old interval and later ADDs repair the bytes it got wrong; a 0xFF byte can be skipped; an ADD costs its header plus its span. Given a width, sectors do not depend on each other, so the minimum over the 11 widths is the v0.03 optimum for these kernels. It is 625 bytes, one byte under best-of, all of it on `04-insert`.

**The greedy parameter search did not find that plan.** The grid is 96 points: `MIN_MATCH` in {3, 4, 8}, stride in {1, 4}, window in {4, 8}, and the on/off switches for skipping 0xFF, multi-probe, and merging ADDs across a short gap. Cap stays 64, and demote and 0xFF suppression stay on. The best point is window 8, stride 1, `MIN_MATCH` 3, one probe, no gap merge. It ties sector-local B at 860 bytes and is 235 bytes behind the optimum. Changing the knobs of a left-to-right matcher does not discover "LOAD the old interval, then patch."

## What the oracle rejected

`candidates/01-gap-load.rs` emits a normal ADD cover and then a LOAD from the first byte that is not old-real (or a LOAD that runs off the sector when the sector is full). The device is not required to hold 0xFF in a gap, so the decoder rejects the LOAD even though a filled model would read 0xFF there. The harness drops every width, prints `error: no width covered the new bytes`, and prints no score. The cage then has nothing to sum. The kernel is recorded in `nuggets/delta-kernel/trajectory.md` with no byte count.

Two earlier drafts of strategy B failed rustc before they could run: an untyped `Vec` in the merge pass, then a lookup vector annotated with the wrong element type. The source that scored 860 is the repaired one. The trajectory file lists both failures.

## Format proposals

These are not implemented. The files in this repo are v0.03.

**A LOAD may read any real byte still in flash.** Sectors are stored in ascending order, so while sector *k* is encoded, earlier sectors hold the new image and later sectors still hold the old one. The address field widens to `ceil(log2(span))` and at least 11 bits. For the widths 11 through 15 the LOAD/STORE header stays 5 bytes, the same as v0.03; 16 bits would make it 6. On this corpus the header does not grow. Replaying the same optimal parse against that snapshot costs **447** bytes, ratio 0.329. The whole saving is `07-move-earlier` (70 instead of 248): the 180-byte block sits at `0x08000800` in the old image and at `0x08000000` in the new one, so it is still readable while sector 0 is encoded. A shift to a *higher* address cannot use this. That sector has already been erased and stored. `04-insert` moves `worker()` upward inside one sector, and the proposal is the same size as the v0.03 optimum there (61).

**A copy opcode in the reserved `0b00` slot.** v0.03 uses `0b00` as a fatal reserved opcode, not as an end marker. A short WINDOW/COPY there would help matches that are too small to pay a 5-byte LOAD and too large to want as an ADD. The current optimum already uses ADD for the one-byte patches and LOAD for the long copies, so the copies that would benefit are the medium ones strategy B sometimes demotes. This report does not price that opcode. It would be a different format, and the 625-byte figure would stop being the right baseline.

## Corpus

No production firmware was available. The eight pairs are small enough to compile inside `cargo test`.

| Pair | How it was made |
|---|---|
| `01-bugfix` | `corpus/firmware/bugfix.c`, `link.ld`. `-DSEED=1` against `-DSEED=2`. One Thumb immediate. 30 real bytes, one byte differs. |
| `02-table` | `table.c`. `cal[i] = ((i * 17) ^ 0x5A)`, and every index divisible by 17 is xored with `VER`. `-DVER=0` against `-DVER=0xFF` (16 entries). The instructions are the same. |
| `03-gaps` | `gaps.c`, `gaps.ld`. `.rodata` is a separate section at `0x08000800`, so the SREC has a 2016-byte hole. `SEED` changes one immediate. The table matches, so the second sector is not stored. |
| `04-insert` | `insert.c`. `-DEXTRA=1` inserts `extra()` in front of `worker()`. `worker()` is 30 register-only bytes and moves from `0x08000016` to `0x0800002A`. |
| `05-shift-inside` | `crab-cage delta gen-synthetic`. 128 bytes. A 40-byte ramp and 40 bytes of `0x80` swap places. |
| `06-scattered` | Same generator. 512 bytes, every 17th byte xored with `0xFF`. |
| `07-move-earlier` | Same generator. A 180-byte block moves from `0x08000800` to `0x08000000`. This is the case a cross-sector LOAD can see and v0.03 cannot. |
| `08-unaligned` | Same generator. 48 bytes at `0x08000040`. One byte changes from `0x10` to `0x11`. The sector base in the delta is `0x08000000`. |

`corpus/firmware/build.sh` is the compile. It uses `arm-none-eabi-gcc -mcpu=cortex-m4 -mthumb -Os -ffreestanding -nostdlib -nostartfiles` and does not link newlib or libgcc. The toolchain that produced the committed SREC files was `arm-none-eabi-gcc (15:13.2.rel1-2) 13.2.1 20231009`. CI does not need it. Each pair directory has a README with that build's section headers and disassembly.

To add a real pair, create `corpus/pairs/<name>/old.srec` and `new.srec`, and run:

```sh
crab-cage delta sync-cases
crab-cage delta measure
```

`sync-cases` rewrites `nuggets/delta-kernel/cases`. Names are taken in directory order.

## Running it again

Kernels are the marked region of `nuggets/delta-kernel/template.rs`. The sources are written without closures, iterator adapters, `vec!`, or `?`, so a later lust can check them. This run did not have a lust binary, so the proofs are rustc, which is the path Crab Cage already uses when lust is absent.

```sh
crab-cage delta eval nuggets/delta-kernel/candidates/06-optimal.rs
crab-cage run nuggets/delta-kernel --candidates nuggets/delta-kernel/candidates
crab-cage run nuggets/delta-kernel --backend local --model qwen2.5-coder:14b
```

`delta eval` reprints the decoder's verdict as JSON. `run --candidates` feeds the folder through the cage's existing proposer. `run --backend local` asks a loopback model (default `http://127.0.0.1:11434/v1`) for a new region; the contract comment above `// CRAB:BEGIN` is part of the prompt. The attempt log is `nuggets/delta-kernel/trajectory.md`.
