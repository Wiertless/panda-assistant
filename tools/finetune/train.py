"""LoRA fine-tune of Gemma 4 E2B on Panda's conversations, for a Kaggle notebook
(GPU T4 x2, Internet on). Run the cells of train.ipynb, or `python train.py`.

It clones the repo, builds the data with data.py, renders every conversation
with FastFlowLM's own chat template (what the laptop runs, not Google's newer
one), trains only on the assistant's parts, and saves the LoRA adapter in
/kaggle/working/panda-lora (download that; the laptop merges and converts it).
"""

import json
import os
import subprocess
import sys

os.environ.setdefault("PYTORCH_CUDA_ALLOC_CONF", "expandable_segments:True")

REPO = "https://github.com/Wiertless/panda-assistant"
BRANCH = os.environ.get("PANDA_BRANCH", "claude/finetune")
WORK = os.environ.get("PANDA_WORK", "/kaggle/working")
MODEL = "google/gemma-4-E2B-it"
MAX_LEN = 5120
EPOCHS = float(os.environ.get("PANDA_EPOCHS", "2"))
LR = 2e-4
RANK = 16


def sh(cmd):
    print("$", cmd, flush=True)
    subprocess.run(cmd, shell=True, check=True)


def setup():
    sh("pip install -q -U transformers peft accelerate bitsandbytes")
    src = f"{WORK}/panda-assistant"
    if not os.path.isdir(src):
        sh(f"git clone -q --depth 1 --branch {BRANCH} {REPO} {src}")
    sh(f"python3 {src}/tools/finetune/data.py {WORK}/train.jsonl")
    return src


def assistant_spans(tok, template, msgs, tools):
    """Character spans of what the model writes: each assistant message, found by
    rendering the conversation up to it with and without it."""
    spans = []
    for k, m in enumerate(msgs):
        if m["role"] != "assistant":
            continue
        before = tok.apply_chat_template(msgs[:k], tools=tools, chat_template=template,
                                         tokenize=False, add_generation_prompt=True)
        upto = tok.apply_chat_template(msgs[: k + 1], tools=tools, chat_template=template, tokenize=False)
        # The template may close the last turn differently than a continued one; the
        # assistant's own text always starts right after the generation prompt.
        spans.append((len(before), len(upto)))
    full = tok.apply_chat_template(msgs, tools=tools, chat_template=template, tokenize=False)
    return full, spans


def native(msgs):
    """Tool-call arguments as objects: the template then renders them the way the model
    itself writes calls (FLM turns those into the JSON the app gets)."""
    out = []
    for m in msgs:
        m = dict(m)
        if m.get("tool_calls"):
            m["tool_calls"] = [
                {**c, "function": {**c["function"], "arguments": json.loads(c["function"]["arguments"])}}
                for c in m["tool_calls"]
            ]
        out.append(m)
    return out


def encode(tok, template, row):
    text, spans = assistant_spans(tok, template, native(row["messages"]), row["tools"])
    enc = tok(text, return_offsets_mapping=True, add_special_tokens=False, truncation=True, max_length=MAX_LEN)
    labels = []
    for (a, b), tid in zip(enc["offset_mapping"], enc["input_ids"]):
        inside = any(s <= a and b <= e for s, e in spans)
        labels.append(tid if inside else -100)
    return {"input_ids": enc["input_ids"], "attention_mask": enc["attention_mask"], "labels": labels}


def main():
    src = setup()
    import torch
    from peft import LoraConfig, get_peft_model
    from transformers import (AutoModelForCausalLM, AutoTokenizer, BitsAndBytesConfig, Trainer,
                              TrainingArguments)

    template = open(f"{src}/tools/finetune/chat_template.jinja").read()
    tok = AutoTokenizer.from_pretrained(MODEL)
    rows = [json.loads(l) for l in open(f"{WORK}/train.jsonl")]
    data = [encode(tok, template, r) for r in rows]
    trained = sum(sum(1 for x in d["labels"] if x != -100) for d in data)
    print(f"{len(data)} conversations, {sum(len(d['input_ids']) for d in data)} tokens, {trained} trained on")
    # Show one, so a template problem is visible before hours of training.
    first = data[0]
    print("TRAINED PART OF THE FIRST:", repr(tok.decode([t for t in first["labels"] if t != -100])[:600]))

    model = AutoModelForCausalLM.from_pretrained(
        MODEL,
        quantization_config=BitsAndBytesConfig(load_in_4bit=True, bnb_4bit_quant_type="nf4",
                                               bnb_4bit_compute_dtype=torch.float16, bnb_4bit_use_double_quant=True),
        dtype=torch.float16,
        device_map="auto",
        attn_implementation="eager",
    )
    # Not peft's prepare_model_for_kbit_training: it casts every weight that isn't 4-bit
    # to float32, and Gemma 4 E2B's per-layer embeddings (262144 x 8960) alone then need
    # 8.75 GB (out of memory on a T4). LoRA trains only its own small weights; the rest
    # stays frozen in float16, so checkpointing and input gradients are all it needs.
    model.config.use_cache = False
    for prm in model.parameters():
        prm.requires_grad_(False)
    model.gradient_checkpointing_enable(gradient_checkpointing_kwargs={"use_reentrant": False})
    model.enable_input_require_grads()
    lora = LoraConfig(r=RANK, lora_alpha=RANK * 2, lora_dropout=0.05, task_type="CAUSAL_LM",
                      # The language model's linear layers only, not the vision or audio towers
                      # (names differ with how transformers loads Gemma 4: with or without
                      # "language_model").
                      target_modules=r"^(?!.*(vision|audio)).*\.(q_proj|k_proj|v_proj|o_proj|gate_proj|up_proj|down_proj)$")
    model = get_peft_model(model, lora)
    model.print_trainable_parameters()

    def collate(batch):
        n = max(len(b["input_ids"]) for b in batch)
        pad = tok.pad_token_id or 0
        return {
            "input_ids": torch.tensor([b["input_ids"] + [pad] * (n - len(b["input_ids"])) for b in batch]),
            "attention_mask": torch.tensor([b["attention_mask"] + [0] * (n - len(b["attention_mask"])) for b in batch]),
            "labels": torch.tensor([b["labels"] + [-100] * (n - len(b["labels"])) for b in batch]),
        }

    # transformers 5 has no warmup_ratio: about 5% of the steps as a count. Checkpointing
    # is already on (above), so the Trainer isn't asked to set it up again.
    steps = max(1, int(len(data) * EPOCHS / 8))
    args = TrainingArguments(
        output_dir=f"{WORK}/run", num_train_epochs=EPOCHS, per_device_train_batch_size=1,
        gradient_accumulation_steps=8, learning_rate=LR, lr_scheduler_type="cosine",
        warmup_steps=max(1, steps // 20), fp16=True, logging_steps=5, save_strategy="no", report_to=[],
        remove_unused_columns=False, optim="paged_adamw_8bit",
    )
    class TrainedOnly(Trainer):
        """The loss on the trained tokens only, with logits for just those positions:
        the full vocabulary (262K) for every position of a ~3K-token conversation is
        3.3 GB in float32 alone (out of memory on a T4), and only ~1% is trained."""

        def compute_loss(self, model, inputs, return_outputs=False, num_items_in_batch=None):
            labels = inputs.pop("labels")
            target = labels[:, 1:]
            keep = (target[0] != -100).nonzero().squeeze(-1)  # position t predicts token t+1
            out = model(input_ids=inputs["input_ids"], attention_mask=inputs["attention_mask"],
                        logits_to_keep=keep)
            loss = torch.nn.functional.cross_entropy(out.logits[0].float(), target[0, keep])
            return (loss, out) if return_outputs else loss

    TrainedOnly(model=model, args=args, train_dataset=data, data_collator=collate).train()
    out = f"{WORK}/panda-lora"
    model.save_pretrained(out)
    json.dump({"base": MODEL, "rank": RANK, "alpha": RANK * 2, "examples": len(data), "epochs": EPOCHS,
               "branch": BRANCH}, open(f"{out}/panda.json", "w"))
    sh(f"cd {WORK} && zip -qr panda-lora.zip panda-lora && ls -la panda-lora panda-lora.zip")

    # A quick look at what it learned, with FLM's template.
    model.eval()
    msgs = [{"role": "system", "content": rows[0]["messages"][0]["content"]},  # noqa: E501
            {"role": "user", "content": "przyciemnij trochę ekran\n\n(Answer in Polish.)"}]
    prompt = tok.apply_chat_template(msgs, tools=rows[0]["tools"], chat_template=template, tokenize=False,
                                     add_generation_prompt=True)
    ids = tok(prompt, return_tensors="pt", add_special_tokens=False).to(model.device)
    with torch.no_grad():
        outp = model.generate(**ids, max_new_tokens=60, do_sample=False)
    print("SAMPLE:", tok.decode(outp[0][ids["input_ids"].shape[1]:], skip_special_tokens=False))


if __name__ == "__main__":
    sys.exit(main())
