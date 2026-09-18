#Requires -Version 5.1
<#
.SYNOPSIS
    Fast WorldOS quality gates  -  the same checks CI runs.

.DESCRIPTION
    Runs: cargo fmt --check, cargo clippy (-D warnings), TypeScript
    typecheck, Python SDK compile. Fails fast on the first broken gate
    unless -ContinueOnError is given.

.PARAMETER ContinueOnError
    Run every gate and report all failures at the end.

.EXAMPLE
    pwsh scripts/check.ps1
    pwsh scripts/check.ps1 -ContinueOnError
#>
[CmdletBinding()]
param(
    [switch]$ContinueOnError,
    [switch]$SkipNode
)

$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Push-Location $root

$script:failed = @()

function Invoke-Gate {
    param([string]$Name, [scriptblock]$Run)
    Write-Host "`n== $Name ==" -ForegroundColor Cyan
    try {
        & $Run
        if ($LASTEXITCODE -ne 0) { throw "exit code $LASTEXITCODE" }
        Write-Host "   ok" -ForegroundColor Green
    } catch {
        $script:failed += $Name
        Write-Host "   FAILED: $_" -ForegroundColor Red
        if (-not $ContinueOnError) { throw }
    }
}

try {
    Invoke-Gate "cargo fmt --check" { cargo fmt --all -- --check }
    Invoke-Gate "cargo clippy" { cargo clippy --workspace --all-targets -- -D warnings }

    if (-not $SkipNode) {
        if (Test-Path (Join-Path $root "node_modules")) {
            Invoke-Gate "npm typecheck" { npm run typecheck --workspaces --if-present }
        } else {
            Write-Host "`n== npm typecheck == skipped (node_modules missing  -  run npm ci)" -ForegroundColor DarkGray
        }
    }

    $pyExe = $null
    foreach ($c in (Get-Command python -All -ErrorAction SilentlyContinue)) {
        if ($c.Source -notmatch "WindowsApps") { $pyExe = $c.Source; break }
    }
    if ($pyExe) {
        Invoke-Gate "python sdk compile" { & $pyExe -m py_compile sdks/worldos-py/worldos.py }
    } else {
        Write-Host "`n== python sdk compile == skipped (no real python on PATH)" -ForegroundColor DarkGray
    }
} finally {
    Pop-Location
}

if ($script:failed.Count -gt 0) {
    Write-Host "`nGates failed: $($script:failed -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host "`nAll checks passed." -ForegroundColor Green
exit 0
