#Requires -Version 5.1
<#
.SYNOPSIS
    Diagnose the WorldOS development environment on Windows.

.DESCRIPTION
    Checks every tool WorldOS development touches and prints actionable
    diagnostics. Never prints secrets -- tokens are reported as
    present/absent only.

.EXAMPLE
    pwsh scripts/doctor.ps1
#>
[CmdletBinding()]
param()

$script:failures = 0
$script:warnings = 0

function Write-Check {
    param(
        [string]$Name,
        [bool]$Ok,
        [string]$Detail = "",
        [string]$Fix = ""
    )
    if ($Ok) {
        Write-Host "  [ok]   $Name" -ForegroundColor Green -NoNewline
        if ($Detail) { Write-Host "  $Detail" -ForegroundColor DarkGray } else { Write-Host "" }
    } else {
        $script:failures++
        Write-Host "  [FAIL] $Name" -ForegroundColor Red -NoNewline
        if ($Detail) { Write-Host "  $Detail" -ForegroundColor DarkGray } else { Write-Host "" }
        if ($Fix) { Write-Host "         fix: $Fix" -ForegroundColor Yellow }
    }
}

function Write-Warn {
    param([string]$Name, [string]$Detail = "", [string]$Fix = "")
    $script:warnings++
    Write-Host "  [warn] $Name" -ForegroundColor Yellow -NoNewline
    if ($Detail) { Write-Host "  $Detail" -ForegroundColor DarkGray } else { Write-Host "" }
    if ($Fix) { Write-Host "         fix: $Fix" -ForegroundColor Yellow }
}

function Get-CmdVersion {
    param([string]$Cmd, [string[]]$VArgs = @("--version"))
    try {
        $out = & $Cmd @VArgs 2>$null | Select-Object -First 1
        return $out
    } catch { return $null }
}

Write-Host ""
Write-Host "WorldOS doctor" -ForegroundColor Cyan
Write-Host "==============" -ForegroundColor Cyan

# ----- platform -----------------------------------------------------------
Write-Host "`nPlatform"
$os = [System.Environment]::OSVersion
Write-Check "Windows" ($os.Platform -eq "Win32NT") "$($os.VersionString)"
Write-Check "PowerShell" ($PSVersionTable.PSVersion.Major -ge 5) "v$($PSVersionTable.PSVersion)" "Install PowerShell 5.1+ or pwsh 7+"

# ----- git / github -------------------------------------------------------
Write-Host "`nRepository"
$gitCmd = Get-Command git -ErrorAction SilentlyContinue
$gitPath = if ($gitCmd) { $gitCmd.Source } else { $null }
$isWsl = $gitPath -and ($gitPath -match "wsl|/usr/bin|/bin/")
if ($gitPath -and -not $isWsl) {
    Write-Check "git (Windows-native)" $true "$gitPath"
} elseif ($isWsl) {
    Write-Check "git (Windows-native)" $false "resolved to WSL git: $gitPath" "Use Windows Git. Do NOT mix WSL and Windows git on this checkout."
} else {
    Write-Check "git (Windows-native)" $false "git not on PATH" "Install Git for Windows"
}

$ghCmd = Get-Command gh -ErrorAction SilentlyContinue
$ghPath = if ($ghCmd) { $ghCmd.Source } else { $null }
if ($ghPath) {
    Write-Check "gh CLI" $true "$ghPath"
    $ghAuth = & gh auth status 2>&1 | Out-String
    if ($ghAuth -match "Logged in to github.com account (\S+)") {
        Write-Check "gh auth (github.com)" $true "account: $($Matches[1])"
    } else {
        Write-Warn "gh auth (github.com)" "not logged in" "gh auth login"
    }
} else {
    Write-Warn "gh CLI" "not found" "winget install GitHub.cli"
}

if ($gitPath -and -not $isWsl) {
    $root = & git rev-parse --show-toplevel 2>$null
    if ($root) {
        Write-Check "repo root" $true $root
        $branch = & git branch --show-current 2>$null
        Write-Check "branch" ([bool]$branch) "$branch"
        $dirty = & git status --porcelain 2>$null | Select-Object -First 5
        if ($dirty) {
            Write-Warn "working tree" "modified paths present" "git status --short"
        } else {
            Write-Check "working tree clean" $true
        }
        $ident = & git config user.name 2>$null
        $email = & git config user.email 2>$null
        Write-Check "git identity" ([bool]$ident -and [bool]$email) "$ident <$email>"
    } else {
        Write-Warn "repo root" "not inside a git work tree" "cd to the worldos checkout"
    }
}

# ----- rust -----------------------------------------------------------------
Write-Host "`nRust toolchain"
$rustc = Get-CmdVersion rustc
$cargo = Get-CmdVersion cargo
Write-Check "rustc" ([bool]$rustc) "$rustc" "rustup install stable"
Write-Check "cargo" ([bool]$cargo) "$cargo" "rustup install stable"
$host_ = & rustc -vV 2>$null | Select-String "host:" | ForEach-Object { ($_ -split "host:\s*")[1] }
if ($host_ -eq "x86_64-pc-windows-msvc" -or $host_ -eq "x86_64-pc-windows-gnu") {
    Write-Check "host triple" $true "$host_"
} else {
    Write-Warn "host triple" "$host_" "Expected x86_64-pc-windows-{msvc,gnu} for CI parity"
}
foreach ($comp in @("clippy", "rustfmt")) {
    $ok = & rustup component list --installed 2>$null | Select-String "^$comp"
    if ($ok) { Write-Check $comp $true } else { Write-Warn $comp "not installed" "rustup component add $comp" }
}

# ----- node / npm -------------------------------------------------------------
Write-Host "`nJS toolchain"
$node = Get-CmdVersion node
$npm = Get-CmdVersion npm "--version"
Write-Check "node" ([bool]$node) "$node" "Install Node >= 20"
Write-Check "npm" ([bool]$npm) "v$npm" "Install Node >= 20"
$npmModules = Test-Path (Join-Path (Join-Path $PSScriptRoot "..") "node_modules")
if ($npmModules) { Write-Check "npm install" $true "node_modules present" } else { Write-Warn "npm install" "node_modules missing" "npm ci" }

# ----- python -------------------------------------------------------------------
Write-Host "`nPython"
# Prefer a real interpreter over the WindowsApps Store shim: collect all
# 'python' resolutions and pick the first one that is not the Store stub.
$pyExe = $null
foreach ($c in (Get-Command python -All -ErrorAction SilentlyContinue)) {
    if ($c.Source -notmatch "WindowsApps") { $pyExe = $c.Source; break }
}
if (-not $pyExe) {
    foreach ($c in (Get-Command py -All -ErrorAction SilentlyContinue)) {
        if ($c.Source -notmatch "WindowsApps") { $pyExe = $c.Source; break }
    }
}
if ($pyExe) {
    $py = Get-CmdVersion $pyExe
    if ($py -and $py -match "Python (\d+)\.(\d+)") {
        $ok = ([int]$Matches[1] -gt 3) -or ([int]$Matches[1] -eq 3 -and [int]$Matches[2] -ge 10)
        Write-Check "python >= 3.10" $ok "$py ($pyExe)" "Install Python 3.10+"
    } else {
        Write-Warn "python" "resolved but no version output ($pyExe)"
    }
} else {
    Write-Warn "python" "not found (or only the WindowsApps Store shim)" "Install Python 3.10+ (SDK/examples/plugins only)"
}

# ----- native build tools ---------------------------------------------------------
Write-Host "`nNative build (optional -- OCCT ships prebuilt via crates)"
$cmake = Get-CmdVersion cmake
if ($cmake) { Write-Check "cmake" $true "$cmake" } else { Write-Warn "cmake" "not found" "Only needed to build OCCT from source" }
$cc = Get-CmdVersion cc
$cl = Get-Command cl.exe -ErrorAction SilentlyContinue
$ccDesc = if ($cc) { $cc } elseif ($cl) { $cl.Source } else { $null }
if ($ccDesc) { Write-Check "C++ compiler" $true "$ccDesc" } else { Write-Warn "C++ compiler" "not found" "Only needed for source builds of native deps" }

# ----- env vars (presence only -- NEVER print values) --------------------------------
Write-Host "`nEnvironment variables (presence only)"
foreach ($v in @("WORLDOS_LLM_KIND", "WORLDOS_LLM_BASE_URL", "WORLDOS_LLM_MODEL", "OPENAI_API_KEY", "WORLDOS_PLUGIN_PATH", "OCCT_ROOT")) {
    $set = [bool](Get-Item "Env:$v" -ErrorAction SilentlyContinue)
    if ($set) {
        $safe = if ($v -match "KEY|TOKEN|SECRET|PASSWORD") { "set (hidden)" } else { "set" }
        Write-Check $v $true $safe
    } else {
        Write-Host "  [info] $v" -ForegroundColor DarkGray -NoNewline
        Write-Host "  not set" -ForegroundColor DarkGray
    }
}

# ----- summary -----------------------------------------------------------------------
Write-Host "`n--------------" -ForegroundColor Cyan
if ($script:failures -eq 0 -and $script:warnings -eq 0) {
    Write-Host "All checks passed." -ForegroundColor Green
} else {
    $color = if ($script:failures) { "Red" } else { "Yellow" }
    Write-Host "$($script:failures) failure(s), $($script:warnings) warning(s)." -ForegroundColor $color
}
exit $(if ($script:failures -gt 0) { 1 } else { 0 })
