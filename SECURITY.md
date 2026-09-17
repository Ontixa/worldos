# Security Policy

## Reporting

Report vulnerabilities privately to the maintainers (security@worldos.dev
once provisioned; until then, open a private GitHub security advisory).
Do not file public issues for exploitable bugs.

## Model

WorldOS's security boundary is the **permission-checked command layer**:

- Every mutation carries an `ActorId` and runs through command handlers
  that declare required permissions.
- Capabilities declare the permissions they need; the registry checks
  the caller's actor before dispatch.
- Agents, plugins, and remote clients get *distinct* actors — their
  permissions can be narrowed independently of the human user's.

## Known scope (Genesis)

- `.worldos` files are local trust boundaries: a project file grants
  whatever permissions its actors carry. Treat project files from
  untrusted sources like documents that can embed commands in history.
- The WS/stdio transports have no authentication — bind to localhost
  (the default) and treat any connected client as the local actor.
  Remote/multi-user auth is Forge work.
- MCP exposes the same surface to AI clients; assume a connected MCP
  client can execute any command the serving actor permits.

## Supported versions

Pre-1.0: only the latest commit on `main` receives fixes.
