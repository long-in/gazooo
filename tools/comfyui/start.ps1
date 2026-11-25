# ローカル ComfyUI の起動(Windows)。docs/08-local-comfyui.md 参照。
#Requires -Version 5.1
$ErrorActionPreference = 'Stop'

$Here = Split-Path -Parent $MyInvocation.MyCommand.Path
$Root = (Resolve-Path (Join-Path $Here '..\..')).Path
$Dest = Join-Path $Root 'comfyui'
$Py   = Join-Path $Dest 'venv\Scripts\python.exe'
$Main = Join-Path $Dest 'ComfyUI\main.py'

if (-not ((Test-Path $Py) -and (Test-Path $Main))) {
  Write-Error 'ERROR: 未セットアップです。先に npm run comfyui:setup を実行してください'
  exit 1
}

$Port = if ($env:COMFYUI_PORT) { $env:COMFYUI_PORT } else { '8188' }
& $Py $Main --listen 127.0.0.1 --port $Port --base-directory (Join-Path $Dest 'data') --disable-dynamic-vram --disable-auto-launch
exit $LASTEXITCODE
