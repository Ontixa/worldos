#!/usr/bin/env bash
# Regenerate examples/genesis/genesis.worldos via the WorldOS CLI.
set -euo pipefail
cd "$(dirname "$0")"

W=worldos
command -v worldos >/dev/null || W=../../target/debug/worldos.exe

rm -f genesis.worldos
"$W" new genesis --path genesis.worldos
"$W" command genesis.worldos requirement.create \
  '{"name":"housing-clearance","expression":"1 == 1","description":"demo requirement"}'
"$W" command genesis.worldos object.create \
  '{"type":"core:note","name":"readme","components":{"doc:text":{"text":"Genesis example project."}}}'
"$W" command genesis.worldos geometry.create_primitive \
  '{"kind":"cube","name":"reference-cube","position":[0,0,0]}'
"$W" agent genesis.worldos "create another cube next to reference-cube named housing"
"$W" inspect genesis.worldos
