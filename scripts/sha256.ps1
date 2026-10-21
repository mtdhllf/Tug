param(
    [Parameter(Mandatory = $true)]
    [string] $Directory
)

$ErrorActionPreference = 'Stop'

$Directory = (Resolve-Path -LiteralPath $Directory).Path
$files = @(
    Get-ChildItem -LiteralPath $Directory -File |
        Where-Object { $_.Extension -eq '.exe' -or $_.Extension -eq '.zip' } |
        Sort-Object Name
)

if ($files.Count -eq 0) {
    Write-Error "No .exe or .zip files found in $Directory"
    exit 1
}

$lines = foreach ($file in $files) {
    $hash = (Get-FileHash -LiteralPath $file.FullName -Algorithm SHA256).Hash.ToLowerInvariant()
    '{0}  {1}' -f $hash, $file.Name
}

$lines | Set-Content -LiteralPath (Join-Path $Directory 'SHA256SUMS.txt') -Encoding ASCII
$lines | ForEach-Object { Write-Host "  $_" }
