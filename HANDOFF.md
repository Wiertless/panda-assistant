# Handoff: where the work stands (updated 2026-11-21 evening)

Read this first, then `README.md`. The redesign plan lived in the cloud session's scratchpad and is gone; its decisions are summarized below.
Delete this file once the work below is done.

## Now: waiting for the owner's image update (2026-11-21 evening)

The owner is updating to the next silverpanda image (built from main after PR 49 merged), which brings **Assistant 0.32.0** and the **light sensor reconnect after sleep**. Auto Brightness is already in their image. When they're on it:
1. `rpm-ostree status` shows the new image; `/usr/libexec/panda-assistant/assistantd --version` says 0.32.0.
2. Speed: `~/.local/state/panda-assistant/rounds.jsonl` after some real use: round-0 `cached_tokens` should be ~7.4K and `first_ms` ~1.3 s (was 4.3 s median, mostly cold). Config already has `ctx_len: 16384`. No `--embed` on `flm serve`.
3. After a real suspend/resume: `journalctl -b -u silverpanda-als-after-sleep` ran, the sensor reads > 0 (`gdbus call --system -d net.hadess.SensorProxy -o /net/hadess/SensorProxy -m org.freedesktop.DBus.Properties.Get net.hadess.SensorProxy LightLevel`), the screen follows the room.

Still open after that: "ciszej proszę" volume value; English replies to Polish math; super-heavy research (owner deciding); FastFlowLM merged-LoRA PR.

## The two repositories

- **panda-assistant** (this one): the assistant, Rust. `core/` = `assistantd` (D-Bus service, agent loop, tools), `app/` = GTK4/libadwaita window.
- **silverpanda**: the Fedora Silverblue 45 BlueBuild image. `files/scripts/panda-assistant.sh` pins a release (VERSION + SHA256). Lemonade Server is installed by `files/scripts/lemonade.sh`.

## How releases go

1. Commit on `claude/great-ride-v90usy`, fast-forward `main`, push both.
2. When `ci` is green on main, run `ci.yml` by hand (workflow_dispatch) on main: that run makes the GitHub release.
3. Check the tarball (core-bin/assistantd, core-bin/panda-assistant, the clipboard extension, Reminders.desktop) and its sha256 against the release's `.sha256`.
4. In silverpanda: branch `claude/great-ride-v90usy` from origin/main, bump VERSION/SHA256, PR, wait for `bluebuild`, check its log shows `assistantd X` / `Assistant X`, merge. The owner allows merging these pin PRs without asking.

Released so far: see "Pinned / releasing" (latest **0.34.1**, pinned (silverpanda PR 57 merged 2026-11-23; 0.34.0 was PR 51); nothing unreleased on main).

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

## Speed work (2026-11-21): released in 0.32.0, pinned

Measured: first round cold (median 4.3 s) because routing changed the tool list each question and FLM only reuses an exact instructions+tools block. **Eval 44920114036** (Gemma E2B, repeat 3): all-tools 91.3% vs baseline 91.1% (+0.4, 95% −1.6..+2.3), 98% vs 90% of the prompt reused; all-tools-short no gain (not shipped). On the NPU: all tools 7.5K tokens read once in 6.4 s (done at load by `Agent::warm()`), then 0.65 s per question; 16K = same memory as 8K.

On claude/great-ride-v90usy (790eb64, on top of c60a873): `route_tools = ctx_len < ALL_TOOLS_CTX_LEN (16384)`, embedding model only loaded when routing, default context 16K, Gemma 4 E2B default model, warm-up after load. Tests + clippy pass. **Live-tested on the NPU (owner's OK):** the warm-up first missed because the machine context had free memory and battery charge in it (changed between chats → whole prompt re-read). 41cda23 keeps only stable values (get_system_info/get_battery give the current ones). After that: warm-up + three new chats, all 7469 tokens reused, 1.3 s to the first tool call (6 s before), 2.2 s for a whole reply; follow-up rounds 0.6–0.75 s. The owner's config is now `ctx_len: 16384` (backup of the old one was in the session scratchpad). **0.32.0 released** (f4aa643, release run 44933885668, SHA256 7b2575b9cac39fd0a08676b8732f1e83a1b34ad2bcc3deb451b75389affb1997, tarball checked). **Pinned: silverpanda PR 48 merged 2026-11-21** (bluebuild green, log shows assistantd/Assistant 0.32.0). Then after the owner's next image: check rounds.jsonl for first-round times.

## Smarter model keeps the chat (2026-11-22): released in 0.33.0 (panda-assistant PR 1), pinned (silverpanda PR 50)

Owner's Kinect chat: after a hand-off to qwen3.5:9b the follow-up went back to Gemma, which answered shallowly and wrongly. Now after `ask_smarter_model` (or a take-over when Gemma is stuck) `helper_holds` is set; each follow-up is weighed by the smarter model with a short one-shot (`HOLD_DECIDE`, last Q/A + new message → BIG/SMALL, a few seconds); BIG → it answers with the full chat (`hold_turn`), SMALL → Gemma answers and holds the chat again. A held turn starts with the smarter model loaded (service `attempt`), no compaction on held turns: Lemonade keeps one model, and loading Gemma evicted the 9B and its prompt cache. Held: big model idle unload 10 min. The hand-off note now says "You are the smarter model…" (it used to deny having one when the user wrote "ask the smarter model"). In memory only.

Live-tested (owner asked to keep going until it works): deeper follow-ups 56 s / 52 s, first token 2.3 s with 8.3–8.7K tokens reused (before: 269 s / 110 s, 40 s cold each); "thanks" → SMALL → Gemma, 20 s; two model swaps in the whole chat. A first try with a `hand_back` tool failed (the 9B never called it, once wrote "hand_back" as its reply) and was removed. Also seen earlier: the 9B once called `use_skill` first and started its answer with "that wasn't actually a saved skill" — not looked into.

## Knows its own setup (2026-11-22): released in 0.34.0 (panda-assistant PR 2), pinned (silverpanda PR 51)

Owner's pop-up chat: "what api does lemonade advertise" with a link selected in the terminal → Gemma rewrote the selection, then read_window("Terminal") found nothing ("No window is focused"). Built, all behind switches off by default: `list_servers` tool (/proc listening sockets + owner names, Lemonade/FLM/Ollama described), `self_info` line after the machine description (Lemonade at http://localhost:13305/api/v1), `context_rule` (screen context only when the question refers to it). read_window now also matches GNOME's app name via desktop ids (always on; live: "Terminal" finds Ptyxis). Eval baseline is now as shipped (all tools, 16K). Six new scenarios (lemonade address ×2 incl. a selected link, is ollama running, dev server port, two selection controls).

**Eval 44965080030** verdict (Gemma E2B, repeat 3): all three UNDECIDED overall (+1.0/+1.4/+1.7). self-aware (self_info + context_rule): lemonade address 0/3 → 3/3 wordings, 12 fixed / 7 broke, no time cost → **on by default** (`flm::self_info` from the client's backend + URL; context_rule true). local-servers: only variant that answers dev-server port / is-ollama-running, but proofread/translate selection dropped to 1–2/3 and self-aware-servers is +15% time → **off**, kept as variant `local-servers` (rerun on selection families before turning on). Check run 45035798333 with the new defaults: lemonade 4/5 (the "link selected ~2" wording still answers with the link's port 443).

Streamlined in the same release (~1,300 lines): removed required rounds, compact tool set + `Tool::resolve` forwarding, route_by_examples/force_proposal + `requests.tsv`, `parallel_tool_calls`; variants pruned (refuted or = baseline), added `routed` (8K, routing + embeddings) and `no-self-aware`. eval.yml: `plan` job builds the model matrix (no more skipped jobs), embedding server only for variants with route_tools. Research middle writer (qwen3.5:4b) only when installed — **the owner removed qwen3.5:4b from the laptop on 2026-11-22** and switched config to gemma4-it:e2b (measured on the NPU: Gemma 18–21 tok/s decode, ~1,800 tok/s prefill vs Qwen 4B 12 and ~500).

## In progress (2026-11-23 night): branch claude/agent-stage1, not released

All pushed, tests pass (131). On the branch: calculate tool; reply language named per turn (request-only note); newest mail marked; agent stage 1 (edit_image / images_to_pdf with ImageMagick, files by pattern or folder, results in Pictures/Edited, undo whole_task, "Allow for This Task" button); app: thinking block, live steps with times and details, Think First toggle (config `thinking`); stage 2: plan tool + checklist card + end-of-turn check, and a "call plan first" note for requests of 3+ actions. Owner tested live (dev build running from target/release instead of the installed service until a restart): undo and editing work; plan wasn't used before the plan-first note (task: find today's screenshot, PDF, send — it took a new screenshot and sent the PNG). That task is now an eval scenario; **next: run it on the NPU with baseline vs no-plan**, then release 0.35.0 with the owner's OK.

Evals: NPU E2B full 310/362 (86%), 4.2 s/request; E4B partial 98 vs 94 on the first 108 prompts at 2.4x the time → not worth it, deleted (also deleted: embed-gemma; laptop has only E2B + 9B). Hand-offs now judged in the eval (escalate_stub): clearer hand-off rule didn't help (9/12 = baseline), thinking on 11/12. GitHub: run 45265156274 = E2B vs QAT E2B (google/gemma-4-E2B-it-qat-q4_0-gguf), repeat 3 — read the verdict.

FLM converter: tools/flm-convert/NOTES.md. A cloud agent is reverse-engineering the q4nx layout (branch claude/flm-convert if it could push). Source weights downloaded on the laptop in ~/.cache/flm-convert (20 GB; delete when done).

## Research language (0.34.1, 2026-11-23)

An English research question's report came out in Turkish: qwen3.5:9b wrote it from notes in several languages and the prompts only said "the question's language". `in_language()` now names it (Polish by its letters, else English; shortcut noted) in the outline, section, summary and quick-answer prompts. Same chat, not fixed: "thanks" answered in Polish (small-model language slip), mail summary called an older mail "the newest". **Nightly update proven 2026-11-23 08:00**: with the light sensor masked, the old script restarted, after-update saw the login and stayed awake; the owner is now on 45.20261123 (PRs 53–56).

## Nightly update ignores Steam (silverpanda PR 52, merged 2026-11-22)

`nightly-update.sh` restarts on battery above 20% (PR 53, was 50%; 2026-11-23 skipped at 25%) and skips `steam` / `com.valvesoftware.Steam` like Assistant when deciding whether apps are open (owner doesn't mind it closing). A game launched outside Steam still counts. PR 54: a unit counts as open only while its cgroup runs something besides GNOME helpers (gsd-*, gnome-shell, gnome-session*, xdg-*, at-spi*, dbus-*); a G4Music scope left holding only gsd-media-keys (started by a media key) blocked every run until relogin. Owner's test 2026-11-23 with a WakeSystem on-calendar timer: battery and lock checks passed, only that stale scope stopped it. PR 55: 07:37 test woke from sleep fine and passed every check, then systemd refused the reboot ("Transaction for reboot.target/start is destructive": silverpanda-als-after-sleep.service start job queued from the same wake). Now: waits up to 15 min until no other systemd jobs and no rpm-ostree transaction (covers the light sensor, the Flatpak/rpm-ostree timers and fstrim catching up at wake), skips on shutdown block inhibitors, checks battery/lock/apps after the wait, retries a refused reboot once, removes the marker if it still fails. `nightly-update.sh --check` = dry run, prints every check then the verdict (PR 56; owner ran it: all checks right, blocked only by use + Ptyxis). TPM unlock is PCR 7 only, so image updates don't prompt for the disk password. **Still to see: one real restart through the script** (needs the image with PR 53–55). Leftover on the laptop: test timers nightly-test3/4 (gone after a reboot).

## Screen flicker while changing brightness (2026-11-21): owner chose to live with it

Flicker when dragging the brightness slider (mouse or scroll), also before Auto Brightness. Ruled out: the extension, GNOME (a direct logind ramp flickers too), ABM (`panel_power_savings` 0 still flickers; tuned sets 3 in powersave, 1 balanced-battery), PSR/Panel Replay (both inactive). Evidence points at DCN 3.5 idle power states (IPS): debugfs backlight PWM reads 0 except right after each write, and a `dc_dmub_srv_apply_idle_power_optimizations` WARNING in the kernel log. Untested options (each needs a karg + reboot): `amdgpu.dcdebugmask=0x1000` (IPS only when displays are off), `0x800` (IPS off, may hurt s0ix), `0x40000` (custom brightness curve off). Owner declined for now. Also parked: silverpanda branch `claude/brightness-no-flicker` (one backlight write per slider step with Auto Brightness on), unmerged, untested.

## Light sensor after sleep (silverpanda PR 49, 2026-11-21)

After an s2idle suspend the sensor read 0 for good; restarting iio-sensor-proxy didn't help, unbind/bind of `i2c-FRMW0005:00` (i2c_hid_acpi) did (0 → 21 lux) and Auto Brightness re-claimed by itself. PR 49 adds `silverpanda-als-after-sleep.service` (after every sleep) + `als-after-sleep.sh`. Owner ran the script as-is: works (21 lux). **Merged 2026-11-21.** Next: after the next image and a real suspend, check `journalctl -b -u silverpanda-als-after-sleep` and that the sensor reads > 0.

## Auto Brightness (silverpanda PR 47, 2026-11-21) — tested, works

Extension `auto-brightness@silverpanda.wiertless.github.io`: computes GNOME Shell's auto-brightness target (intercepts `Main.brightnessManager.autoBrightnessTarget`, so gsd-power's updates are kept but not applied) while Settings → Power → Automatic Screen Brightness is on. Curve = 7 draggable points at 0/10/50/150/500/1500/3500 lux (monotone cubic over log lux; default 3/8/15/25/50/80/100% from measured preferences), `response` glide, slider = GNOME's shift with scaling only near 0/1 (keys never dead). Keyboard backlight with its own switch (on below 70% of `keyboard-off-lux`, off above; `set property`/`external` changes are the user's and learned, gsd's own restores get overridden). Quick Settings tile with a keyboard switch and Settings. Polish translation (po/pl.po → locale/, compiled .mo committed). **Owner tested after relog: all works.** Then added (77576af, untested, needs a relog): noise filter (a reading counts only if >12% and >=3 lux from the last one used). **Merged 2026-11-21 (81f1812), in image 45.20261121.2; the test copy in ~/.local was deleted, the system copy loads from the next login.** Was next: once the image has it, delete the test copy `~/.local/share/gnome-shell/extensions/auto-brightness@silverpanda.wiertless.github.io` (it shadows the image's). The owner's settings stay in dconf.

## Eval state (2026-11-18 evening)

- 160 families (345 prompts), `eval_verdict.py` heads every summary (sign test per family). `--shard K/N`, workflow input `shards` = 3, 4 or 6.
- **Routing verdict (run 44372794027, repeat 3):** `examples` +0.5 points (95% range −1.3 to +2.3) — no real difference; keep baseline routing, `examples` stays only as a variant.
- **Cue-words verdict (44403890158):** overall +1.0 (undecided), but on the target families ciszej 8/9 vs 3/9, "co teraz leci" 4/6 vs 0/6, nothing nearby worse, 18% faster → kept, released in 0.30.0. Was: cue-words eval 44403890158 (repeat 3, 6 shards): `baseline` = new descriptions with Polish cues, `pre-cues` = old ones. pre-cues WORSE → keep the cues and release 0.30.0; no difference → revert them.
- Run over 160: `examples` +1.9 points vs baseline, UNDECIDED (p 0.24), same direction as the 48-family run. **Decisive run 44372794027** (repeat 3, 6 shards, made before the description fixes below): if BETTER, make `route_by_examples` the default and release 0.30.0.
- No-tool fixes in progress: Polish cue words in set_volume/set_brightness/media_control/open_file descriptions, media keywords ("leci", "playing", "utwór"), eval failures now include the reply. Local NPU check: "ścisz trochę" and "a bit quieter" pass, "ciszej proszę" now calls set_volume (value still off). Next: rerun these families locally (`assistantd --eval --url http://127.0.0.1:8002 --model gemma4-it:e2b --only NAME`, ~4 s/prompt on the NPU), then a CI eval of baseline before/after.

## Parked (owner, 2026-11-22): a bigger squeezed model on the iGPU as the smarter model

Prompted by github.com/ShinMegamiBoson/lookatmyquant (importance-based 1–4-bit "quantization compiler"; MLX/Apple-only, quantizer needs H100/B200, no GGUF published). Same idea available as Unsloth Dynamic GGUFs. The iGPU (Radeon 840M, gfx1152) can use at most ~7.4 GB (Lemonade system-info: 0.5 GB VRAM + 7.4 GB GTT), so 27B at 2 bits (7.2 GB) doesn't fit. Pick when resumed: `unsloth/gemma-4-12b-it-GGUF:gemma-4-12b-it-UD-Q3_K_XL.gguf` (6.02 GB, has an MTP draft head; Lemonade lists a Gemma-4-12B MTP recipe), alt. UD-IQ3_XXS 4.64 GB. Plan: pull as `user.` model (Lemonade keeps downloading after the client disconnects; restart lemond to cancel), measure load/memory/prefill/decode vs qwen3.5:9b on the NPU, then CI eval of both. A started download was cancelled and cleaned up.

NPU alternatives checked (2026-11-22): FastFlowLM (ROCm project since 1.0; orchestration MIT, NPU kernels closed binaries) is the only practical runtime; Ryzen AI Software 1.8 has a Linux guide (OGA) but NPU LLMs on Linux unconfirmed; IRON/MLIR-AIE open toolchain (Llama 3.2 1B example, research up to Qwen3-4B); FastFlow-SDK (early access) = IRON tooling for NPU programs. Custom weights on FLM: NPU-Forge (HF LJTSG/npu-forge) reports LoRA-tuned Llama-3.2-1B converted to Q4NX; installed FLM has `--model-extension` manifests — relevant to the merged-LoRA idea.

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
