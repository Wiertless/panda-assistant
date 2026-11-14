# Panda

A local AI assistant for GNOME that runs on an AMD Ryzen AI NPU through [FastFlowLM](https://github.com/FastFlowLM/FastFlowLM). Nothing leaves the computer.

- **Chat window** built with GTK 4 and libadwaita, following the GNOME HIG: model as the window subtitle, model picker, Keyboard Shortcuts and About in the primary menu, toasts, a banner for chat-only models, and a notification when an answer finishes in the background.
- **GNOME search:** type anything (3+ characters) in the overview and pick **Ask: …** to send it to Panda. The search provider only echoes your text back; the model runs only when you pick the result.
- **Desktop actions** through the model's tool calls: open apps, files in your home folder and web pages; read battery, system info, time and settings; copy to the clipboard; switch dark mode, Do Not Disturb, volume, mute and brightness. Turning Wi-Fi or Bluetooth on or off asks first, because it can disconnect you. Nothing runs through a shell, and every argument is validated.

## Models

Panda lists the models FastFlowLM has downloaded (`flm list`). Only models labelled **tool-calling** can take actions; others are marked "chat only". Flash models are single-turn and can't use tools. The default is `gemma4-it:e2b`; the choice is saved in `~/.config/panda-assistant/config.json`.

If nothing is listening on `127.0.0.1:52625`, Panda starts `flm serve <model>` when you send the first message and stops it when you quit. A server you started yourself is left alone. Set `PANDA_FLM_URL` to use another address.

## Keyboard shortcuts

| Shortcut | Action |
| --- | --- |
| Ctrl+N | New chat |
| Ctrl+L | Focus the message field |
| Esc | Stop the answer |
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
