# Assistant (panda-assistant)

Shown simply as **Assistant** on the desktop. A local AI assistant for GNOME that runs on an AMD Ryzen AI NPU through [FastFlowLM](https://github.com/FastFlowLM/FastFlowLM). Nothing leaves the computer.

- **Chat window** built with GTK 4 and libadwaita, following the GNOME HIG: model as the window subtitle, model picker, Keyboard Shortcuts and About in the primary menu, toasts, a banner for chat-only models, and a notification when an answer finishes in the background.
- **GNOME search:** type anything (3+ characters) in the overview and pick **Ask: …**. `assistantd` serves the search itself. If the window is open, the question goes there. Otherwise the service answers on its own and posts the answer as a notification under the app's name (**Open Chat** opens it), so no window process starts. A question after 30 quiet minutes starts a new chat. If an action needs your OK, the notification's **Review** opens the chat with the question waiting. Search also lists matching skills (**Run skill: …**) and earlier chats. Results come from files only; the model runs only when you pick one.
- **Queue messages:** press Enter while an answer is still coming and the message waits, dimmed and marked **Queued**, then goes out when the answer ends (several go one after another). Its undo button puts it back in the text box. **Stop**, or an answer that fails, puts all queued messages back in the text box instead of sending them.
- **Pop-up (Super+Space on SilverPanda):** `panda-assistant --popup` opens a slim floating bar, like the app's message box. Ask, and the answer springs open under it; Esc closes it, and pressing the shortcut again closes it too. What you were looking at goes along with the question: the app and window title you were in and any selected text (from the Assistant Clipboard extension), so "summarize this", "reply to this" or "what does this error mean" work without copying anything. A follow-up, or an action that asks first, moves the chat to the full window. Selected text counts as untrusted, so actions that send or change something ask first.
- **App icon menu:** right-click the icon in the dash for **New Chat**, **Look at My Screen** and **Unload Model**.
- **Gestures (libadwaita):** on narrow windows the chat list and the conversation are two pages; swipe right with two fingers (or from the edge on a touchscreen) to go back to the list. Pinch, Ctrl+scroll or Ctrl+plus/minus/0 change the conversation's text size.
- **Desktop actions** through the model's tool calls:
  - **Web:** search (DuckDuckGo, falling back to Wikipedia's API) and read pages, with results trimmed to fit small contexts.
  - **Open:** apps, files in your home folder, web pages and pages of GNOME Settings. Kinds of apps ("email", "files", "browser", "calendar", "music", "photos", "text editor", "terminal"…) open your default app for them, as set in GNOME Settings → Apps → Default Apps, and apps also match on their generic name and keywords.
  - **Files:** list folders, find files by name (newest first), read text files. Paths may be absolute, start with `~`, be relative to home, or start with a folder name such as "Downloads", which maps to the real folder in any language (e.g. `~/Pobrane`). Everything stays inside the home folder, and hidden files and folders (`.ssh`, keyrings, browser profiles) are off-limits, so a web page can't talk Panda into reading or leaking them.
  - **On screen:** `read_window` reads the text of the focused window (a web page, document or email) and anything selected in it, through the accessibility tree. That suits small models better than a screenshot. When you ask from the assistant's own window, it steps aside for a moment so the window behind becomes the focused one. GTK, GNOME and Firefox apps share their text; some Electron and Chromium apps only do with accessibility turned on.
  - **Personal context**, read on demand and never copied:
    - `search_file_contents` searches inside your files with GNOME's own index (LocalSearch, the one Files and the overview use), including PDFs and office documents, inside your home folder and never hidden files.
    - `read_document` reads the text LocalSearch extracted from a PDF or document (or uses `pdftotext`).
    - `get_events` reads your calendars and `find_contact` your address books, both from Evolution Data Server, so everything set up in Online Accounts, GNOME Calendar or Contacts is included.
  - **App intents:** `show_in_files` opens a file's folder with it selected, using the default file manager for the freedesktop `org.freedesktop.FileManager1` intent (from `intentapps.list` and apps' `Implements=` keys, per the Application Intents Specification); "files" opens that same app. `list_app_actions` / `run_app_action` use the actions apps publish (the same as their right-click menu in the dash: "New Private Window", "Compose New Message"…). `media_control` plays, pauses, skips or tells what's playing in any MPRIS player (Spotify, Firefox, Celluloid…).
    - `search_apps` asks every app's own search at once, the way the overview does (`org.gnome.Shell.SearchProvider2`): Calculator for sums and unit conversions, Characters for emoji, Weather, Contacts, Settings panels, Files and more. `open_search_result` opens a result in its app. Providers you turned off in Settings → Search are skipped.
    - `open_terminal` opens your terminal in a folder through the freedesktop Terminal intent (`org.freedesktop.Terminal1`), falling back to `xdg-terminal-exec`, Ptyxis, Console or GNOME Terminal while no terminal implements it yet.
  - **Networks and Bluetooth:** connect to saved Wi-Fi networks, wired connections and VPNs, and to paired Bluetooth devices by everyday name ("connect my AirPods"). Bluetooth is turned on first if needed. Disconnecting a network asks first.
  - **Screen and images:** with a model that can see (Gemma 4, Qwen 3.5, Qwen3-VL…), Panda can take a screenshot through the desktop portal and look at image files. Attach pictures to a message with the paperclip, by pasting, or by dropping them on the window. Pictures are downscaled to 1024 px, kept for one turn only, and never saved with the chat. `save_screenshot` keeps the screenshot in your Screenshots folder instead and returns its path, so "take a screenshot and send it to my phone" works. If the chosen model can't see, the turn goes to one that can.
  - **Commands:** `run_command` runs a shell command in the `zed` toolbox (created on first use by SilverPanda's `zed-toolbox-setup`), always after you press **Allow**, with the exact command shown. Commands time out after 2 minutes. The toolbox shares your home folder, so the Allow card is the safety net: read it.
  - **Taildrop:** list your Tailscale devices and send a file to one, or a note (`send_text`: an answer, a list, an address) as a text file the phone opens. It doesn't ask, since Taildrop only reaches your own devices. Needs `sudo tailscale set --operator=$USER` once.
  - **Read:** battery, system info, time and current settings.
  - **Change directly:** dark mode, Night Light, power mode, Do Not Disturb, volume, mute, brightness, Wi-Fi, Bluetooth, the clipboard, and any other GNOME desktop setting (`find_settings` / `get_setting` / `change_setting`). Setting changes are limited to `org.gnome.desktop.*`, settings-daemon plugins, mutter, shell keybindings and Files; extension, lockdown and screensaver keys are blocked.
  - **Ask first:** only what can't be undone or sends something out: commands and other apps' actions (MCP). The card has **Always Allow** (manage the list in Preferences), except for commands, which always ask.
  - Nothing runs through a shell, and every argument is validated.
  - After the assistant reads text from outside in a turn (a web page, a document, a calendar invite, the clipboard, another app's tool), anything that sends, keeps or quietly changes something (sending a file or note, opening a link, remembering, changing a setting, MCP tools) asks first, so text written to steer the model can't act on its own.
  - Settings that run commands or let others in are out of reach: custom keyboard shortcuts, default applications, remote desktop, sharing, lockdown and the screensaver, and any per-app (relocatable) schema.
  - Only Assistant's own window can answer an Allow card, and a turn's text, tool results and copied text go only to the window following the chat, not to everything on the session bus.
- **Connects the dots:** the model is told to work out what you mean from the chat, what it remembers and what's on the computer ("my phone" is the device you told it about), and to pass one tool's result to the next. When something isn't found, the error lists what is there: similar file names in the folder, close app names, settings keys, devices with their OS, or tool names. The model is told to fix the call and retry (up to 8 rounds a turn) before telling you it failed.
- **Restart:** **Restart Assistant** in the main menu (also the **Restart** button on error banners) stops the current answer, connected apps and every `flm serve` you run (not only the one the service started: one left over from a crash keeps the NPU and the port), then restarts the service and reopens the chat from disk. If the service hangs, the window kills it after 20 seconds and does the same itself.
- **Recovering by itself:** when a question fails because FastFlowLM broke (connection refused, a server error, a timeout), the service stops every `flm serve`, starts it again and asks once more from where the turn began. If that fails too, the error shows the end of FastFlowLM's log, and says so when the log points at the NPU driver (which only reloading the driver, `sudo modprobe -r amdxdna && sudo modprobe amdxdna`, or a reboot fixes).
- **Research:** "research which e-reader is best for PDFs", "zbadaj, który czytnik e-booków jest najlepszy do PDF-ów, zapisz jako PDF". `research` splits the work into small jobs a 2B model does well, each in a fresh context. The model plans a report of 4–6 sections with a search or two for each; the harness runs them, picks up to 12 pages (three per section, no page twice, at most two per site) and fetches them at once; the model takes a few lines of notes from each page. Then the **smarter model**, when one is set, writes each section from its numbered notes (citing them like [2]), a summary and a conclusion, and the sources are listed at the end. The spinner shows each step. "Save it as a PDF" (or `md`, `txt`) puts the report in `Documents/Research` and the chat gets its summary and the path; otherwise the whole report is shown as written. `depth: quick` keeps the short answer from five pages. Without a smarter model the same model writes it. Pages are treated as untrusted text, like any web page.
- **Files, like a coding agent:** besides finding and reading files, the assistant can write new text files, edit part of a file (replacing exact text, as Claude Code does), rename and move files, make folders, move files to the Trash and search inside text files. All of it stays in your home folder and away from hidden files, every change can be undone, and changing or removing something asks first. It reads PDFs, Word, Excel, PowerPoint, LibreOffice files, EPUB, RTF, HTML, e-mails (`.eml`), notebooks and any text or source file directly, a part at a time for long ones.
- **Music from your library:** "play Bohemian Rhapsody in Gapless", "puść album Random Access Memories", "put on some Queen". `play_music` finds the tracks in GNOME's file index (title, artist, album), or in your Music folder when the index has none, and opens them in the app you name or your default player: one song by title, or a whole album or artist in order. When nothing matches it says so instead of searching the web.
- **Selected text, anywhere:** select text in any app, then ask, from the window or from GNOME search: "proofread the selected text", "make this shorter", "popraw zaznaczony tekst", "translate it to English". `rewrite_selection` reads the selection through the Assistant Clipboard extension, rewrites it in a fresh context with no tools, and puts the result on the clipboard (through the extension when the window is closed) with a notification; paste it with Ctrl+V.
- **Questions about documents:** "how much is the deposit in my lease?", "summarize this PDF", "streść umowę": `ask_document` cuts the document (PDFs up to 200 pages, office documents from GNOME's index, text files) into parts of about 1.2K tokens, picks the three that best match the question, and has the model answer from those alone in a fresh context, so a long document never floods the chat. A summary reads four parts spread over the document and combines them.
- **Search everything:** "when's my flight to Lisbon?", "what did the landlord say about the deposit?". When the answer is somewhere in your things but not clearly in one place, `search_everything` searches the calendar (a month back, a year ahead), mail, file names, what's inside your documents and contacts at once, in about two seconds, and returns what each found.
- **Weather and cards:** "jaka jutro pogoda w Toruniu?" `get_weather` asks Open-Meteo (free, no account; only the place name leaves the computer) for now and the next three days. Weather, the day's events and your reminders show as a small card in the chat, and the answer next to it stays one sentence.
- **Mail, whatever app you use:** "what did Ola write about the lease?", "czy przyszła faktura z Orange?". `search_mail` searches every mail account in **Settings → Online Accounts** over IMAP (Gmail with its sign-in token, other servers with their password, both from GNOME's keyring), so it doesn't matter whether you read mail in Convey, Thunderbird, Evolution, Geary or a browser. It shows the newest matches with sender, subject, date and the start of the text; `read_mail` reads one whole. It only reads: messages aren't marked as read. Mail counts as untrusted text, so anything that sends or changes something afterwards asks first.
- **Calendar and email:** "add lunch with Ola on Friday at 1" puts the event in your default calendar through Evolution Data Server (the one GNOME Calendar and Online Accounts use), falling back to the next calendar if that one is read-only; Undo removes it. "Email Jakub that I'm running late" opens a ready draft in your mail app (a `mailto:` link; a name is looked up in your contacts) for you to check and send: nothing is sent by the assistant.
- **Undo:** each change records what it replaced: dark mode, Do Not Disturb, Night Light, any GNOME setting, volume and mute, brightness, Wi-Fi, Bluetooth, power mode, a reminder, something remembered. "Undo that" / "change it back" / "cofnij" puts the newest one back, and so does the **Undo** button on the action's pill in the window. The last 20 changes are kept while the service runs.
- **Reminders and timers:** "remind me in 20 minutes to take the pizza out", "remind me tomorrow at 9 to call mom", "set a 5-minute timer". They're kept in `~/.local/share/panda-assistant/reminders.json` and fire as notifications with **Snooze 10 min** and **Done**, which work without opening the window. The service stays up while any are pending, checks at least once a minute (so they fire right after waking from suspend), and starts at login when some are waiting, so they survive reboots; ones missed while the laptop was off fire then.
- **Clipboard history:** "what was the link I copied earlier?" The **Assistant Clipboard** GNOME Shell extension (installed with the app; SilverPanda turns it on) keeps the last 20 text clips in the shell's memory only, never on disk, skips anything a password manager marks as secret, and forgets it all on lock and logout. Only `assistantd` can read it: the extension checks the caller's executable. On Wayland only the focused app can read the clipboard, which is why it lives in the shell.
- **Activity:** Preferences → Activity lists everything the assistant did, newest first: what, when, how it went, whether you allowed it, and whether it came from search. The last 500 actions are kept in `~/.local/state/panda-assistant/activity.jsonl`.
- **Answer details:** under each answer a small line like "Thought for 4.2 s · 9.8 s, 21 t/s": how long the model reasoned, how long the whole answer took and how fast it wrote (all its model calls together). **Preferences → Answer Details** turns thinking time, tokens and speed on or off.
- **Speed log:** every model call is logged to `~/.local/state/panda-assistant/rounds.jsonl`: prompt tokens, how many FastFlowLM reused from its prompt cache (`cached_tokens`), tokens written, time to the first token and in total, and the tools it called. `jq -s 'map(.cached_tokens / (.prompt_tokens + 0.1)) | add / length' ~/.local/state/panda-assistant/rounds.jsonl` gives the average share reused.
- **Model scores:** `assistantd --eval` runs about 20 scripted requests (English and Polish: settings, a screenshot to your phone, calendar, contacts, recovering from a wrong device name, chit-chat that shouldn't call tools…) with stand-in tool results, and scores whether the model called the right tools with the right arguments and answered right. Nothing runs on the computer. On the laptop it starts FastFlowLM with the model (`--model qwen3.5:4b`); `--url` points it at any OpenAI-compatible server. The **eval** workflow scores several models on GitHub with llama.cpp on the CPU whenever the prompt, tools or scenarios change, and puts the table in the run summary. Scenarios live in `core/eval/scenarios.json`. Variants (`--variant`, `core/eval/variants.json`) switch the harness settings, the system prompt or tool descriptions; run the workflow by hand with `["baseline", "no-harness"]` to see what the harness adds, or with a variant of your own to try a prompt change. `models` narrows a run to some models and `repeat` runs each scenario several times for a pass rate. The report also counts scenarios passed in all their wordings, prompts whose expected tool routing didn't even offer (a router miss, not the model's), and what each variant fixed and broke against the baseline. Scenarios carry capability tags (single, chain, near-tool, enum, no-tool, ask, multi-turn, memory, recover, polish…) and paraphrases, each judged on its own, and the report scores each capability.
- **Keeping small models on track:** the harness makes up for what a 2–4B model gets wrong:
  - Requests that carry tools are sampled at temperature 0.2, so the tool and its arguments come out the same way each time.
  - Small slips in a call are fixed from the tool's own schema before it runs: "30%" for a number, "yes" for a boolean, "Next" or "prev" for an option, a near-miss argument name, single-quoted JSON or a trailing comma.
  - A reply that says it did something ("Done, I sent it") when no tool ran is taken back, and the model is asked to actually do it.
  - An exact repeat of a call that just failed isn't run again; the model is told to change course. After three failed calls in a turn, or when it runs out of steps, the smarter model takes over the request.
  - Each active tool group comes with one worked example in the prompt, and lookups (find_files, list_devices, find_settings…) end with what to do next.
  - Answering is a tool too: the first round of each turn must call one, and `reply(text)` is among them, so talking instead of acting is a choice among tools rather than the easy way out (as in smolagents' `final_answer`, Apple's required tool mode and Rasa's `ChitChat`). On Gemma 4 E2B in the eval it fixed 3 prompts and broke none (88/91 against 85/91).
  - A request to do something that stopped after a lookup (found the setting, listed the devices) is asked again with only the tools that finish it and a tool call required (the server's constrained decoding), plus `ask_user` for when several things fit.
  - Tool calls a model writes into its reply as text (Gemma's `call:name{…}`, a `tool_code` block, or JSON) are run as calls.
  - Rounds that offer tools ask FastFlowLM not to think first (`think: false`), so a Qwen 3.5 model doesn't spend seconds reasoning before each call; plain answers and the bigger model on a handoff still think.
  - Errors that retrying can't fix (no music player running, an app or service that isn't installed) end the turn at once with the reason, instead of the model trying the same call again.
  - Older FastFlowLM versions that reject `think`, `tool_choice`, `--embed` or the prefix-cache flags get the request again without them.
  - Tool groups unused for two turns drop out of the request, so the tool list doesn't only grow.
  - With FastFlowLM's embed-gemma:300m installed, it runs next to the chat model and tool groups are also picked by meaning, not only by keywords.
- **Tool routing:** every tool's description costs context on every message (all of them: about 3.6K tokens of an 8K context). So a core set always goes along (about 1.2K tokens) and the other groups (settings, files, devices, calendar, apps, web, commands…) join when your message mentions them, in English or Polish, or when the model asks for them with `more_tools`. A group that joined stays for the rest of the chat. The bigger model on a handoff gets every tool.
- **Knows the computer:** each chat's system prompt names the laptop, CPU, memory (total and free), NPU, OS, battery and date, so it doesn't need a tool call for those. Like Home Assistant's list of a home's devices, it also names your paired Bluetooth devices, your other devices on Tailscale and your saved networks, so "my headphones", "my phone" or "the office Wi-Fi" resolve without a lookup.

## Memory, skills and history

Everything lives in `~/.local/share/panda-assistant` as plain files you can read, edit or back up.

- **Memory** (`memory.md`): tell Panda something lasting ("remember I prefer metric units") and it's added to every new chat's prompt. See, add and forget facts in **Preferences → Memory**, or turn memory off.
- **Skills** (`skills/*.md`): saved workflows. Ask "save that as a skill called morning" or write a Markdown file: a `# Title`, an optional one-line description, then steps. Run one by typing `/morning` (anything after the name is passed along), or just ask for it. Manage them in **Preferences → Skills**.
- **Long chats:** when a chat fills about three quarters of the context, the oldest turns are summarized by the model (facts, names, numbers, decisions, open tasks) and replaced by that summary, which is saved with the chat. This happens right after an answer is shown, so the next question doesn't wait for it. The full conversation stays on screen and on disk.
- **Chat history** (`chats/*.json`): every chat is saved after each answer. **Ctrl+F** searches the list. The last 200 chats are kept.

## Other apps (MCP)

Panda is a [Model Context Protocol](https://modelcontextprotocol.io) client, so it can use tools that other apps and services expose. Add servers in **Preferences → Connections**, or in `~/.config/panda-assistant/mcp.json` in the usual format:

```json
{
  "mcpServers": {
    "memory": {"command": "npx", "args": ["-y", "@modelcontextprotocol/server-memory"]},
    "home": {"url": "http://homeassistant.local:8123/api/mcp", "headers": {"Authorization": "Bearer …"}}
  }
}
```

- Local servers (stdio) and remote ones (Streamable HTTP) work. Servers start with your first message and stop when the model unloads. Logs go to `~/.local/state/panda-assistant/mcp-<name>.log`.
- Tools a server marks read-only run straight away; everything else asks first. Add `"trust": true` to a server to skip asking, or `"disabled": true` to turn it off.
- Each tool's description takes room in the small model's context, so at most 16 MCP tools are offered at once. Turn off servers you don't need.
- On Silverblue, servers that need Node or Python packages can run inside a toolbox: `"command": "toolbox", "args": ["run", "-c", "zed", "npx", "-y", "…"]`.

## Models

Panda lists the models FastFlowLM has downloaded (`flm list`). Only models labelled **tool-calling** can take actions; others are marked "chat only". Flash models are single-turn and can't use tools. Until you pick one, it uses the first of `qwen3.5:4b`, `gemma4-it:e4b`, `gemma4-it:e2b` and `qwen3.5:2b` that's installed (Qwen 3.5 4B scores best in the eval and keeps up with Gemma 4 E2B on the NPU); your choice is saved in `~/.config/panda-assistant/config.json`.

## Lemonade Server

When [Lemonade Server](https://lemonade-server.ai) is installed (its `lemond` user service; SilverPanda includes it), Panda uses it instead of starting `flm serve` itself. Lemonade runs FastFlowLM on the NPU and llama.cpp on the graphics chip. Panda starts the service when it needs a model and stops it again when it unloads, if it started it. Models keep their FLM names; models Lemonade runs on the graphics chip (downloaded GGUF chat models, `lemonade pull Qwen3.5-9B-GGUF`) are listed as "… (graphics chip)". If Lemonade won't start, Panda falls back to FastFlowLM directly. `PANDA_BACKEND=flm` or `=lemonade` forces one.

A graphics-chip model runs next to the NPU model instead of replacing it, so a hand-off doesn't reload the small model. It isn't faster: the Radeon 840M reads a 9B model's 6 GB of weights at about the speed of the NPU, prefills prompts more slowly, and draws far more power. So nothing picks one automatically; choose one in **Preferences → Smarter Model** for heavy jobs. It's unloaded a minute after use, keeping the NPU model loaded.

## Smarter model

The fast model can hand a request it can't do well to a bigger local model by calling `ask_smarter_model`. The bigger model gets the conversation and the same desktop actions, and its reply is the answer, so there's no second model swap. FLM keeps one model loaded, so a hand-off takes a few seconds while it loads.

**Preferences → Smarter Model** picks it. **Automatic** chooses the biggest downloaded tool-calling model that should fit in memory (about 0.75 × RAM in billions of parameters: 12B on 16 GB). It can also be a specific model, or **Off**.

## Memory

The model only takes memory while it's loaded, which the window subtitle shows (**Loaded** / **Not loaded**).

- **Load on demand:** typing in the message field starts `flm serve` in the background, so it's usually ready by the time you press Enter.
- **Keep the small model, drop the big one:** the default model stays loaded so answers start right away (set **Unload Model** in Preferences to free it after 1–30 idle minutes). A bigger model loaded for a hand-off or a picture is dropped a minute after it's done: FastFlowLM restarts with just the default model, which frees the bigger model's weights and context. **Unload Model** in the main menu frees everything immediately.
- **Short context:** the server starts with an 8K-token context (4K–32K in Preferences) instead of the model's default (64K for Gemma 4). FLM sizes its buffers for the context length, so this is the biggest saving.
- **Prefix cache:** for Qwen 3.5/3.6, the prompt cache is capped at 1 GiB instead of FLM's 4 GiB.

A server you started yourself is used as it is and never stopped. Set `PANDA_FLM_URL` to use another address.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| Ctrl+N | New chat |
| Ctrl+L | Focus the message field |
| Ctrl+F | Search chats |
| Ctrl+Shift+A | Attach an image |
| Ctrl+plus / Ctrl+minus / Ctrl+0 | Text size |
| Esc | Stop the answer |
| Ctrl+, | Preferences |
| Ctrl+? | Keyboard shortcuts |
| Ctrl+W / Ctrl+Q | Close / quit |

## How it fits together

- **`assistantd`** (Rust, `core/`) is the assistant itself. It runs the model through FastFlowLM, runs the agent loop and all tools, connects MCP servers, keeps memory, skills and chats, and decides when to load and unload models. It reads window text over AT-SPI, file contents from LocalSearch (TinySPARQL, loaded at run time), the calendar from GNOME Shell's calendar service and contacts from Evolution Data Server, all through D-Bus or GNOME's own libraries. It is a D-Bus service (`io.github.wiertless.PandaAssistant.Core`) that the session bus starts on first use. It exits after 10 idle minutes with nothing loaded. It holds the model, so the window can come and go without reloading it.
- **The window** (`app/`, Rust with GTK 4 and libadwaita) is a client. It sends questions over D-Bus, shows the answer as it streams back as signals, and answers Allow/Cancel requests. Opening a chat the service is still answering shows the rest of it.

Settings are shared through `~/.config/panda-assistant/config.json`; after changing them, the window tells the service to re-read them.

## Install

Needs GTK 4, libadwaita (1.5+) and FastFlowLM. `wpctl`, `nmcli`, `tailscale` and `toolbox` are used when present. There is no Python. Building needs Rust and the GTK 4, libadwaita, GLib and gdk-pixbuf development files.

```sh
make build                  # release tarballs already include both binaries (core-bin/)
sudo ./install.sh /usr      # or: make install PREFIX=/usr
```

[SilverPanda](https://github.com/Wiertless/silverpanda) installs a pinned release during its image build.

## Development

```sh
make test                   # Rust tests, with a fake FastFlowLM and a fake MCP server
make lint                   # clippy, warnings as errors
cargo run -p panda-assistant
```

The window needs the service on the session bus: install it, or run `target/debug/assistantd` in a terminal first. On systems with libadwaita older than 1.8, build without the `adw-1-8` feature (`make build FEATURES=`). The window then uses a flow box for suggestions and the older shortcuts window.

Releases: bump `version` in `app/Cargo.toml` and `core/Cargo.toml` (CI checks they match) and add the AppStream `<release>`. Then run the **ci** workflow on `main` (Actions → ci → Run workflow) or push a `vX.Y.Z` tag. CI lints, tests and builds in a Fedora container, so the binaries match the libraries in SilverPanda's image. It then publishes `panda-assistant-X.Y.Z.tar.gz` (sources plus `core-bin/`) with a `.sha256` as a GitHub release.

## License

[MIT](LICENSE).
