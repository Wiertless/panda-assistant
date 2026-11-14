# Panda

A local AI assistant for GNOME that runs on an AMD Ryzen AI NPU through [FastFlowLM](https://github.com/FastFlowLM/FastFlowLM). Nothing leaves the computer.

- **Chat window** built with GTK 4 and libadwaita, following the GNOME HIG: model as the window subtitle, model picker, Keyboard Shortcuts and About in the primary menu, toasts, a banner for chat-only models, and a notification when an answer finishes in the background.
- **GNOME search:** type anything (3+ characters) in the overview and pick **Ask: …** to send it to Panda. The search provider only echoes your text back; the model runs only when you pick the result.
- **Desktop actions** through the model's tool calls:
  - **Web:** search (DuckDuckGo, falling back to Wikipedia's API) and read pages, with results trimmed to fit small contexts.
  - **Open:** apps, files in your home folder, web pages and pages of GNOME Settings. Kinds of apps ("email", "files", "browser", "calendar", "music", "photos", "text editor", "terminal"…) open your default app for them, as set in GNOME Settings → Apps → Default Apps, and apps also match on their generic name and keywords.
  - **Read:** battery, system info, time and current settings.
  - **Change directly:** dark mode, Night Light, power mode, Do Not Disturb, volume, mute, brightness and the clipboard.
  - **Ask first:** Wi-Fi and Bluetooth (they can disconnect you), and any other GNOME desktop setting (`find_settings` / `get_setting` / `change_setting`). Changes are limited to `org.gnome.desktop.*`, settings-daemon plugins, mutter, shell keybindings and Files. Extension, lockdown and screensaver keys are blocked.
  - Nothing runs through a shell, and every argument is validated.
- **Knows the computer:** each chat's system prompt names the laptop, CPU, memory (total and free), NPU, OS, battery and date, so it doesn't need a tool call for those.

## Models

Panda lists the models FastFlowLM has downloaded (`flm list`). Only models labelled **tool-calling** can take actions; others are marked "chat only". Flash models are single-turn and can't use tools. The default is `gemma4-it:e2b`; the choice is saved in `~/.config/panda-assistant/config.json`.

## Smarter model

The fast model can hand a request it can't do well to a bigger local model by calling `ask_smarter_model`. The bigger model gets the conversation and the same desktop actions, and its reply is the answer, so there's no second model swap. FLM keeps one model loaded, so a hand-off takes a few seconds while it loads.

**Preferences → Smarter Model** picks it. **Automatic** chooses the biggest downloaded tool-calling model that should fit in memory (about 0.75 × RAM in billions of parameters: 12B on 16 GB). It can also be a specific model, or **Off**.

## Memory

The model only takes memory while it's loaded, which the window subtitle shows (**Loaded** / **Not loaded**).

- **Load on demand:** typing in the message field starts `flm serve` in the background, so it's usually ready by the time you press Enter.
- **Unload when idle:** after 5 minutes without use (1, 5, 15, 30 minutes or never, in Preferences) Panda stops the server it started. **Unload Model** in the main menu does it immediately. Reloading is quick while Linux still has the model file in its page cache.
- **Short context:** the server starts with an 8K-token context (4K–32K in Preferences) instead of the model's default (64K for Gemma 4). FLM sizes its buffers for the context length, so this is the biggest saving.
- **Prefix cache:** for Qwen 3.5/3.6, the prompt cache is capped at 1 GiB instead of FLM's 4 GiB.

A server you started yourself is used as it is and never stopped. Set `PANDA_FLM_URL` to use another address.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| Ctrl+N | New chat |
| Ctrl+L | Focus the message field |
| Esc | Stop the answer |
| Ctrl+, | Preferences |
| Ctrl+? | Keyboard shortcuts |
| Ctrl+W / Ctrl+Q | Close / quit |

## Install

Requires Python 3, PyGObject, GTK 4, libadwaita (1.4+), and FastFlowLM. `wpctl` and `nmcli` are used for volume and Wi-Fi.

```sh
sudo ./install.sh /usr      # or: make install PREFIX=/usr
```

[SilverPanda](https://github.com/Wiertless/silverpanda) installs a pinned release during its image build.

## Development

```sh
make test                                     # unit tests, fake FLM server
PYTHONPATH=src python3 -m panda_assistant     # run from the checkout
```

Releases: bump `VERSION` in `src/panda_assistant/__init__.py` and the AppStream `<release>`, then run the **ci** workflow on `main` (Actions → ci → Run workflow) or push a `vX.Y.Z` tag. CI tests, builds `panda-assistant-X.Y.Z.tar.gz` with a `.sha256`, and publishes a GitHub release.

## License

[MIT](LICENSE).
