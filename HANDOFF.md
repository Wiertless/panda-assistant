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

- 0.26–0.28 released on GitHub: search_everything, speed line under answers, slim pop-up bar (spring damping 0.425), clipboard extension auto-enable. **0.28.0 is not pinned in silverpanda yet** (silverpanda pins 0.25.0).
- On main, unreleased: mail dates in local time with weekday; `get_weather` (Open-Meteo, web group); cards for weather/events/reminders (`tools::CARDS`, hint via `next_step`); one prompt line on fitting answer length. Tests pass; the live model test (does it answer in one sentence beside the card?) was interrupted, so do it next, then release 0.29.0 and pin.

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
4. **Answers that fit**: short for quick questions, cards for weather/events/timers.
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
