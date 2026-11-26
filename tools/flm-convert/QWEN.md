# FLM .q4nx for Qwen3.5-9B: findings (2026-11-26)

Goal: run a Qwen3.5-9B fine-tune (first: an abliterated one,
`huihui-ai/Huihui-Qwen3.5-9B-abliterated`) on the NPU with FastFlowLM's own,
unmodified Qwen3.5-9B-NPU2 kernels, by writing its weights in FLM's format.
Decoded against FLM 1.0.13's `Qwen3.5-9B-NPU2/model.q4nx`. `convert_qwen.py`
does the conversion. For Gemma 4 E2B see `NOTES.md`: the container is the same,
but the quantization is not.

Status: **works.** The abliterated model, converted, runs on FLM 1.0.13's own
kernels: "capital of Australia" gives "Canberra", it answers in Polish ("Warszawa,
około 1,8 miliona mieszkańców"), 17 x 23 = 391, a tool call comes back as
`set_dark_mode {"enabled": true}`, and it answers what the original model
refuses. Decoding runs at 5–6 tokens/s with an 8K context.

## Container
As for Gemma: 8-byte LE header length, compact JSON padded with spaces to a
multiple of 8, no `__metadata__`, data with no gaps, tensors ordered by dtype
(F32, BF16, I8) and then by name. 475 tensors: I8 249, BF16 178, F32 48. The
shapes and offsets don't depend on the weights, so `convert_qwen.py` reuses
FLM's header verbatim (it reads only the header from FLM's file) and fills in
the data. FLM's bytes aren't in this repo.

## Name map (HF prefix `model.language_model.`)
| FLM | from HF | how |
|---|---|---|
| `model.embed_tokens.weight` | `embed_tokens.weight` | BF16 copy (bit for bit with FLM's) |
| `lm_head.weight` [7760,16,8704] | `lm_head.weight` (not tied) | Q8 |
| `…input_layernorm`, `…post_attention_layernorm`, `model.norm`, `self_attn.q_norm/k_norm` | same | BF16 of **1 + w**, rounded to nearest even (bit for bit) |
| `linear_attn.ssm_norm.weight` | `linear_attn.norm.weight` (F32) | BF16 of w as is (bit for bit) |
| `linear_attn.ssm_conv1d.weight` [4,8192] | `linear_attn.conv1d.weight` [8192,1,4] | transposed (bit for bit) |
| `linear_attn.ssm_a` (F32) | `linear_attn.A_log` | **−exp(A_log)** (to 1 float32 ulp) |
| `linear_attn.ssm_dt.bias` (F32) | `linear_attn.dt_bias` (BF16) | widened (bit for bit) |
| `linear_attn.ssm_alpha_proj.weight` / `ssm_beta_proj.weight` [1,16,8704] | `in_proj_a` / `in_proj_b` | Q8 |
| `…ssm_alpha_proj.bf16.weight` / `…ssm_beta_proj.bf16.weight` | the same two | BF16 copy, kept as well |
| `linear_attn.qkv_proj.weight` | `linear_attn.in_proj_qkv.weight` | Q4 |
| `self_attn.gate_proj.weight` (the 24 linear-attention layers) | `linear_attn.in_proj_z.weight` | Q4 |
| `linear_attn.ssm_out_proj.weight` | `linear_attn.out_proj.weight` | Q4 |
| `self_attn.q_proj` (8 full-attention layers) [8192 rows] | same | Q4, rows reordered: HF interleaves each head's query and output gate (`[q0, g0, q1, g1, …]`, 256 rows each); FLM has all 16 queries, then all 16 gates |
| `self_attn.k/v/o_proj`, `mlp.gate/up/down_proj` | same | Q4 |

Not in FLM's text file: the `mtp.*` (multi-token prediction) layers and the
vision tower (FLM keeps that in `vision_weight.q4nx`).

## Tiles
A matrix W [N out, K in] becomes I8 [N/32, K/256, bytes]: one tile per 32 rows
x 256 columns, row tiles outermost. Inside a tile everything is column-major:
value n is row n % 32, column n // 32; per-group entry p is row p % 32, group
p // 32 (a group is 32 consecutive columns of one row, 8 per row).

### Q4 tile, 4736 bytes: Q4_K with 8-bit sub-scales
| bytes | what |
|---|---|
| 0–256 | `sc`, u8 per group (entry p) |
| 256–512 | `mn`, u8 per group |
| 512–4608 | 8192 4-bit `q`, low nibble first (value n) |
| 4608–4672 | `d`, BF16 per row (32) |
| 4672–4736 | `dmin`, BF16 per row, negative |

**w = d·sc·q + dmin·mn.** One row's 256 values are a Q4_K super-block, but
with 8-bit scales and mins (0–255) where ggml packs 6-bit ones.

### Q8 tile, 8704 bytes
Bytes 0–512 hold a BF16 scale per group (entry p), then 8192 int8 values
(value n). **w = s·q**, with q from −127 to 127.

## How it was found
1. Header listing: same tile geometry as Gemma (32 x 256), but 4736 and 8704
   bytes per tile instead of 5120.
2. Over 208 tiles of `layers.0.mlp.gate_proj`, every nibble position was
   correlated with every weight in its tile. Bytes 512–4608 gave a one-to-one
   map with |corr| > 0.9 (the column-major order above). Bytes 0–512 and
   4608–4736 held no weights.
3. Per (row, 32-column group), w ≈ s·q + c was fitted: 7% relative error, the
   size of 4-bit rounding (32-row groups: 37%). Correlating header bytes with
   the fitted s and c across tiles put one u8 per group's s in bytes 0–256 and
   one per group's c in 256–512.
4. A community model card (f3nc3r/Qwen3.5-4B-NPU2, "Q4_K / Q8_0 hybrid") gave
   the scheme. The last 128 bytes, read as BF16 d[32] then dmin[32], matched
   s/sc and c/mn to 0.2%.
5. The first conversion ran but answered gibberish ("134 / 60 / 25" for the
   capital of Australia). Decoding every tile of both files and comparing them
   found one tensor kind off: `q_proj` in the full-attention layers, 3840 of
   4096 tiles (the first 512 rows matched). Matching each of FLM's rows to the
   closest HF row (cosine 0.994 or more, one to one) showed the query/gate
   reorder above. With it, every tile matches FLM's within quantization noise.
6. Check: FLM's tiles decoded with this layout differ from the abliterated
   weights by 7.3–7.9% on Q4 tensors (all 8 kinds) and 0.6% on Q8 ones.
   Tiles packed by `convert_qwen.py` and decoded by an independent reader:
   7.0% and 0.54%. The quantizer is ggml's make_qkx2 search per group, then d
   = max(s)/255 and dmin = min(m)/255 per row.

## Running it
```
python convert_qwen.py HF_DIR ~/.config/flm/models/Qwen3.5-9B-NPU2/model.q4nx OUT/model.q4nx
```
22 minutes on a Ryzen AI laptop (performance power profile). It streams one tensor at a time and
stays under 100 MB of its own memory; on a 14 GB machine run it under a cap
anyway (`systemd-run --user -p MemoryMax=3G …`). Put the result in a model
folder with FLM's `config.json`, tokenizer files, chat template and
`vision_weight.q4nx` (the abliterated model has the same tokenizer; keep FLM's
chat template, which is what FLM's tool calling expects). To try it without
touching FLM's own model: `FLM_MODEL_PATH=ROOT flm serve qwen3.5:9b`, with the
folder at `ROOT/models/Qwen3.5-9B-NPU2`. Replacing FLM's `model.q4nx` makes it
the `qwen3.5:9b` everything uses; a FLM model update may download the original
over it.

To find a broken tensor, decode every tile of both files and compare them
(dequantized, relative RMS per tile): the same weights quantized twice differ by
about 8% on Q4 and 0.5% on Q8, so a reorder or a wrong transform stands out
at 100% and up.
