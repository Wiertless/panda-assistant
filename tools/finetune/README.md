# Fine-tuning Gemma 4 E2B for Panda

A LoRA fine-tune of `google/gemma-4-E2B-it` on Panda's own conversations, run on a
free GPU, merged and converted on the laptop, and run by FastFlowLM on the NPU.

## The steps

1. **Data** (`data.py`, on any computer): ~1,000 conversations in the app's exact
   shape: the system prompt and tool schemas captured from `assistantd`
   (`template.json`), the language note as the app writes it (its detector,
   ported from `core/src/agent.rs`), a tool call on the first round (chit-chat
   through `reply`), tool results, answers. Each example carries its own tools,
   `reply`, `plan` and 10 others. `python3 -I data.py out/train.jsonl` builds and
   checks them: tools and required arguments exist, enums are valid, and no user
   message is one of the eval's prompts (the eval must stay unseen).
   - `data.py`: the basics; `habits.py`: tools before answers; `judgement.py`:
     screen context, asking back, errors; `realistic.py`, `talk.py`,
     `colloquial.py`, `more.py`: hand-written; `generated*.py`: families over
     pools of real values. About a third is hand-written; nearly half the Polish
     turns also come typed without Polish letters.
2. **Train** (`train.py`, a free T4: Colab, or Kaggle once it gives GPU quota):
   ```
   !git clone -q --depth 1 --branch claude/finetune https://github.com/Wiertless/panda-assistant /content/panda-assistant
   !PANDA_EPOCHS=1.5 PANDA_WORK=/content python /content/panda-assistant/tools/finetune/train.py
   ```
   4-bit base, LoRA rank 16 on the language model's linear layers; rendered with
   **FLM's** chat template (`chat_template.jinja`; Google's newer one is kept as
   `chat_template.google-2026-07-09.jinja`), tool-call arguments as objects (the
   model's own call syntax), loss only on what the model writes, logits only at
   those positions. ~55 s a step on one T4 for ~3K-token examples. Download
   `panda-lora.zip` (~90 MB).
3. **Merge and convert** (`make_npu_model.sh ZIP NAME`, the laptop, ~15 min):
   `merge.py` writes W + (B @ A) * alpha / r into a reflink copy of the base
   weights (small memory); `../flm-convert/convert.py` turns them into FLM's
   `model.q4nx`; the result goes into its own model root,
   `~/.cache/flm-convert/ft-root-NAME/models/Gemma4-E2B-IT-NPU2/`, next to the
   stock model's other files (FLM appends `models/` to `FLM_MODEL_PATH`).
4. **Run and measure**: `FLM_MODEL_PATH=~/.cache/flm-convert/ft-root-NAME flm serve
   gemma4-it:e2b --ctx-len 16384 --port 8092`, then `assistantd --eval --url
   http://127.0.0.1:8092 --model gemma4-it:e2b --json out.json`, against stock E2B
   with the same build. Ship only what's clearly better.

## Results

| Run | Data | Full NPU eval (381 prompts) |
|---|---|---|
| stock Gemma 4 E2B | — | 321/381 (84%), 5.0 s/request |
| ft1: 444 conversations, 2 epochs | first pipeline run | 340/381 (89%), 4.9 s/request: **BETTER**, +4.8 points (95% +0.6 to +9.2), p = 0.01, 27 families better, 11 worse |
| stock, 0.35.0 RC build (b22db7f) | — | 324/381 (85%), 5.2 s/request |
| ft1, same RC build | | 340/381 (89%), 4.9 s/request: UNDECIDED, +3.6 points (95% −0.7 to +7.8), p = 0.10, 24 better, 13 worse, 5% faster |
| ft2: 993 conversations, 1.5 epochs, RC build | before `balance.py` | 340/381 (89%), 4.6 s/request: **BETTER**, +4.6 points (95% +1.2 to +8.2), p = 0.04, 20 better, 8 worse, 11% faster |

(The first two rows ran on the build before `change: up/down` for volume and
brightness, which counts against ft1 on those scenarios.)

## What ft1 got wrong (and the data now covers)

On the RC build ft1 broke 14 scenarios that stock passed, nearly all from
imbalances in the first data: short text files read with `ask_document` (32
examples against 2 for `read_file`), text the user gave sent as a file
(`send_file` 60, `send_text` 14), "make a file that says…" as `make_folder`,
what's open read and then passed to `ask_document`, `find_files` repeated 8
times when nothing matched (2 examples of an empty result). `balance.py` adds
those cases, and `data.py` now writes `find_files` results as the app does (bare
paths: the first data had sizes and dates after each one).

## Lessons from the first run

- `prepare_model_for_kbit_training` casts every non-4-bit weight to float32:
  Gemma 4 E2B's per-layer embeddings then need 8.75 GB (out of memory on a T4).
- Full-vocabulary logits for a ~3K-token conversation are 3.3 GB in float32:
  compute them only where the loss is (`logits_to_keep`).
- transformers 5 dropped `warmup_ratio`.
- Kaggle gives new accounts 0 GPU hours until a weekly reset: runs queue forever.
