/// Model notes for a 16 GB laptop GPU and 64 GB of system RAM.
/// Weights are not shipped. The user downloads them once, then runs offline.
pub fn recommended() -> &'static str {
    "\
Crab Cage does not download models and does not call a cloud endpoint.
Point --backend local at a loopback OpenAI-compatible server (Ollama or llama.cpp).

Hardware assumed: RTX 5080 Laptop, 16 GB VRAM, 64 GB RAM.

Resident model (structural mutations):
  Qwen2.5-Coder-14B-Instruct, GGUF Q5_K_M.
  Weights are about 10.5 GB. With an 8k context and FP16 KV cache the
  working set is about 12 to 13 GB, so it fits in 16 GB with the desktop
  still up. Apache-2.0.
  If you need a 16k context, use Q4_K_M (about 9 GB of weights) or Q5_K_M
  with a Q8 KV cache.

Faster alternative, when you want headroom for a second process:
  Qwen2.5-Coder-7B-Instruct, Q6_K or Q8_0, roughly 6 to 8 GB. Apache-2.0.
  One resident model is better than swapping. Prefer the 14B if it fits.

Not the default:
  Qwen2.5-Coder-32B-Instruct Q4_K_M is about 18 to 19 GB of weights, which
  does not fit in 16 GB. It runs only with part of the model in system RAM.
  That is usable for a single rewrite of the winner, not for a population.
  Q3_K_M is about 15 GB and is tight once the KV cache is added.

Runtime, after the one-time download:
  ollama pull qwen2.5-coder:14b
  OLLAMA_HOST=127.0.0.1:11434 ollama serve
  crab-cage evolve --backend local --endpoint http://127.0.0.1:11434/v1 --model qwen2.5-coder:14b

  llama-server -m qwen2.5-coder-14b-instruct-q5_k_m.gguf -ngl 99 -c 8192 --host 127.0.0.1 --port 8080
  crab-cage evolve --backend local --endpoint http://127.0.0.1:8080/v1 --model qwen2.5-coder

Context 8192 is enough: the prompt is the plant, the current controller, and the genome schema.
llama.cpp is MIT. Ollama is MIT. Codestral is not recommended; its license is not permissive.
"
}
