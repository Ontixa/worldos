# Opt-in native CAD through the CLI

The default `cargo build --locked -p worldos-cli` does not include the native
CAD adapter. To include it, build explicitly:

```sh
cargo build --locked -p worldos-cli --features cad
```

Use the newly built `target/debug/worldos` (`worldos.exe` on Windows), not an
older executable on PATH. `--cad` must also be supplied for each session that
needs CAD commands. A build without the feature rejects this flag before
creating a project or artifact sidecar. Merely opening a project does not
automatically enable native CAD.

An explicit `--cad` attachment may create its artifact directory even for an
inspection/catalog command. In a CAD-enabled `new`, project creation precedes
kernel/store attachment: an initialization failure may leave the new project
file behind. Only rejection of an unavailable build feature is guaranteed to
precede these writes; failures do not imply general filesystem rollback.

From the repository root, choose a new disposable project path with no valuable
existing file. These example scripts contain JSON data, not shell commands:

```sh
target/debug/worldos --cad --json new cad-demo --path cad-demo.worldos
target/debug/worldos --cad --json batch cad-demo.worldos examples/cad/box.json
target/debug/worldos --json history cad-demo.worldos
target/debug/worldos --json undo cad-demo.worldos
target/debug/worldos --json redo cad-demo.worldos
target/debug/worldos --cad --json batch cad-demo.worldos examples/cad/regenerate.json
```

The first batch creates a 50 × 40 × 20 mm B-rep box, measures it with the kernel
(volume 40,000 mm³), and exports STEP into the artifact store. The entire batch
is one transaction: undo removes its graph changes; redo restores them. Each
CLI invocation opens the saved project again. The second batch regenerates the
persisted recipe and measures the result. There is no agent or LLM invocation.

`cad.export_step` returns a `sha256:` artifact reference, **not** an arbitrary
output filename. `cad.import_step` accepts that reference in its `step` field,
or a filesystem path in `file` (requiring the actor's `filesystem.read`
permission). Use `worldos --cad commands <project> --json` for the exact current
schemas. General `worldos export` remains a project-JSON export, not STEP export.

### Copy the STEP artifact to a file

Copy the `step` reference from the export result (the final result in the box
batch), then replace `sha256:...` below with that actual reference:

```sh
target/debug/worldos --json artifact export cad-demo.worldos sha256:... block.step
```

This command also works in the default CLI build, without `--cad` or a native
kernel. It returns `reference` (normalized to `sha256:<hex>`), `path`, `bytes`
and `verified: true` only after hash verification, writing and file sync succeed.
Both a lowercase 64-hex digest and its `sha256:` form are accepted. The fixed
maximum artifact size is 64 MiB; there is no unlimited override. Existing output
files, directories and final-component symlinks are never overwritten. Parent
directories must already exist. Missing or corrupt artifacts do not create the
destination or initialize a sidecar.

The resulting `block.step` can be opened in another STEP-capable application.
To reimport it through WorldOS, save this as a JSON batch and run it with
`worldos --cad --json batch cad-demo.worldos <script-path>`:

```json
[
  {"type":"cad.import_step","input":{"file":"block.step","name":"roundtrip"}},
  {"type":"cad.measure","input":{"object":"roundtrip"}}
]
```

The roundtrip volume should remain approximately 40,000 mm³. Paths are relative
to the current working directory, not the project directory.

Artifact copying is a trusted local-operator operation exposed by a shared
`Engine::export_project_artifact` API, not an agent/RPC command. The API requires
a caller-asserted human actor with `artifact.export` and `filesystem.write`;
this is not authenticated human identity or an isolation boundary. Default
agent permissions are unchanged. Parent paths are trusted; no filesystem sandbox
or concurrent-path-replacement protection is promised.

Only an existing regular project path and its associated sidecar are required.
The copy does not open, validate or migrate the database, check graph membership
of the artifact, modify project history, or attach CAD. It is not a graph-level
authorization mechanism. External output files are outside undo. On write or sync
failure the error names the newly created output, which is retained and may be
partial (or fully written but with uncertain durability). Inspect/remove that
file manually before retrying. Process crashes can likewise leave output behind;
atomic publication and crash rollback are not claimed.

## Persistence and authority

Keep `cad-demo.worldos` and `cad-demo.worldos.artifacts/` together when copying
or backing up the project. Large BRep/STEP/STL outputs are content-addressed
blobs, not inline SQLite components. Undo removes/restores graph references;
it does not delete blobs or undo arbitrary external filesystem effects. Do not
remove the sidecar while the project still references its artifacts.

The CLI attaches the existing kernel to `Engine` and executes existing handlers;
it does not bypass command validation, actor permissions, history or capability
execution. Ordinary CLI commands retain the existing `local-user` human actor.
The CLI tests verify that attribution, not a new actor-selection/permission API.
Generic restricted-actor tests remain in the engine suite.

`--cad` also applies when this CLI opens a project for RPC/MCP/plugin/agent
sessions; it does not grant extra actor permissions. Existing transports have
no authentication boundary beyond their documented local deployment model.
Do not expose them to untrusted clients. No transport or process sandbox is added.

## Build and verification limits

The optional adapter uses pinned cadrum 0.8.20 / OCCT 8.0.1. Native builds may
download prebuilt OCCT artifacts and add substantial link time and binary size
(ADR 0006 records approximately 120 MB per static consumer). Follow the existing
Windows toolchain instructions; this change does not establish new Linux/macOS
support. `cargo build --workspace` and engine tests already include CAD consumers;
the lightweight promise is scoped to the default CLI package build.

```sh
cargo test --locked -p worldos-cli --no-default-features
cargo test --locked -p worldos-cli --features cad
```

Feature-enabled CLI tests exercise separate real CLI processes for create,
measure, STEP artifact import/export, undo/redo, reopen and regeneration, and
reject invalid geometry/missing artifacts without changing graph/history. They
also require each command response to be parseable JSON. The engine's CAD
vertical test alone does not prove CLI behavior. Native kernel operations are
not a universal reversibility or numerical-robustness guarantee.

Desktop initialization and its viewport are unchanged: this is a CLI exposure,
not a desktop CAD release. No model credentials or provider calls are needed.
