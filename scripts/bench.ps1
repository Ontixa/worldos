#Requires -Version 5.1
<#
.SYNOPSIS
    Run WorldOS benchmarks / performance baselines.

.DESCRIPTION
    Runs the WorldBench task corpus via `worldos-bench` (deterministic
    non-LLM baseline: YAML tasks -> engine execution -> JSON evidence),
    the `worldos-bench perf` size-scaled baseline (create/save/reopen/
    query/undo wall-times at N objects), and `cargo bench` for any
    crates that declare [[bench]] targets.
    Reports are written under bench/reports/ and target/bench/.

.PARAMETER Suite
    kernel | graph | persistence | cad | worldbench | perf | all
    (worldbench runs the YAML corpus; perf runs the size-scaled
    baseline; others filter cargo benches)

.PARAMETER PerfSizes
    Comma-separated object counts for -Suite perf / all.
    Default "100,10000". "100000" measured ~2.5 min on a Windows dev
    host (the save phase dominates: ~90 s to fsync + serialize a 93 MB
    project file under antivirus scanning) — supported but opt-in.

.EXAMPLE
    pwsh scripts/bench.ps1
    pwsh scripts/bench.ps1 -Suite worldbench
    pwsh scripts/bench.ps1 -Suite perf -PerfSizes 100,1000,10000
#>
[CmdletBinding()]
param(
    [ValidateSet("kernel", "graph", "persistence", "cad", "worldbench", "perf", "all")]
    [string]$Suite = "all",
    [string]$PerfSizes = "100,10000"
)

$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Push-Location $root

try {
    Write-Host "WorldOS bench" -ForegroundColor Cyan
    Write-Host "  suite: $Suite"
    $stamp = Get-Date -Format "yyyyMMdd-HHmmss"
    $reports = Join-Path $root "bench/reports"

    # ---- WorldBench corpus (the real baseline) --------------------
    if ($Suite -eq "worldbench" -or $Suite -eq "all") {
        $tasks = Join-Path $root "bench/tasks"
        New-Item -ItemType Directory -Force -Path $reports | Out-Null
        $out = Join-Path $reports "worldbench-$stamp.json"

        Write-Host "`n== worldos-bench --tasks bench/tasks --strict ==" -ForegroundColor Cyan
        cargo run -p worldos-bench -- --tasks $tasks --out $out --strict
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        Write-Host "  wrote $out" -ForegroundColor DarkGray
    }

    # ---- Performance baselines ------------------------------------
    # Single-run wall times on a Windows dev host: NTFS + antivirus
    # scanning + synchronous=FULL make save/create fsync-bound. Treat
    # the numbers as regression baselines, not SLAs (see
    # docs/engineering/LIMITATIONS.md).
    if ($Suite -eq "perf" -or $Suite -eq "all") {
        New-Item -ItemType Directory -Force -Path $reports | Out-Null
        $out = Join-Path $reports "worldbench-perf-$stamp.json"

        Write-Host "`n== worldos-bench perf --sizes $PerfSizes ==" -ForegroundColor Cyan
        # Dev profile on purpose: the workspace already builds every
        # dependency at opt-level=2, and dev keeps the baseline in the
        # same profile `cargo test` runs under. The report records the
        # profile so a release-mode comparison stays unambiguous.
        cargo run -p worldos-bench -- perf --sizes $PerfSizes --out $out
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        Write-Host "  wrote $out" -ForegroundColor DarkGray
    }

    # ---- cargo bench targets ---------------------------------------
    if ($Suite -ne "worldbench" -and $Suite -ne "perf") {
        $benchTargets = @()
        Get-ChildItem -Recurse -Path crates -Filter "Cargo.toml" | ForEach-Object {
            $toml = Get-Content $_.FullName -Raw
            if ($toml -match "\[\[bench\]\]") {
                $benchTargets += Split-Path (Split-Path $_.FullName -Parent) -Leaf
            }
        }

        if ($benchTargets.Count -eq 0) {
            Write-Host "`nNo [[bench]] targets exist yet in this workspace." -ForegroundColor Yellow
            Write-Host "The measured baseline is `worldos-bench perf` (see -Suite perf)." -ForegroundColor DarkGray
            $out = Join-Path $root "target/bench"
            New-Item -ItemType Directory -Force -Path $out | Out-Null
            $record = [ordered]@{
                suite = $Suite
                timestamp = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                benches_found = 0
                results = @()
                note = "no [[bench]] targets defined yet; `worldos-bench perf` is the measured baseline"
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
