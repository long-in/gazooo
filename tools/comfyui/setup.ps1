# ローカル ComfyUI のセットアップ(Windows / NVIDIA)。冪等。docs/08-local-comfyui.md 参照。
#Requires -Version 5.1
$ErrorActionPreference = 'Stop'

$Here = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = (Resolve-Path (Join-Path $Here '..\..')).Path

# versions.env を読み込む(KEY=VALUE、# 始まりはコメント)
$V = @{}
Get-Content -Encoding UTF8 (Join-Path $Here 'versions.env') |ForEach-Object {
  if ($_ -match '^\s*([A-Z_]+)\s*=\s*(.+?)\s*$') { $V[$Matches[1]] = $Matches[2] }
}

$Dest    = Join-Path $Root 'comfyui'
$App     = Join-Path $Dest 'ComfyUI'
$Venv    = Join-Path $Dest 'venv'
$Data    = Join-Path $Dest 'data'
$GgufDir = Join-Path $Data 'custom_nodes\ComfyUI-GGUF'
$ImpactDir  = Join-Path $Data 'custom_nodes\ComfyUI-Impact-Pack'
$SubpackDir = Join-Path $Data 'custom_nodes\ComfyUI-Impact-Subpack'
$AuxDir     = Join-Path $Data 'custom_nodes\comfyui_controlnet_aux'
$TaggerDir  = Join-Path $Data 'custom_nodes\ComfyUI-WD14-Tagger'
$VenvPy  = Join-Path $Venv 'Scripts\python.exe'

function Fail($msg) { Write-Error "ERROR: $msg"; exit 1 }

# ネイティブコマンドは $ErrorActionPreference では止まらないため、終了コードを確認する
function Invoke-Checked {
  param([string]$Exe, [string[]]$Arguments)
  & $Exe @Arguments
  if ($LASTEXITCODE -ne 0) { Fail "$Exe $($Arguments -join ' ') が失敗しました (exit $LASTEXITCODE)" }
}

# 指定した ref(タグまたはコミット)を、浅い履歴で取得して checkout する
function Checkout-Pinned([string]$Dir, [string]$Url, [string]$Ref) {
  if (-not (Test-Path (Join-Path $Dir '.git'))) {
    New-Item -ItemType Directory -Force -Path $Dir | Out-Null
    Invoke-Checked git @('-C', $Dir, 'init', '-q')
    Invoke-Checked git @('-C', $Dir, 'remote', 'add', 'origin', $Url)
  }
  Invoke-Checked git @('-C', $Dir, 'fetch', '-q', '--depth', '1', 'origin', $Ref)
  Invoke-Checked git @('-C', $Dir, '-c', 'advice.detachedHead=false', 'checkout', '-q', 'FETCH_HEAD')
}

if (-not (Get-Command git -ErrorAction SilentlyContinue)) { Fail 'git が見つかりません' }

# Python 3.12 を探す(py ランチャー優先)
$PyVer = $V.PYTHON_VERSION
$PyExe = $null
if (Get-Command py -ErrorAction SilentlyContinue) {
  $found = & py "-$PyVer" -c "import sys; print(sys.executable)" 2>$null
  if ($LASTEXITCODE -eq 0 -and $found) { $PyExe = $found.Trim() }
}
if (-not $PyExe -and (Get-Command python -ErrorAction SilentlyContinue)) {
  $have = & python -c "import sys; print('%d.%d' % sys.version_info[:2])" 2>$null
  if ($have -eq $PyVer) { $PyExe = (Get-Command python).Source }
}
if (-not $PyExe) { Fail "Python $PyVer が見つかりません。https://www.python.org/downloads/ から導入してください" }

Write-Host "==> ComfyUI $($V.COMFYUI_TAG)"
Checkout-Pinned $App $V.COMFYUI_REPO "refs/tags/$($V.COMFYUI_TAG)"

Write-Host "==> ComfyUI-GGUF $($V.GGUF_NODE_COMMIT)"
New-Item -ItemType Directory -Force -Path (Join-Path $Data 'custom_nodes'), (Join-Path $Data 'models') | Out-Null
Checkout-Pinned $GgufDir $V.GGUF_NODE_REPO $V.GGUF_NODE_COMMIT
$head = (& git -C $GgufDir rev-parse HEAD).Trim()
if ($head -ne $V.GGUF_NODE_COMMIT) { Fail 'GGUF ノードのコミットが固定値と一致しません' }

Write-Host "==> ComfyUI-Impact-Pack $($V.IMPACT_PACK_COMMIT)"
Checkout-Pinned $ImpactDir $V.IMPACT_PACK_REPO $V.IMPACT_PACK_COMMIT
if ((& git -C $ImpactDir rev-parse HEAD).Trim() -ne $V.IMPACT_PACK_COMMIT) { Fail 'Impact Pack のコミットが固定値と一致しません' }

Write-Host "==> ComfyUI-Impact-Subpack $($V.IMPACT_SUBPACK_COMMIT)"
Checkout-Pinned $SubpackDir $V.IMPACT_SUBPACK_REPO $V.IMPACT_SUBPACK_COMMIT
if ((& git -C $SubpackDir rev-parse HEAD).Trim() -ne $V.IMPACT_SUBPACK_COMMIT) { Fail 'Impact Subpack のコミットが固定値と一致しません' }

Write-Host "==> comfyui_controlnet_aux $($V.CONTROLNET_AUX_COMMIT)"
Checkout-Pinned $AuxDir $V.CONTROLNET_AUX_REPO $V.CONTROLNET_AUX_COMMIT
if ((& git -C $AuxDir rev-parse HEAD).Trim() -ne $V.CONTROLNET_AUX_COMMIT) { Fail 'controlnet_aux のコミットが固定値と一致しません' }

Write-Host "==> ComfyUI-WD14-Tagger $($V.WD14_TAGGER_COMMIT)"
Checkout-Pinned $TaggerDir $V.WD14_TAGGER_REPO $V.WD14_TAGGER_COMMIT
if ((& git -C $TaggerDir rev-parse HEAD).Trim() -ne $V.WD14_TAGGER_COMMIT) { Fail 'WD14 Tagger のコミットが固定値と一致しません' }

Write-Host "==> Python venv ($PyVer)"
if (Test-Path $VenvPy) {
  $have = & $VenvPy -c "import sys; print('%d.%d' % sys.version_info[:2])"
  if ($have -ne $PyVer) { Fail "既存の venv が Python $have です。comfyui\venv を削除して再実行してください" }
} else {
  Invoke-Checked $PyExe @('-m', 'venv', $Venv)
}

$Pip = @('-m', 'pip', 'install', '--disable-pip-version-check')

# CUDA 版 PyTorch。別のインデックス(例: CPU 版)を使うときは COMFYUI_TORCH_INDEX で上書き
$TorchIndex = if ($env:COMFYUI_TORCH_INDEX) { $env:COMFYUI_TORCH_INDEX } else { $V.TORCH_CUDA_INDEX }
Write-Host "==> PyTorch $($V.TORCH_VERSION) ($TorchIndex)"
Invoke-Checked $VenvPy ($Pip + @(
  "torch==$($V.TORCH_VERSION)", "torchvision==$($V.TORCHVISION_VERSION)", "torchaudio==$($V.TORCHAUDIO_VERSION)",
  '--index-url', $TorchIndex))

Write-Host '==> ComfyUI requirements'
Invoke-Checked $VenvPy ($Pip + @('-r', (Join-Path $App 'requirements.txt')))

Write-Host '==> ComfyUI-GGUF requirements'
Invoke-Checked $VenvPy ($Pip + @('-r', (Join-Path $GgufDir 'requirements.txt')))

Write-Host '==> Impact Pack / Subpack requirements'
Invoke-Checked $VenvPy ($Pip + @('-r', (Join-Path $ImpactDir 'requirements.txt')))
Invoke-Checked $VenvPy ($Pip + @('-r', (Join-Path $SubpackDir 'requirements.txt')))

Write-Host '==> controlnet_aux requirements'
Invoke-Checked $VenvPy ($Pip + @('-r', (Join-Path $AuxDir 'requirements.txt')))

Write-Host '==> WD14 Tagger requirements (onnxruntime, CPU)'
Invoke-Checked $VenvPy ($Pip + @('-r', (Join-Path $TaggerDir 'requirements.txt')))

Invoke-Checked $VenvPy @('-c', "import torch; print('torch', torch.__version__, '/ CUDA available:', torch.cuda.is_available())")

Write-Host ''
Write-Host '完了。起動: npm run comfyui:start'
Write-Host 'モデルは comfyui\data\models\ 配下に配置してください(docs/08-local-comfyui.md §5)'
