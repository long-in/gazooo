#!/usr/bin/env bash
# ローカル ComfyUI のセットアップ(macOS)。冪等。docs/08-local-comfyui.md 参照。
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$HERE/../.." && pwd)"
# shellcheck source=versions.env
source "$HERE/versions.env"

DEST="$ROOT/comfyui"
APP="$DEST/ComfyUI"
VENV="$DEST/venv"
DATA="$DEST/data"
GGUF_DIR="$DATA/custom_nodes/ComfyUI-GGUF"
IMPACT_DIR="$DATA/custom_nodes/ComfyUI-Impact-Pack"
SUBPACK_DIR="$DATA/custom_nodes/ComfyUI-Impact-Subpack"
AUX_DIR="$DATA/custom_nodes/comfyui_controlnet_aux"
TAGGER_DIR="$DATA/custom_nodes/ComfyUI-WD14-Tagger"

fail() { echo "ERROR: $*" >&2; exit 1; }

[ "$(uname -s)" = "Darwin" ] || fail "setup.sh は macOS 用です(Windows は setup.ps1。npm run comfyui:setup が自動で選びます)"
command -v git >/dev/null || fail "git が見つかりません"
PY="python${PYTHON_VERSION}"
command -v "$PY" >/dev/null || fail "$PY が見つかりません。例: brew install python@${PYTHON_VERSION}"

# 指定した ref(タグまたはコミット)を、浅い履歴で取得して checkout する
checkout_pinned() {
  local dir="$1" url="$2" ref="$3"
  if [ ! -d "$dir/.git" ]; then
    mkdir -p "$dir"
    git -C "$dir" init -q
    git -C "$dir" remote add origin "$url"
  fi
  git -C "$dir" fetch -q --depth 1 origin "$ref"
  git -C "$dir" -c advice.detachedHead=false checkout -q FETCH_HEAD
}

echo "==> ComfyUI ${COMFYUI_TAG}"
checkout_pinned "$APP" "$COMFYUI_REPO" "refs/tags/${COMFYUI_TAG}"

echo "==> ComfyUI-GGUF ${GGUF_NODE_COMMIT}"
mkdir -p "$DATA/custom_nodes" "$DATA/models"
checkout_pinned "$GGUF_DIR" "$GGUF_NODE_REPO" "$GGUF_NODE_COMMIT"
[ "$(git -C "$GGUF_DIR" rev-parse HEAD)" = "$GGUF_NODE_COMMIT" ] || fail "GGUF ノードのコミットが固定値と一致しません"

echo "==> ComfyUI-Impact-Pack ${IMPACT_PACK_COMMIT}"
checkout_pinned "$IMPACT_DIR" "$IMPACT_PACK_REPO" "$IMPACT_PACK_COMMIT"
[ "$(git -C "$IMPACT_DIR" rev-parse HEAD)" = "$IMPACT_PACK_COMMIT" ] || fail "Impact Pack のコミットが固定値と一致しません"

echo "==> ComfyUI-Impact-Subpack ${IMPACT_SUBPACK_COMMIT}"
checkout_pinned "$SUBPACK_DIR" "$IMPACT_SUBPACK_REPO" "$IMPACT_SUBPACK_COMMIT"
[ "$(git -C "$SUBPACK_DIR" rev-parse HEAD)" = "$IMPACT_SUBPACK_COMMIT" ] || fail "Impact Subpack のコミットが固定値と一致しません"

echo "==> comfyui_controlnet_aux ${CONTROLNET_AUX_COMMIT}"
checkout_pinned "$AUX_DIR" "$CONTROLNET_AUX_REPO" "$CONTROLNET_AUX_COMMIT"
[ "$(git -C "$AUX_DIR" rev-parse HEAD)" = "$CONTROLNET_AUX_COMMIT" ] || fail "controlnet_aux のコミットが固定値と一致しません"

echo "==> ComfyUI-WD14-Tagger ${WD14_TAGGER_COMMIT}"
checkout_pinned "$TAGGER_DIR" "$WD14_TAGGER_REPO" "$WD14_TAGGER_COMMIT"
[ "$(git -C "$TAGGER_DIR" rev-parse HEAD)" = "$WD14_TAGGER_COMMIT" ] || fail "WD14 Tagger のコミットが固定値と一致しません"

echo "==> Python venv (${PYTHON_VERSION})"
if [ -x "$VENV/bin/python" ]; then
  have="$("$VENV/bin/python" -c 'import sys; print("%d.%d" % sys.version_info[:2])')"
  [ "$have" = "$PYTHON_VERSION" ] || fail "既存の venv が Python $have です。comfyui/venv を削除して再実行してください"
else
  "$PY" -m venv "$VENV"
fi

PIP=("$VENV/bin/python" -m pip install --disable-pip-version-check)

echo "==> PyTorch ${TORCH_VERSION}"
"${PIP[@]}" "torch==${TORCH_VERSION}" "torchvision==${TORCHVISION_VERSION}" "torchaudio==${TORCHAUDIO_VERSION}"

echo "==> ComfyUI requirements"
"${PIP[@]}" -r "$APP/requirements.txt"

echo "==> ComfyUI-GGUF requirements"
"${PIP[@]}" -r "$GGUF_DIR/requirements.txt"

echo "==> Impact Pack / Subpack requirements"
"${PIP[@]}" -r "$IMPACT_DIR/requirements.txt"
"${PIP[@]}" -r "$SUBPACK_DIR/requirements.txt"

echo "==> controlnet_aux requirements"
"${PIP[@]}" -r "$AUX_DIR/requirements.txt"

"$VENV/bin/python" - <<'PYEOF'
import torch
print(f"torch {torch.__version__} / MPS available: {torch.backends.mps.is_available()}")
PYEOF

echo
echo "完了。起動: npm run comfyui:start"
echo "モデルは comfyui/data/models/ 配下に配置してください(docs/08-local-comfyui.md §5)"
