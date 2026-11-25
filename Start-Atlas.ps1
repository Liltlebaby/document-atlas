param([string]$ConfigPath = (Join-Path $PSScriptRoot 'server-config.json'))
$ErrorActionPreference = 'Stop'
$ConfigPath = (Resolve-Path -LiteralPath $ConfigPath).Path
$config = ConvertFrom-Json -InputObject (Get-Content -LiteralPath $ConfigPath -Raw -Encoding UTF8)
if ($config.EnableChat -isnot [bool]) { throw 'EnableChat must be true or false.' }
$port = [int]$config.Port
if ($port -lt 1024 -or $port -gt 65535) { throw 'Port must be 1024-65535.' }
$node = Get-Command node.exe -ErrorAction Stop
$version = (& $node.Source --version).Trim()
if ($LASTEXITCODE -ne 0 -or [int]($version.TrimStart('v').Split('.')[0]) -lt 24) { throw 'Install Node.js 24 x64 or a compatible newer release.' }
$env:ATLAS_ROOT = $PSScriptRoot
$env:ATLAS_NODE = $node.Source
$env:ATLAS_PORT = [string]$port
$env:HF_HOME = Join-Path $PSScriptRoot 'data\embedding'
$env:ATLAS_ENABLE_CHAT = '0'
$env:ATLAS_CODEX_ENTRY = Join-Path $PSScriptRoot 'tools\codex\node_modules\@openai\codex\bin\codex.js'
if ($config.EnableChat) {
    if (!(Test-Path -LiteralPath $env:ATLAS_CODEX_ENTRY)) { throw 'Run Install-AtlasCodex.ps1 under the Atlas service account first.' }
    $status = & {
        $ErrorActionPreference = 'Continue'
        & $node.Source $env:ATLAS_CODEX_ENTRY login status 2>&1
    } | Out-String
    if ($LASTEXITCODE -ne 0 -or $status -notmatch 'ChatGPT') { throw 'Log in to Codex with the approved ChatGPT account under the same Windows service account.' }
    $env:ATLAS_ENABLE_CHAT = '1'
}
if ($config.PortalCredentialFile) {
    if (!$config.PortalUrl) { throw 'Set PortalUrl to the exact Portal URL in the snapshot.' }
    $credential = Import-Clixml -LiteralPath $config.PortalCredentialFile
    if ($credential -isnot [PSCredential]) { throw 'PortalCredentialFile must contain a Windows DPAPI protected PSCredential.' }
    $env:ATLAS_PORTAL_URL = $config.PortalUrl
    $env:ATLAS_PORTAL_USER = $credential.UserName
    $env:ATLAS_PORTAL_PASSWORD = $credential.GetNetworkCredential().Password
}
try {
    if ($null -ne $config.Lan) {
        if (!$config.Lan.PfxCredentialFile) { throw 'Set Lan.PfxCredentialFile to a DPAPI-protected PSCredential file.' }
        $tlsCredential = Import-Clixml -LiteralPath $config.Lan.PfxCredentialFile
        if ($tlsCredential -isnot [PSCredential]) { throw 'Lan.PfxCredentialFile must contain a Windows DPAPI-protected PSCredential.' }
        $env:ATLAS_TLS_PASSPHRASE = $tlsCredential.GetNetworkCredential().Password
        & $node.Source (Join-Path $PSScriptRoot 'lan-gateway.mjs') $ConfigPath
    } else {
        & (Join-Path $PSScriptRoot 'document-atlas-web.exe')
    }
    exit $LASTEXITCODE
} finally {
    Remove-Item -LiteralPath Env:ATLAS_PORTAL_PASSWORD -ErrorAction SilentlyContinue
    Remove-Item -LiteralPath Env:ATLAS_TLS_PASSPHRASE -ErrorAction SilentlyContinue
}
