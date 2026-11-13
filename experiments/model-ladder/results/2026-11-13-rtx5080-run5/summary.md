# Model ladder

- Date: 2026-11-13 14:58
- CPU: Intel(R) Core(TM) Ultra 9 275HX
- GPU: Intel(R) Graphics; NVIDIA GeForce RTX 5080 Laptop GPU (nvidia-smi: NVIDIA GeForce RTX 5080 Laptop GPU, 16303 MiB)
- RAM: 63 GB
- Ollama: 0.40.16
- crab-cage: cc2b102; rustc 1.96.0 (ac68faa20 2026-05-25)
- lust: lust 0.3.1 (build 16d67467)
- Runs per model and nugget: 3; budget: 20 candidates; one job

## Models as loaded

| Model | Size | On the GPU | Context | First load |
|---|---:|---:|---:|---:|
| qwen2.5-coder:7b | 5.6 GB | 5.6 GB | 16384 | 2.6 s |
| qwen2.5-coder:14b | 11.9 GB | 11.9 GB | 16384 | 5.2 s |

## Solved

Each cell: runs solved of runs made; then, over the solved runs, the median number of model candidates it took and the median wall time.

| Model | run-lengths | merge-ranges | normalize-path |
|---|---|---|---|
| qwen2.5-coder:7b | 2/3; 3.5 cand.; 10.7 s | 3/3; 1 cand.; 4.6 s | 0/3 |
| qwen2.5-coder:14b | 3/3; 1 cand.; 4.3 s | 3/3; 1 cand.; 6.1 s | 1/3; 5 cand.; 33.9 s |

## Unsolved runs

| Model | Nugget | Rep | Seed score | Best proven | Candidates | Wall | How it ended |
|---|---|---:|---:|---:|---:|---:|---|
| qwen2.5-coder:7b | run-lengths | 1 | 7 | 2 | 3 | 20.9 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |
| qwen2.5-coder:7b | normalize-path | 1 | 8 | 8 | 19 | 31.6 s |  |
| qwen2.5-coder:7b | normalize-path | 2 | 8 | 8 | 19 | 24.9 s |  |
| qwen2.5-coder:7b | normalize-path | 3 | 8 | 8 | 19 | 22.5 s |  |
| qwen2.5-coder:14b | normalize-path | 1 | 8 | 1 | 19 | 167.7 s |  |
| qwen2.5-coder:14b | normalize-path | 2 | 8 | 1 | 19 | 94.8 s |  |

## Where candidates were rejected

| Model | Candidates | prefilter | lust check | build | run | oracle | diverged | Seconds per candidate |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| qwen2.5-coder:7b | 71 | 0 | 30 | 0 | 0 | 0 | 0 | 1.9 |
| qwen2.5-coder:14b | 58 | 0 | 11 | 0 | 0 | 0 | 0 | 6.4 |

## lust error codes, all models

| Code | Candidates |
|---|---:|
| L0113 | 32 |
| L0610 | 28 |
| L0110 | 5 |
| L0111 | 2 |
| L0308 | 2 |
| L0001 | 1 |
| L0104 | 1 |

Every run keeps its journal, its candidates and any `divergences/` under this directory.
