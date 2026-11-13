# Model ladder

- Date: 2026-11-13 15:08
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
| gpt-oss:20b | 12.8 GB | 12.8 GB | 16384 | 11.1 s |

## Solved

Each cell: runs solved of runs made; then, over the solved runs, the median number of model candidates it took and the median wall time.

| Model | run-lengths | merge-ranges | normalize-path |
|---|---|---|---|
| gpt-oss:20b | 3/3; 1 cand.; 5.3 s | 3/3; 1 cand.; 11.8 s | 3/3; 1 cand.; 15.5 s |

## Unsolved runs

| Model | Nugget | Rep | Seed score | Best proven | Candidates | Wall | How it ended |
|---|---|---:|---:|---:|---:|---:|---|

## Where candidates were rejected

| Model | Candidates | prefilter | lust check | build | run | oracle | diverged | Seconds per candidate |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| gpt-oss:20b | 10 | 0 | 0 | 0 | 0 | 0 | 0 | 17.3 |

## lust error codes, all models

None.

Every run keeps its journal, its candidates and any `divergences/` under this directory.
