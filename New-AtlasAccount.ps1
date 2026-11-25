param(
    [Parameter(Mandatory)][ValidatePattern('^[a-zA-Z0-9_.@-]{1,80}$')][string]$Username,
    [ValidateSet('viewer', 'reviewer')][string]$Role = 'viewer',
    [switch]$Rotate
)
$ErrorActionPreference = 'Stop'
$path = Join-Path $PSScriptRoot 'accounts.json'
$accounts = @()
if (Test-Path -LiteralPath $path) {
    $stored = ConvertFrom-Json -InputObject (Get-Content -LiteralPath $path -Raw -Encoding UTF8)
    foreach ($account in $stored) { $accounts += $account }
}
if (($accounts | Where-Object username -eq $Username) -and !$Rotate) {
    throw "Account already exists. Use -Rotate to generate a new password."
}
$rng = [Security.Cryptography.RandomNumberGenerator]::Create()
$hash = [Security.Cryptography.SHA256]::Create()
try {
    $bytes = New-Object byte[] 32
    $rng.GetBytes($bytes)
    $password = [Convert]::ToBase64String($bytes).TrimEnd('=').Replace('+','-').Replace('/','_')
    $digest = [BitConverter]::ToString($hash.ComputeHash([Text.Encoding]::UTF8.GetBytes($password))).Replace('-','').ToLowerInvariant()
    $accounts = @($accounts | Where-Object username -ne $Username)
    $accounts += [PSCustomObject]@{ username = $Username; passwordSha256 = $digest; role = $Role }
    $temporary = "$path.$([Guid]::NewGuid().ToString('N')).tmp"
    [IO.File]::WriteAllText($temporary, (ConvertTo-Json -InputObject $accounts -Depth 5), (New-Object Text.UTF8Encoding $false))
    if ([IO.File]::Exists($path)) {
        $backup = "$temporary.bak"
        [IO.File]::Replace($temporary, $path, $backup)
        Remove-Item -LiteralPath $backup
    } else {
        [IO.File]::Move($temporary, $path)
    }
    Write-Host "Username: $Username ($Role)"
    Write-Host "One-time password display: $password"
    Write-Host 'Store it in the company password manager. Restart Atlas to load updated accounts.'
} finally {
    $hash.Dispose()
    $rng.Dispose()
}
