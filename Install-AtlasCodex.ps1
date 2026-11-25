param([switch]$DeviceLogin)
$ErrorActionPreference = 'Stop'
$destination = Join-Path $PSScriptRoot 'tools\codex'
$node = (Get-Command node.exe -ErrorAction Stop).Source
$npm = (Get-Command npm.cmd -ErrorAction Stop).Source
New-Item -ItemType Directory -Path $destination -Force | Out-Null
& $npm install --prefix $destination --no-audit --no-fund --omit=dev --ignore-scripts --save-exact '@openai/codex@0.166.0'
if ($LASTEXITCODE -ne 0) { throw 'Codex installation failed.' }
$entry = Join-Path $destination 'node_modules\@openai\codex\bin\codex.js'
& $node $entry --version
if ($LASTEXITCODE -ne 0) { throw 'Codex failed to start.' }
if ($DeviceLogin) { & $node $entry login --device-auth; exit $LASTEXITCODE }
