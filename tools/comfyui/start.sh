#!/usr/bin/env bash
# ローカル ComfyUI の起動(macOS)。docs/08-local-comfyui.md 参照。
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
DEST="$ROOT/comfyui"

[ -x "$DEST/venv/bin/python" ] && [ -f "$DEST/ComfyUI/main.py" ] || {
  echo "ERROR: 未セットアップです。先に npm run comfyui:setup を実行してください" >&2
  exit 1
}

export PYTORCH_ENABLE_MPS_FALLBACK=1
exec "$DEST/venv/bin/python" "$DEST/ComfyUI/main.py" \
  --listen 127.0.0.1 \
  --port "${COMFYUI_PORT:-8188}" \
  --base-directory "$DEST/data" \
  --disable-dynamic-vram \
  --disable-auto-launch
