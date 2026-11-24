#!/usr/bin/env bash
# panda-lora.zip -> merged weights -> FLM .q4nx in a separate model root -> served on port 8092.
#   make_npu_model.sh ~/Pobrane/panda-lora.zip NAME
set -euo pipefail
HERE=$(cd "$(dirname "$0")" && pwd)
ZIP=$1; NAME=${2:-ft1}
C=$HOME/.cache/flm-convert; PY=$C/venv/bin/python
W=$C/ft/$NAME; mkdir -p "$W"
rm -rf "$W/adapter"; mkdir -p "$W/adapter"
unzip -q -o "$ZIP" -d "$W/adapter-zip"
A=$(dirname "$(find "$W/adapter-zip" -name adapter_config.json | head -1)")
cp "$A"/adapter_config.json "$A"/adapter_model.safetensors "$W/adapter/"
$PY -I "$HERE/merge.py" "$C/src/gemma-4-E2B-it/model.safetensors" "$W/adapter" "$W/merged/model.safetensors"
cp "$C/src/gemma-4-E2B-it/config.json" "$W/merged/"
ROOT=$C/ft-root-$NAME/models/Gemma4-E2B-IT-NPU2; mkdir -p "$ROOT"
for f in "$HOME"/.config/flm/models/Gemma4-E2B-IT-NPU2/*; do
  [ "$(basename "$f")" = model.q4nx ] || cp --reflink=auto "$f" "$ROOT/"
done
$PY -I "$HERE/../flm-convert/convert.py" "$W/merged" "$ROOT/model.q4nx"
rm -rf "$W/merged" "$W/adapter-zip"
echo "ready: FLM_MODEL_PATH=$C/ft-root-$NAME flm serve gemma4-it:e2b --ctx-len 16384 --port 8092"
