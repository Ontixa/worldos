#Requires -Version 5.1
<#
.SYNOPSIS
    Run WorldOS benchmarks / performance baselines.

.DESCRIPTION
    Runs the WorldBench task corpus via `worldos-bench` (deterministic
    non-LLM baseline: YAML tasks -> engine execution -> JSON evidence)
    and `cargo bench` for any crates that declare [[bench]] targets.
    Reports are written under bench/reports/ and target/bench/.

.PARAMETER Suite
    kernel | graph | persistence | cad | worldbench | all
    (worldbench runs the YAML corpus; others filter cargo benches)

.EXAMPLE
    pwsh scripts/bench.ps1
    pwsh scripts/bench.ps1 -Suite worldbench
#>
[CmdletBinding()]
param(
    [ValidateSet("kernel", "graph", "persistence", "cad", "worldbench", "all")]
    [string]$Suite = "all"
)

$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Push-Location $root

try {
    Write-Host "WorldOS bench" -ForegroundColor Cyan
    Write-Host "  suite: $Suite"
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"

    # ---- WorldBench corpus (the real baseline) --------------------
    if ($Suite -eq "worldbench" -or $Suite -eq "all") {
        $tasks = Join-Path $root "bench/tasks"
        $reports = Join-Path $root "bench/reports"
        New-Item -ItemType Directory -Force -Path $reports | Out-Null
        $out = Join-Path $reports "worldbench-$stamp.json"

        Write-Host "`n== worldos-bench --tasks bench/tasks --strict ==" -ForegroundColor Cyan
        cargo run -p worldos-bench -- --tasks $tasks --out $out --strict
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        Write-Host "  wrote $out" -ForegroundColor DarkGray
    }

    # ---- cargo bench targets ---------------------------------------
    $benchTargets = @()
    Get-ChildItem -Recurse -Path crates -Filter "Cargo.toml" | ForEach-Object {
        $toml = Get-Content $_.FullName -Raw
        if ($toml -match "\[\[bench\]\]") {
            $benchTargets += Split-Path (Split-Path $_.FullName -Parent) -Leaf
        }
    }

    if ($Suite -ne "worldbench") {
        if ($benchTargets.Count -eq 0) {
            Write-Host "`nNo [[bench]] targets exist yet in this workspace." -ForegroundColor Yellow
            Write-Host "bench.ps1 runs cargo bench once real benches land (see docs/engineering/NEXT.md item 10)." -ForegroundColor DarkGray
            $out = Join-Path $root "target/bench"
            New-Item -ItemType Directory -Force -Path $out | Out-Null
            $record = [ordered]@{
                suite = $Suite
                timestamp = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                benches_found = 0
                results = @()
                note = "no [[bench]] targets defined yet"
            }
            $file = Join-Path $out "bench-$stamp.json"
            $record | ConvertTo-Json | Set-Content $file
            Write-Host "  wrote $file" -ForegroundColor DarkGray
        } else {
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
        }
    }
} finally {
    Pop-Location
}
