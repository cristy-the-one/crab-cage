# Model ladder

- Date: 2026-11-12 09:00
- CPU: Intel(R) Core(TM) Ultra 9 275HX
- GPU: Intel(R) Graphics; NVIDIA GeForce RTX 5080 Laptop GPU (nvidia-smi: NVIDIA GeForce RTX 5080 Laptop GPU, 16303 MiB)
- RAM: 63 GB
- Ollama: 0.40.16
- crab-cage: c28e6d5; rustc 1.96.0 (ac68faa20 2026-05-25)
- lust: lust 0.3.0 (build 8eb1e88c)
- Runs per model and nugget: 3; budget: 20 candidates; one job
- Skipped: qwen2.5-coder:14b (ollama pull failed)

## Models as loaded

| Model | Size | On the GPU | Context | First load |
|---|---:|---:|---:|---:|
| qwen2.5-coder:7b | 5.6 GB | 5.6 GB | 16384 | 2.3 s |
| gpt-oss:20b | 12.8 GB | 12.8 GB | 16384 | 38.5 s |

## Solved

Each cell: runs solved of runs made; then, over the solved runs, the median number of model candidates it took and the median wall time.

| Model | sum-squares | run-lengths | merge-ranges | normalize-path |
|---|---|---|---|---|
| qwen2.5-coder:7b | 3/3; 1 cand.; 1.9 s | 2/3; 1 cand.; 3.05 s | 3/3; 1 cand.; 4.3 s | 0/3 |
| gpt-oss:20b | 3/3; 1 cand.; 3.7 s | 3/3; 1 cand.; 8 s | 3/3; 1 cand.; 20.3 s | 3/3; 2 cand.; 38.7 s |

## Unsolved runs

| Model | Nugget | Rep | Seed score | Best proven | Candidates | Wall | How it ended |
|---|---|---:|---:|---:|---:|---:|---|
| qwen2.5-coder:7b | run-lengths | 1 | 7 | 2 | 1 | 8.7 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |
| qwen2.5-coder:7b | normalize-path | 1 | 8 | 6 | 4 | 13.4 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |
| qwen2.5-coder:7b | normalize-path | 2 | 8 | 8 | 1 | 7.9 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |
| qwen2.5-coder:7b | normalize-path | 3 | 8 | 6 | 2 | 11.8 s | local proposer finished: 3 proposal errors in a row: the local model repeated a region it already tried |

## Where candidates were rejected

| Model | Candidates | prefilter | lust check | build | run | oracle | diverged | Seconds per candidate |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| qwen2.5-coder:7b | 16 | 0 | 3 | 0 | 0 | 0 | 0 | 4.2 |
| gpt-oss:20b | 18 | 0 | 2 | 0 | 0 | 0 | 0 | 17.9 |

## lust error codes, all models

| Code | Candidates |
|---|---:|
| L0113 | 4 |
| L0610 | 1 |
| L0110 | 1 |

Every run keeps its journal, its candidates and any `divergences/` under this directory.
