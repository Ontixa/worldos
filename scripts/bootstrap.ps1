#Requires -Version 5.1
<#
.SYNOPSIS
    Verify and prepare a Windows machine for WorldOS development.

.DESCRIPTION
    Checks for required tooling and explains how to install what's
    missing. By default it does NOT install anything  -  use -Install
    for guided winget installs of missing pieces.

    WorldOS deliberately avoids system-level native dependencies:
    the OCCT CAD kernel arrives prebuilt via crates.io (cadrum), so
    no CMake/OCCT install is required for normal development.

.PARAMETER Install
    Offer to install missing tools via winget.

.EXAMPLE
    pwsh scripts/bootstrap.ps1
    pwsh scripts/bootstrap.ps1 -Install
#>
[CmdletBinding()]
param(
    [switch]$Install
)

$ErrorActionPreference = "Continue"
Write-Host "WorldOS bootstrap" -ForegroundColor Cyan
Write-Host "=================" -ForegroundColor Cyan

$missing = @()

function Require {
    param(
        [string]$Name,
        [string]$Cmd,
        [string]$WingetId,
        [string]$Why
    )
    $found = Get-Command $Cmd -All -ErrorAction SilentlyContinue | Where-Object { $_.Source -notmatch "WindowsApps" } | Select-Object -First 1
    if ($found) {
        $v = & $found.Source --version 2>$null | Select-Object -First 1
        Write-Host "  [ok]      $Name  $v" -ForegroundColor Green
        return
    }
    Write-Host "  [missing] $Name  -  $Why" -ForegroundColor Yellow
    Write-Host "            install: winget install $WingetId" -ForegroundColor DarkGray
    $script:missing += @{ Name = $Name; Id = $WingetId }
    return
}

Write-Host "`nRequired"
Require "Rust (rustup/cargo)" "cargo" "Rustlang.Rustup" "workspace builds, tests, clippy"
Require "Git for Windows"    "git"   "Git.Git"         "version control  -  Windows-native, not WSL"
Require "Node.js >= 20"      "node"  "OpenJS.NodeJS"   "TS SDK + desktop frontend"

Write-Host "`nRecommended"
Require "GitHub CLI"         "gh"    "GitHub.cli"      "PRs, issues, releases"
Require "Python >= 3.10"     "python" "Python.Python.3.12" "Python SDK + example plugins"

Write-Host "`nOptional (native builds only  -  prebuilt OCCT ships via crates)"
$cmake = Get-Command cmake -ErrorAction SilentlyContinue
if ($cmake) { Write-Host "  [ok]      cmake (source builds of native deps)" -ForegroundColor Green }
else { Write-Host "  [info]    cmake not found  -  only needed if building OCCT from source" -ForegroundColor DarkGray }

# rustup components
if (Get-Command rustup -ErrorAction SilentlyContinue) {
    Write-Host "`nRust components"
    foreach ($c in @("clippy", "rustfmt")) {
        $has = & rustup component list --installed 2>$null | Select-String "^$c"
        if ($has) {
            Write-Host "  [ok]      $c" -ForegroundColor Green
        } else {
            Write-Host "  [install] rustup component add $c" -ForegroundColor Yellow
            if ($Install) { & rustup component add $c }
        }
    }
}

# repo-local deps
Write-Host "`nRepo-local"
$root = Resolve-Path (Join-Path $PSScriptRoot "..")
Push-Location $root
try {
    if (Test-Path node_modules) {
        Write-Host "  [ok]      node_modules (npm workspaces installed)" -ForegroundColor Green
    } else {
        Write-Host "  [run]     npm ci" -ForegroundColor Yellow
        if ($Install) { npm ci }
    }
} finally { Pop-Location }

# offer installs
if ($missing.Count -gt 0 -and $Install) {
    Write-Host "`nInstalling missing tools via winget..."
    foreach ($m in $missing) {
        Write-Host "  winget install $($m.Id)" -ForegroundColor Cyan
        winget install --id $m.Id -e --accept-source-agreements --accept-package-agreements
    }
} elseif ($missing.Count -gt 0) {
    Write-Host "`nMissing $($missing.Count) tool(s). Re-run with -Install to install via winget." -ForegroundColor Yellow
}

# next steps
Write-Host "`nNext steps" -ForegroundColor Cyan
Write-Host "  pwsh scripts/doctor.ps1   # verify everything"
Write-Host "  pwsh scripts/check.ps1    # fmt + clippy + sdk gates"
Write-Host "  pwsh scripts/test.ps1     # test suites"
Write-Host "  cargo build -p worldos-cli"
Write-Host ""
exit $(if ($missing.Count -gt 0 -and -not $Install) { 1 } else { 0 })
