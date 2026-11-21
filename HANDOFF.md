# Handoff: where the work stands (2026-11-17)

Read this first, then `README.md`. The redesign plan lived in the cloud session's scratchpad and is gone; its decisions are summarized below.
Delete this file once the work below is done.

## The two repositories

- **panda-assistant** (this one): the assistant, Rust. `core/` = `assistantd` (D-Bus service, agent loop, tools), `app/` = GTK4/libadwaita window.
- **silverpanda**: the Fedora Silverblue 45 BlueBuild image. `files/scripts/panda-assistant.sh` pins a release (VERSION + SHA256). Lemonade Server is installed by `files/scripts/lemonade.sh`.

## How releases go

1. Commit on `claude/great-ride-v90usy`, fast-forward `main`, push both.
2. When `ci` is green on main, run `ci.yml` by hand (workflow_dispatch) on main: that run makes the GitHub release.
3. Check the tarball (core-bin/assistantd, core-bin/panda-assistant, the clipboard extension, Reminders.desktop) and its sha256 against the release's `.sha256`.
4. In silverpanda: branch `claude/great-ride-v90usy` from origin/main, bump VERSION/SHA256, PR, wait for `bluebuild`, check its log shows `assistantd X` / `Assistant X`, merge. The owner allows merging these pin PRs without asking.

Released so far: Assistant **0.23.0** (pinned in silverpanda). `main` has more on top, unreleased (below).

## The owner's wishes (keep to them)

- No hand-written intent matcher ("don't hardcode commands"); keyword routing exists but the plan is to replace it with learned routing.
- **Models stay local** (no cloud models). Small model on the NPU (Gemma 4 E2B or Qwen 3.5 4B); a bigger one (qwen3.5:9b) for hand-offs and research writing.
- **No proactive reminders or suggestions** unless asked.
- Mail must work with **any mail app** ("Convey" is theirs): read mail over IMAP through GNOME Online Accounts, plus local Thunderbird/Evolution/Geary stores as a fallback.
- Polish and English. Terse. fish shell.
- LoRA: dropped for now (FLM can't load fine-tunes; GPU via Lemonade is too heavy). Later: a merged-LoRA weight converter for FastFlowLM (ROCm/FastFlowLM, MIT, maintainers use AI tools), as a PR, **after** this project.

## Done on main but not released yet (would be 0.24.0)

- File tools like a coding agent's: `write_file`, `edit_file` (exact old_text→new_text), `move_file`, `make_folder`, `trash_file`, `search_text`; all undoable, changing ones ask first; same limits as reading (inside home, no hidden files). Compact set has one broad `change_file`.
- Reads all common file types directly (`core/src/tools/extract.rs`): docx/pptx/xlsx, ODF, EPUB, RTF, HTML, eml, ipynb, zip lists, any text/code; PDFs via `pdftotext` (**add `poppler-utils` to the silverpanda image**).
- The smarter model gets 20 rounds (HELPER_ROUNDS) for multi-step work.
- Eval: judge fixes for Polish wordings; EmbeddingGemma server in the eval job (`--embed-url`), `no-embed`, `required`, `compact-required` variants; partial reports survive the 3-hour job limit; model calls and prompt tokens per request in the report.
- `rewrite_selection` gets picked for selected text; "put on some X" routes to music.

## Done since

- 0.24: in-depth research with PDF/MD/TXT export, reply offered every round (prompt cache: 0.6 s to first token), pop-up (`--popup`, Super+Space via silverpanda dconf), file tools, all file types readable.
- 0.25.0: mail search/read over IMAP via GNOME Online Accounts (any mail app). Pinned in silverpanda (PR 41).
- `assistantd --tool NAME JSON` runs one tool, for live tests. Live testing: download the GitHub release, `pkill -x assistantd`, run the release's `core-bin/assistantd`, call it over D-Bus (`gdbus call ... Core1.Ask`).
- Rust on the laptop: the `panda-dev` toolbox; call `/usr/bin/cargo-fmt` and `/usr/bin/cargo-clippy clippy` directly (~/.cargo/bin has broken rustup shims). `gh` is in that toolbox. Build on GitHub, not locally, when possible.

- 0.26–0.29 released: search_everything, speed line under answers, slim pop-up bar (spring damping 0.425), clipboard extension auto-enable, get_weather + cards (weather/events/reminders via `tools::CARDS`, hint via `next_step`), answer-length line in the prompt, local mail dates. 0.29.0 pinned in silverpanda (PR 42).
- Live checks of 0.29: weather → one call + one sentence; calendar → one call; "ile to 15% z 240?" answered right but **in English** (small-model language slip on short questions; not fixed).

- Redesign step 1 built (0ef7cce): kNN over `tools/requests.tsv` (EmbeddingGemma), variants `examples`, `examples-force`, `examples-only`, `examples-force-only`. **Eval run 44282799806** (Gemma E2B) compares them with baseline: read it, ship the winner as the default, and if an `-only` variant holds up, delete keyword routing (step 3). Thresholds were calibrated on eval prompts (mild optimism).
- Step 2 (typed workflows) parked: of Gemma's 14 baseline failures, 8 are "called no tool", only 2 are chains.
- embed-gemma-300m-FLM is now downloaded on the laptop (routing by meaning was off there before). Lemonade 2026.46 forwards the wrong name for it; `flm.rs` works around it.

## Pinned / releasing

- 0.30.0 pinned (silverpanda PR 44). silverpanda PR 45 merged: lemond MemoryHigh=10G, MemoryMax=11G.
- 0.31.0 released and pinned (owner pinned it; SHA matches): research in depth by default + gap round + "quick" from the user's words, writer by free memory (9B only with >= 10.5 GB available, else qwen3.5:4b), write_file to .pdf, "what's playing" names the app, unload after 15 min. The owner pins it in silverpanda himself (VERSION/SHA256 in files/scripts/panda-assistant.sh).
- Open: embedding model on or off by default (owner tried 0.30 with it); "ciszej proszę" volume value; English replies to Polish math.

- silverpanda PR 46 merged: the nightly restart counts only apps with a visible launcher (was skipped for the DiskUtilityNotify helper; missed D-Bus-started apps like Ptyxis). Tested as root on the laptop. Takes effect with the image after 45.20261120.4.

## Speed work (2026-11-21): eval done, implemented, not released

Measured: first round cold (median 4.3 s) because routing changed the tool list each question and FLM only reuses an exact instructions+tools block. **Eval 44920114036** (Gemma E2B, repeat 3): all-tools 91.3% vs baseline 91.1% (+0.4, 95% −1.6..+2.3), 98% vs 90% of the prompt reused; all-tools-short no gain (not shipped). On the NPU: all tools 7.5K tokens read once in 6.4 s (done at load by `Agent::warm()`), then 0.65 s per question; 16K = same memory as 8K.

On claude/great-ride-v90usy (790eb64, on top of c60a873): `route_tools = ctx_len < ALL_TOOLS_CTX_LEN (16384)`, embedding model only loaded when routing, default context 16K, Gemma 4 E2B default model, warm-up after load. Tests + clippy pass. **Live-tested on the NPU (owner's OK):** the warm-up first missed because the machine context had free memory and battery charge in it (changed between chats → whole prompt re-read). 41cda23 keeps only stable values (get_system_info/get_battery give the current ones). After that: warm-up + three new chats, all 7469 tokens reused, 1.3 s to the first tool call (6 s before), 2.2 s for a whole reply; follow-up rounds 0.6–0.75 s. The owner's config is now `ctx_len: 16384` (backup of the old one was in the session scratchpad). **0.32.0 released** (f4aa643, release run 44933885668, SHA256 7b2575b9cac39fd0a08676b8732f1e83a1b34ad2bcc3deb451b75389affb1997, tarball checked). **silverpanda PR 48 pins it** (owner asked to pin): when `bluebuild` is green and its log shows `assistantd 0.32.0` / `Assistant 0.32.0`, merge. Then after the owner's next image: check rounds.jsonl for first-round times.

## Auto Brightness (silverpanda PR 47, 2026-11-21) — tested, works

Extension `auto-brightness@silverpanda.wiertless.github.io`: computes GNOME Shell's auto-brightness target (intercepts `Main.brightnessManager.autoBrightnessTarget`, so gsd-power's updates are kept but not applied) while Settings → Power → Automatic Screen Brightness is on. Curve = 7 draggable points at 0/10/50/150/500/1500/3500 lux (monotone cubic over log lux; default 3/8/15/25/50/80/100% from measured preferences), `response` glide, slider = GNOME's shift with scaling only near 0/1 (keys never dead). Keyboard backlight with its own switch (on below 70% of `keyboard-off-lux`, off above; `set property`/`external` changes are the user's and learned, gsd's own restores get overridden). Quick Settings tile with a keyboard switch and Settings. Polish translation (po/pl.po → locale/, compiled .mo committed). **Owner tested after relog: all works.** Then added (77576af, untested, needs a relog): noise filter (a reading counts only if >12% and >=3 lux from the last one used). **Merged 2026-11-21 (81f1812), in image 45.20261121.2; the test copy in ~/.local was deleted, the system copy loads from the next login.** Was next: once the image has it, delete the test copy `~/.local/share/gnome-shell/extensions/auto-brightness@silverpanda.wiertless.github.io` (it shadows the image's). The owner's settings stay in dconf.

## Eval state (2026-11-18 evening)

- 160 families (345 prompts), `eval_verdict.py` heads every summary (sign test per family). `--shard K/N`, workflow input `shards` = 3, 4 or 6.
- **Routing verdict (run 44372794027, repeat 3):** `examples` +0.5 points (95% range −1.3 to +2.3) — no real difference; keep baseline routing, `examples` stays only as a variant.
- **Cue-words verdict (44403890158):** overall +1.0 (undecided), but on the target families ciszej 8/9 vs 3/9, "co teraz leci" 4/6 vs 0/6, nothing nearby worse, 18% faster → kept, released in 0.30.0. Was: cue-words eval 44403890158 (repeat 3, 6 shards): `baseline` = new descriptions with Polish cues, `pre-cues` = old ones. pre-cues WORSE → keep the cues and release 0.30.0; no difference → revert them.
- Run over 160: `examples` +1.9 points vs baseline, UNDECIDED (p 0.24), same direction as the 48-family run. **Decisive run 44372794027** (repeat 3, 6 shards, made before the description fixes below): if BETTER, make `route_by_examples` the default and release 0.30.0.
- No-tool fixes in progress: Polish cue words in set_volume/set_brightness/media_control/open_file descriptions, media keywords ("leci", "playing", "utwór"), eval failures now include the reply. Local NPU check: "ścisz trochę" and "a bit quieter" pass, "ciszej proszę" now calls set_volume (value still off). Next: rerun these families locally (`assistantd --eval --url http://127.0.0.1:8002 --model gemma4-it:e2b --only NAME`, ~4 s/prompt on the NPU), then a CI eval of baseline before/after.

## Idea on hold (owner deciding): super-heavy research

A very long overnight research mode with a big model for a better PDF. AirLLM (layer streaming) is the wrong engine: a dense 70B streamed from the NVMe is ~10 s/token. Instead a big **MoE** via llama.cpp with mmap (CPU + SSD, NPU stays free):
- **gpt-oss-120b** (~63 GB, 5.1B active, est. 1–3 t/s, 1–3 h per report) — the pick; Qwen3-Next-80B-A3B (~45 GB, 3B active) as the alternative; Qwen3-235B-A22B too slow.
- Middle ground: gpt-oss-20b (11.6 GB, in Lemonade's list) on the iGPU, 20–40 min, but needs ~12 GB of the 14 GB RAM free.
- Plan: ~30 pages, small model takes notes on the NPU, the big model writes each section, then a critique-and-revise pass, notification when the PDF is ready. **First measure real speed** (needs the ~63 GB download — ask first). Laptop: 251 GB disk free.

## Eval verdicts (Gemma 4 E2B, CPU, 119 prompts)

baseline 105-107, no-embed 104, required 101 (+35% tokens), compact 83, compact-required 81.
So: keep narrow tools + routing + embeddings; **don't** switch to the compact set or required rounds.

## Next, in order

1. ~~**Pop-up**~~ (done in 0.24) on a global shortcut (Super+Space): a small floating box at the top of the screen, Esc closes; the full window stays for long chats. It attaches the current context automatically: focused app + window title, selected text (clipboard extension), last copied text, a screenshot when asked about the screen.
2. ~~Mail~~ (done in 0.25); still to do: one **personal search** across files (LocalSearch + embeddings), calendar, contacts and **mail** (IMAP via GNOME Online Accounts; local stores as fallback).
3. **The rest of the redesign** (decided earlier with a review agent):
   - ~~compact toolbox~~ and ~~required rounds~~: refuted by the eval (see above);
   - a learned proposer: k-nearest-neighbour over EmbeddingGemma vectors of example requests → force the likely tool with `tool_choice` naming it;
   - typed workflows behind the tools (finish the job in Rust, error kinds Done/NeedChoice/Fixable/Fatal);
   - delete keyword routing, more_tools, nudge/force heuristics once the above beats them in the eval.
4. ~~**Answers that fit**~~ (done in 0.29).
5. The FastFlowLM merged-LoRA PR.

## Evals running when this was written

On GitHub Actions (eval.yml, Gemma 4 E2B and Qwen 3.5 4B):
- run 44140760696: `baseline` vs `required` vs `compact` vs `compact-required` — decides step 3.
- run 44143234436: `baseline` (with embeddings) vs `no-embed`.

Earlier result (Gemma 4 E2B, 2 runs × 116 prompts): owner's 0.18.1 prompts 205/232 (88%), 34.2 s/request; the 0.18.0 prompts 208/232 (90%), 31.6 s, Polish 18/20 vs 15/20. Suggested but **not applied** (needs the owner's OK): keep their prompt's structure, put back the tool names in the web/memory/escalate parts and "a reply without a tool call ends your turn", drop "verify when practical".

## On the laptop

- Speed log: `~/.local/state/panda-assistant/rounds.jsonl` (prompt/cached tokens, time to first token). Collect after a day of use to measure the prefix cache.
- FLM log: `~/.local/state/panda-assistant/flm-serve.log`; Lemonade: `journalctl --user -u lemond`.
- Night-time update restart: `silverpanda-nightly.path` arms `silverpanda-nightly.timer` only while an update is staged; logs `journalctl -u silverpanda-nightly`.
