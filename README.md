# Crab Cage

Offline search for process controllers. A local model proposes a Rust control law, a genetic algorithm tunes it, and the result is a single `.rs` file you can compile with `rustc`. When [lust](https://github.com/cristy-the-one/lust-lang) is installed, candidates are type-checked and built in lust's sandbox.

The design is in [docs/DESIGN.md](docs/DESIGN.md). This file is how to run the demo on the laptop it was sized for: an RTX 5080 Laptop (16 GB VRAM) and 64 GB of RAM, with no cloud calls.

Crab Cage is MIT OR Apache-2.0. It does not phone home. The only network client is the optional model backend, and it refuses every host that is not `127.0.0.1`, `localhost`, or `::1`.

## What you need

- Rust (the 1.74 toolchain or anything newer). Linux and Windows are the tested hosts. macOS is the same Unix path as Linux for the address-space cap.
- Nothing else for the demo. The mock backend is deterministic and does not load a model.
- Optional: a lust 0.3.0 binary on `PATH`. `--max-memory` is in that release, on lust-lang main. A 0.2.0 binary still runs; Crab Cage sees that the flag is missing and uses the host memory cap instead.
- Optional, for `--backend local`: Ollama or llama.cpp, and a GGUF that fits in 16 GB. See below. Download the weights once, then disconnect.

## Build

The first build fetches crates from crates.io. After that, `--offline` works.

```sh
cargo build --release
cargo test --locked
```

The binary is `target/release/crab-cage` (`crab-cage.exe` on Windows).

The tests use a fake lust, so they cannot see how a real lust behaves. `tests/real_lust.rs` runs the default evolution and the probe against a real binary. It does nothing unless `CRAB_LUST_BIN` is set. Set `LUST_NO_DAEMON=1` too, so lust leaves no background daemon: `CRAB_LUST_BIN=/path/to/lust cargo test --test real_lust` (PowerShell: `$env:CRAB_LUST_BIN = "C:\path\to\lust.exe"; $env:LUST_NO_DAEMON = "1"; cargo test --test real_lust`).

## Run the demo

```sh
cargo run --release -- evolve
```

That is the jacketed-tank simulator: six measured inputs, a heater, a coolant valve, and a one-step temperature prediction. The search starts from weak proportional controllers. The mock model turns on integral action, then feedforward, then a predictor. The run prints one line per generation and the best fitness, and writes:

```text
crab-out/best_controller.rs
crab-out/report.md
crab-out/report.json
```

`best_controller.rs` has no dependencies. Check it yourself:

```sh
rustc --edition 2021 -O -o crab-out/controller crab-out/best_controller.rs
```

Useful flags:

```sh
crab-cage evolve --seed 2 --population 16 --generations 12 --out-dir crab-out
crab-cage evolve --eval native --no-verify          # search only, no rustc/lust replay
crab-cage evolve --lust-bin /path/to/lust           # a lust that is not on PATH
crab-cage models                                    # the 16 GB model notes
crab-cage lust-probe                                # version, path, and whether --max-memory and --time-limit exist
```

Copy `crab-cage.toml.example` to `crab-cage.toml` to set the lust path and the model endpoint without flags. A flag on the command line wins over the file, and the file wins over the built-in default.

## Lust

Lust is an external binary. This repo does not contain it.

```sh
# once 0.3.0 is installed, either on PATH or pointed at:
crab-cage lust-probe --lust-bin "$HOME/lust/bin/lust"
crab-cage evolve --lust-bin "$HOME/lust/bin/lust"
```

With a 0.3.0 binary the winner is checked with `lust check --json --sandbox` and built with `lust build --sandbox --max-memory`. When that binary also advertises `--time-limit`, the same build carries the run timeout and a guest that runs past it exits 101 with `L0533`. The host still enforces the wall clock, because a straight-line body is not probed and an older lust has no guest deadline, and it still sets an address-space cap (Unix) or a job-object commit cap (Windows), because the guest cap does not cover the stack or a later touch of reserved pages.

`--max-memory` defaults to `128M`. The host backstop defaults to `1G`. Both accept lust's syntax: `64K`, `128M`, `1G`, or a bare byte count.

### The warm daemon

`crab-cage eval` runs lust with its daemon off by default: each call starts lust cold. `--lust-daemon` turns the daemon on. It needs `--lust-bin` and `--work-dir`:

```sh
crab-cage eval nuggets/sum-squares cand.rs --lust-bin "$HOME/lust/bin/lust"   --work-dir work --lust-daemon --json
```

lust keeps one warm session per source path, so with the daemon on every candidate is compiled as `work/slot.rs` (the candidate is still saved as `work/<hash>.rs`). lust is started in the work directory, and its socket file and `.lust-cache` stay there. The first call starts the daemon and is no faster. Later calls that name the same work directory are: on the Windows laptop this was measured on, `lust check` went from about 110 ms to about 27 ms and `lust build` from about 400 ms to about 45 ms.

The daemon keeps running after `crab-cage` exits. It stops on its own after 30 minutes idle, or at once when `work/.lust-daemon-*.sock` is deleted.

Use a lust built from lust-lang `next` at `f2dc9a8b` or later. An older lust lets its daemon keep the caller's output pipes; Crab Cage no longer hangs on that, but each lust call then waits an extra half second.

## A local model

The mock backend is the default and is what CI runs. A real model is optional.

Recommended resident model: **Qwen2.5-Coder-14B-Instruct**, GGUF **Q5_K_M** (about 10.5 GB of weights, Apache-2.0). With an 8k context it sits around 12–13 GB, which fits in 16 GB. The 7B Q6/Q8 build is the one to use if something else needs the card. The 32B Q4 build does not fit in 16 GB; it can run with part of the weights in system RAM for a single rewrite, not for a population. `crab-cage models` prints the same notes.

Ollama, after `ollama pull qwen2.5-coder:14b`:

```sh
OLLAMA_HOST=127.0.0.1:11434 ollama serve
crab-cage evolve --backend local \
  --endpoint http://127.0.0.1:11434/v1 \
  --model qwen2.5-coder:14b
```

llama.cpp:

```sh
llama-server -m qwen2.5-coder-14b-instruct-q5_k_m.gguf -ngl 99 -c 8192 \
  --host 127.0.0.1 --port 8080
crab-cage evolve --backend local \
  --endpoint http://127.0.0.1:8080/v1 \
  --model qwen2.5-coder
```

`https://…` and any non-loopback host are refused. There is no API key.

## Tests

```sh
cargo test --locked
```

The suite checks that:

- the search lowers fitness on the tank for several seeds, and the same seed repeats bit-for-bit,
- the emitted controller compiles with `rustc` and matches the in-process genome,
- a candidate that ignores the wall clock is killed,
- a large allocation dies under a tight host cap and a small one survives a loose cap,
- the lust runner passes `--sandbox`, `--max-memory` and `--time-limit`, and classifies lust's `L0532` memory stop and `L0533` time stop.

CI runs that suite on Linux and Windows, plus `cargo fmt --check` and `cargo deny`. `deny.toml` allows MIT, Apache-2.0, BSD, ISC, Zlib, and the Unicode licenses used by the parser crates. GPL and LGPL are not allowed. Dependencies come from crates.io only.

## Layout

```text
src/            the crate: plant, genome, GA, emitter, lust runner, supervisor
tests/          replay, host limits, fake lust, CLI
docs/DESIGN.md  the design, the fitness, the model notes, the lust gaps
deny.toml       license and source audit
```
