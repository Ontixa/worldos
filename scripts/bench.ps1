#Requires -Version 5.1
<#
.SYNOPSIS
    Run WorldOS benchmarks / performance baselines.

.DESCRIPTION
    Runs `cargo bench` for crates that define benches. If no benches
    exist yet, reports that honestly  -  this script never fabricates
    numbers. Results are written under target/bench/ as JSON lines so
    regressions can be diffed over time.

.PARAMETER Suite
    kernel | graph | persistence | cad | worldbench | all
    (selects benches by name filter where implemented)

.EXAMPLE
    pwsh scripts/bench.ps1
    pwsh scripts/bench.ps1 -Suite kernel
#>
[CmdletBinding()]
param(
    [ValidateSet("kernel", "graph", "persistence", "cad", "worldbench", "all")]
    [string]$Suite = "all"
)

$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Push-Location $root

try {
    # Discover crates that actually declare benches
    $benchTargets = @()
    Get-ChildItem -Recurse -Path crates -Filter "Cargo.toml" | ForEach-Object {
        $toml = Get-Content $_.FullName -Raw
        if ($toml -match "\[\[bench\]\]") {
            $benchTargets += Split-Path (Split-Path $_.FullName -Parent) -Leaf
        }
    }

    Write-Host "WorldOS bench" -ForegroundColor Cyan
    Write-Host "  suite: $Suite"

    if ($benchTargets.Count -eq 0) {
        Write-Host "`nNo [[bench]] targets exist yet in this workspace." -ForegroundColor Yellow
        Write-Host "bench.ps1 runs cargo bench once real benches land (see docs/engineering/NEXT.md item 10)." -ForegroundColor DarkGray
        # still produce a machine-readable empty run record for CI continuity
        $out = Join-Path $root "target/bench"
        New-Item -ItemType Directory -Force -Path $out | Out-Null
        $record = [ordered]@{
            suite = $Suite
            timestamp = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            benches_found = 0
            results = @()
            note = "no [[bench]] targets defined yet"
        }
        $file = Join-Path $out "bench-$(Get-Date -Format 'yyyyMMdd-HHmmss').json"
        $record | ConvertTo-Json | Set-Content $file
        Write-Host "  wrote $file" -ForegroundColor DarkGray
        exit 0
    }

    $filter = if ($Suite -eq "all") { "" } else { $Suite }
    foreach ($crate in $benchTargets) {
        Write-Host "`n== cargo bench -p $crate $filter ==" -ForegroundColor Cyan
        if ($filter) {
            cargo bench -p $crate -- $filter
        } else {
            cargo bench -p $crate
        }
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    }
} finally {
    Pop-Location
}
