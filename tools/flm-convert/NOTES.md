# FLM .q4nx converter: findings (2026-11-24)

Goal: turn Hugging Face weights of an architecture FastFlowLM already supports
(Gemma 4 E2B first: QAT weights, later fine-tunes) into FLM's `model.q4nx`, so
FLM runs them on the NPU with its own (unmodified, closed) kernels.

License: FLM's NPU binaries may be used/redistributed unmodified
(LICENSE_BINARY.txt); runtime is MIT; nothing restricts making weight files.
Gemma 4 weights (google/gemma-4-E2B-it, google/gemma-4-E2B-it-qat-q4_0-unquantized)
and FLM's conversion (FastFlowLM/Gemma4-E2B-IT-NPU2) are Apache-2.0, not gated.

Files: `convert.py` (HF dir -> model.q4nx, numpy only), `verify.py` (convert,
then compare with FLM's file tensor by tensor), `q4nx.py` (minimal reader).

## Status in one paragraph
Every layout is solved and verified: the header (byte-identical), tensor
order, all 459 BF16 tensors (byte-identical, including the re-tiled ones), the
F32/I8 embedding format, and the Q4 tile/group/nibble layout (re-packing FLM's
own scales/mins/nibbles with our packer gives FLM's bytes back exactly, for
every Q4 tensor). What is *not* reproduced is FLM's choice of quantization
parameters: FLM's Q4 scales/mins were picked by an activation-aware
(calibrated) quantizer, so Q4 tensors and the two int8 embeddings are not
byte-identical. Ours are plain round-to-grid of the same weights with lower
plain-MSE error; see "Q4 values" below and the verify output.

## Source weights
FLM converted **google/gemma-4-E2B-it** (non-QAT). All norms, layer scalars and
BF16 matrices match it bit for bit; against the QAT-unquantized weights they
differ (e.g. `layers.0.input_layernorm` max diff 0.5, `layer_scalar` 0.0209 vs
0.0178), and FLM's decoded Q4 weights are 1.42x the QAT ones in scale.

## Container
safetensors-style: 8-byte LE header length, compact JSON
(`json.dumps(h, separators=(",",":"))`, no `__metadata__`), padded with spaces
so 8 + header length is a multiple of 8, then data with no gaps. Tensor order
(both in the JSON and in the data): by dtype F32, then BF16, then I8; inside a
dtype, by name in plain string order (`layers.0, layers.1, layers.10, ...`).
709 tensors: BF16 459, I8 248, F32 2. `build_header()` reproduces the 81056
header bytes exactly.

## Name / transform map (HF prefix `model.language_model.` -> FLM)
| FLM | from HF | transform |
|---|---|---|
| `model.layers.N.input_layernorm / post_attention_layernorm / pre_feedforward_layernorm / post_feedforward_layernorm .weight`, `self_attn.q_norm/k_norm` | same name | copy BF16 |
| `model.layers.N.post_layernorm.weight` | `layers.N.post_per_layer_input_norm.weight` | copy |
| `model.layers.N.layer_output_scale.weight` | `layers.N.layer_scalar` | copy |
| `model.norm.weight`, `model.per_layer_proj_norm.weight` | `norm.weight`, `per_layer_projection_norm.weight` | copy |
| `model.layers.N.inp_gate.weight` [8,1536,32] | `layers.N.per_layer_input_gate.weight` [256,1536] | decode tiling `W.reshape(N/32,32,K).transpose(0,2,1)` |
| `...inp_gate.weight_prefill` [4,6,16384] | same | prefill tiling `W.reshape(N/64,64,K/256,256).transpose(0,2,1,3)` |
| `model.layers.N.per_layer_projection.weight` [48,256,32] / `_prefill` [24,1,16384] | `layers.N.per_layer_projection.weight` [1536,256] | decode / prefill tiling |
| `model.per_layer_model_proj.weight_layerN` [8,1536,32] | rows N*256..N*256+256 of `per_layer_model_projection.weight` [8960,1536] | decode tiling |
| `model.per_layer_model_proj.weight_prefill` [140,6,16384] | whole `per_layer_model_projection.weight` | prefill tiling |
| `rope_freqs.weight` [256] | config | global_head_dim/2 = 256 entries: first 512*0.25/2 = 64 are 1.0, rest 1e30 (bf16 0x714A): proportional RoPE of the full-attention layers |
| `model.layers.N.{self_attn.q,k,v,o_proj, mlp.gate,up,down_proj}.weight` | same | Q4 tiles (below) |
| `lm_head.weight` [49152,5120] | `embed_tokens.weight` (tied) | Q4 tiles, no scaling |
| `model.embed_tokens.weight` I8 [262144,1536] + `.scale` F32 [262144,48] | `embed_tokens.weight` | x sqrt(1536), int8 per 32 (below) |
| `model.per_layer_token_embd.weight` I8 [262144,8960] + `.scale` F32 [262144,280] | `embed_tokens_per_layer.weight` | x sqrt(256)=16, int8 per 32 |

KV-shared layers (15..34) still have their own k/v_proj in both HF and FLM files.

## Q4 tile layout (solved, verified by round trip on every Q4 tensor)
HF weight W [N out, K in]. FLM tensor I8 [N/32 * K/256, 5120]; each row is one
tile of 32 output rows x 256 input columns; tile index = rt*(K/256) + kt
(column tiles fastest). Row bytes:
- 0..512: 256 BF16 scales, 512..1024: 256 BF16 mins; entry g = j*32 + r, where
  r = row in tile (0..31), j = 32-column group (0..7). Group = 32 consecutive
  input columns of one output row (normal Q4_1 grouping along K).
- 1024..5120: 8192 nibbles, low nibble of each byte first. Nibble n holds
  r = (n>>12)*16 + (n&15), j = (n>>9)&7, i = (n>>4)&31 (column inside group),
  i.e. order [row-half 2][group 8][column 32][row 16].
- w = scale*q + min, q 0..15; min <= 0 always (quantizer clamps min to <= 0).

How it was found: scale means of lm_head blocks repeat with period 6 (=1536/256)
-> tiles of 32x256; a min/max fit picked "groups column-major in the tile";
then the nibble map came from correlating, across all 1152 tiles of
`layers.0.mlp.gate_proj`, each nibble position with each (r, column) of the
tile's values in "grid units" (w-min)/scale: every position had one partner with
correlation 0.997 and the map was a bijection, which is the formula above.

## Q4 values (partially understood; NOT reproduced)
- FLM nibble = round((w - min)/scale) with its stored params for 99% of weights;
  mean |w - dequant| = 0.225 step (near ideal rounding), so no GPTQ-style error
  feedback and no AWQ column scaling of the stored values.
- Its (scale, min) are not min/max (only ~8% of mins equal the group min) and
  not ggml make_qkx2/qkx3 with weights 1, x^2, |x| or rms+|x| (<= 9% exact),
  and not an unweighted least-squares fixed point (LS on FLM's own nibbles gives
  its params for only ~22% of groups).
- It is activation-aware: the per-input-column error relative to min/max
  quantization is shared by q/k/v of a layer (corr 0.97) and by gate/up
  (0.99), but not across layers or between minmax quantizations of those
  matrices (corr ~0.1-0.4, some negative). Some columns get ~100x lower error.
  So FLM weights the error per input channel with calibration statistics
  (imatrix-like). Weighted LS with weights 1/(FLM's own per-column error)
  raises exact params to ~29%, consistent but far from exact.
- Consequence: byte-for-byte reproduction needs FLM's calibration data or
  importance vectors, which aren't published. Plain unweighted RMSE vs the
  source: FLM ~0.0028, ours (qkx) ~0.0026 on q_proj; whether FLM's weighting
  gives better model output is untested (needs the NPU).
- Possible next step: estimate a per-(layer, input) importance vector from FLM's
  file itself (e.g. fit weights so a weighted make_qkx search reproduces FLM's
  params) and reuse it for fine-tunes of the same base.

## Int8 embeddings (format solved, values not reproduced)
int8 per 32 consecutive values of a row, F32 scale per group, value = q*scale,
scale = amax/127 (|q| reaches 127 in every group). FLM's int8 values were made
from an already-quantized tensor: `embed_tokens` groups take only 16 distinct
values (a Q4_1 grid of x*sqrt(1536)), `per_layer_token_embd` about 20 of 32
(a 5-bit grid of x*16). Those intermediate grids are not the lm_head's Q4 (65%
of int8 equal when re-deriving from lm_head). We quantize directly from BF16
to int8 (more precise than FLM's); see verify output for the differences.

## Dead ends (kept so nobody repeats them)
- Exact (min, (max-min)/15) fingerprints and contiguous 32-windows along rows
  or columns, against both HF repos: no match (wrong tiling + optimized params).
- Tolerant min/max fingerprints and sorted-value correlation: not
  discriminative (hundreds of near matches per group).
- Ordered correlation of 16-byte-contiguous groups (several nibble orders):
  chance level, because groups are spread over the tile (see layout).

## Check
`python3 verify.py <hf dir> <FLM model.q4nx> <out.q4nx>` converts and compares.
On a 12-core laptop: convert ~13 min, compare ~5 min, a few GB of RAM; no NPU.

Result against google/gemma-4-E2B-it (full log: `verify-output.txt`):
- header EXACT (81056 bytes), same tensor set, same file size (4,671,875,302).
- 459/709 tensors byte-identical: every BF16 tensor.
- 246/246 Q4 tensors: layout round trip EXACT; 57-70% of bytes equal,
  76-85% of nibbles equal, 7-20% of scales/mins equal; RMSE vs source lower
  for ours (e.g. lm_head FLM 0.00266, ours 0.00220).
- embed_tokens int8 10.8% equal, mean |ours-FLM| 0.073 (values are x39.2);
  per_layer_token_embd 18.1% equal, mean 0.032; scales differ (median 1.5% / 0.8%)
  because FLM's int8 comes from its 4/5-bit grids and ours straight from BF16.
