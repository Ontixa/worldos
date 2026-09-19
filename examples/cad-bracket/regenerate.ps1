# Regenerate examples/cad-bracket/bracket.worldos from plan.json.
# Requires `worldos` on PATH (cargo install --path crates/worldos-cli
# or cargo run -p worldos-cli -- ...).
$ErrorActionPreference = 'Stop'
$here = Split-Path -Parent $MyInvocation.MyCommand.Path
$proj = Join-Path $here 'bracket.worldos'
if (Test-Path $proj) { Remove-Item $proj }
worldos new bracket --path $proj
worldos batch $proj (Join-Path $here 'plan.json')
worldos command $proj cad.measure '{\"object\":\"bracket\"}' --json
