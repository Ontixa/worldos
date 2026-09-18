#Requires -Version 5.1
<#
.SYNOPSIS
    Run WorldOS test suites.

.PARAMETER Suite
    Which suite to run: core | integration | desktop | sdks | all.
    Default: all (rust workspace tests + sdk checks + desktop check).

.PARAMETER Crate
    Restrict to a single crate (e.g. -Crate worldos-store).

.PARAMETER Release
    Run tests in release mode.

.EXAMPLE
    pwsh scripts/test.ps1
    pwsh scripts/test.ps1 -Suite core
    pwsh scripts/test.ps1 -Crate worldos-engine -Release
#>
[CmdletBinding()]
param(
    [ValidateSet("core", "integration", "desktop", "sdks", "all")]
    [string]$Suite = "all",
    [string]$Crate = "",
    [switch]$Release
)

$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Push-Location $root
$script:failed = @()

function Step {
    param([string]$Name, [scriptblock]$Run)
    Write-Host "`n== $Name ==" -ForegroundColor Cyan
    & $Run
    if ($LASTEXITCODE -ne 0) { $script:failed += $Name }
}

try {
    $cargoArgs = @("test")
    if ($Release) { $cargoArgs += "--release" }

    if ($Crate) {
        Step "cargo test -p $Crate" { & cargo @cargoArgs -p $Crate --all-targets }
    } elseif ($Suite -in @("core", "all")) {
        Step "cargo test --workspace" { & cargo @cargoArgs --workspace }
    }

    if (-not $Crate -and $Suite -in @("integration", "all")) {
        # integration tests are per-crate tests/ dirs  -  included in workspace run;
        # this step runs the heavier e2e targets explicitly for visibility
        Step "cli e2e" { & cargo @cargoArgs -p worldos-cli --test cli }
    }

    if (-not $Crate -and $Suite -in @("sdks", "all")) {
        if (Test-Path (Join-Path $root "node_modules")) {
            Step "ts sdk build" { npm run build -w @worldos/sdk }
        } else {
            Write-Host "`n== ts sdk build == skipped (npm ci first)" -ForegroundColor DarkGray
        }
        $pyExe = $null
        foreach ($c in (Get-Command python -All -ErrorAction SilentlyContinue)) {
            if ($c.Source -notmatch "WindowsApps") { $pyExe = $c.Source; break }
        }
        if ($pyExe) {
            Step "python sdk compile" { & $pyExe -m py_compile sdks/worldos-py/worldos.py }
        }
    }

    if (-not $Crate -and $Suite -in @("desktop", "all")) {
        if (Test-Path (Join-Path $root "apps/desktop/src-tauri")) {
            Step "desktop cargo check" {
                Push-Location apps/desktop/src-tauri
                try { cargo check } finally { Pop-Location }
            }
        }
    }
} finally {
    Pop-Location
}

if ($script:failed.Count -gt 0) {
    Write-Host "`nSuites failed: $($script:failed -join ', ')" -ForegroundColor Red
    exit 1
}
Write-Host "`nAll suites passed." -ForegroundColor Green
exit 0
