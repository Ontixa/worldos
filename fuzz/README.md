# WorldOS fuzzing

These libFuzzer targets exercise the input boundaries an untrusted client
or file can reach — no network, credentials, or production data:

- `requirement_expr` — the requirement/`done_when` expression parser and
  evaluator against a fixed project (deep nesting must error, not overflow);
- `state_op_stream` — `StateOp` JSON streams applied forward and backward
  through `Project::apply` (undo/redo/replay primitive);
- `json_rpc` — the `RpcService` dispatch surface on an in-memory engine.
  File-IO methods (`project.open/save/diff/create`, export/import commands,
  `artifact.export`, `plugin.run`, `agent.run`) are skipped so the target
  stays pure and deterministic; on-disk coverage lives in `store_migration`;
- `plugin_protocol` — the hosted-plugin JSON-RPC line pump
  (`serve_stream`) fed arbitrary plugin stdout;
- `store_migration` — `Snapshot` JSON plus real `.worldos` bytes opened
  through `SqliteStore::open` + `load` (schema migration, corrupt/garbage
  rejection) inside a per-process temp dir;
- `cad_selector` — `Selector` JSON (object and bare-string shorthand, as
  accepted by `select`/`edge_select` params and persisted recipes)
  resolved through `target()`/`resolve*` against a fixed box-like view
  and a fuzzed `TopologyView`;
- `mesh_import` — the hand-rolled binary-STL and OBJ parsers behind
  `geometry.import`, plus the `geom:mesh` component decoder; decoded
  meshes are pushed back through the STL/OBJ writers and re-parsed.

Install the same pinned tools used by CI, then run any target from the
repository root:

```sh
rustup toolchain install nightly-2026-08-20 --profile minimal
cargo install cargo-fuzz --version 0.13.2 --locked
cargo +nightly-2026-08-20 fuzz run requirement_expr -- -max_total_time=60 -max_len=4096 -rss_limit_mb=2048
cargo +nightly-2026-08-20 fuzz run state_op_stream -- -max_total_time=60 -max_len=4096 -rss_limit_mb=2048
cargo +nightly-2026-08-20 fuzz run json_rpc -- -max_total_time=60 -max_len=4096 -rss_limit_mb=2048
cargo +nightly-2026-08-20 fuzz run plugin_protocol -- -max_total_time=60 -max_len=4096 -rss_limit_mb=2048
cargo +nightly-2026-08-20 fuzz run store_migration -- -max_total_time=60 -max_len=65536 -rss_limit_mb=2048
cargo +nightly-2026-08-20 fuzz run cad_selector -- -max_total_time=60 -max_len=4096 -rss_limit_mb=2048
cargo +nightly-2026-08-20 fuzz run mesh_import -- -max_total_time=60 -max_len=65536 -rss_limit_mb=2048
```

Pull requests and `main` receive a bounded smoke run; the weekly schedule
spends five minutes on each target. Local corpora, coverage data, and
crash artifacts are intentionally ignored. If a crash may cross a trust
boundary (capability checks, transactions, containment), preserve it
privately and follow [`SECURITY.md`](../SECURITY.md) before opening a
public issue or pull request.
