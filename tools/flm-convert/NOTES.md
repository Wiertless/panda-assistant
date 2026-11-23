# FLM .q4nx converter: findings so far (2026-11-23)

Goal: turn Hugging Face weights of an architecture FastFlowLM already supports
(Gemma 4 E2B first: QAT weights, later fine-tunes) into FLM's `model.q4nx`, so
FLM runs them on the NPU with its own (unmodified, closed) kernels.

License: FLM's NPU binaries may be used/redistributed unmodified
(LICENSE_BINARY.txt); runtime is MIT; nothing restricts making weight files.
Gemma 4 weights (google/gemma-4-E2B-it, google/gemma-4-E2B-it-qat-q4_0-unquantized)
and FLM's conversion (FastFlowLM/Gemma4-E2B-IT-NPU2) are Apache-2.0, not gated.

## Container
`model.q4nx` is safetensors-style: 8-byte little-endian header length, JSON
header {name: {dtype, shape, data_offsets}}, data. 709 tensors: BF16 459, I8 248,
F32 2. Names follow HF without the `language_model.` part
(`model.layers.0.self_attn.q_proj.weight`). `q4nx.py` reads it.

## Quantized linear weights (I8 dtype, really packed Q4_1)
Example `model.layers.0.self_attn.q_proj.weight` I8 [384, 5120]; HF tensor is
`model.language_model.layers.0.self_attn.q_proj.weight` BF16 [2048, 1536]
(3,145,728 weights). Each of the 384 rows is one block of 8192 weights:
- bytes 0..512: 256 BF16 scales (all > 0)
- bytes 512..1024: 256 BF16 minimums (all <= 0)
- bytes 1024..5120: 4096 bytes = 8192 4-bit values
So 256 groups of 32; w = scale * q + min, q in 0..15 (Q4_1).
MLP: up/gate/down [1152, 5120] = 6144*1536 weights. k/v [48, 5120] = 256*1536.

## Not solved yet: which weights go into which block/group, and nibble order
Tried against google/gemma-4-E2B-it (non-QAT): no group of FLM's block 0 matches
32 contiguous HF weights along rows or columns, by (min, (max-min)/15)
fingerprint or by decoded nibbles (overlap 8/32 = chance). Next: the same against
google/gemma-4-E2B-it-qat-q4_0-unquantized (FLM may have converted the QAT
weights), then non-contiguous groupings (strided, tiles of 32x32, head-interleaved
rows), and the nibble order once a group matches.

## Other tensors (later)
- embed_tokens I8 [262144, 1536] with embed_tokens.weight.scale F32 [262144, 48]:
  int8 per weight, scale per 32 (Q8_0-like).
- per_layer_token_embd I8 [262144, 8960] + scale F32 [262144, 280].
- BF16 tensors are re-tiled, often in two layouts: `inp_gate.weight` [8, 1536, 32]
  and `inp_gate.weight_prefill` [4, 6, 16384]; `per_layer_model_proj.weight_layerN`
  and `.weight_prefill`. Norms are plain [1536].

## Check
A converter is right when converting google's source weights reproduces
FLM's published `model.q4nx` byte for byte (then any fine-tune of the same
architecture converts the same way). That check can run on GitHub Actions
(no NPU needed); only running the result needs the laptop's NPU.
