//! The `Engine`: one object owning project state, command execution,
//! transactions, history, capabilities and validation.
//!
//! Every interface (GUI, CLI, SDK, MCP, agents) drives an `Engine` —
//! directly in-process or via the RPC layer — so all mutations share the
//! same governed path.

use crate::error::EngineError;
use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use worldos_capability::{Capability, CapabilityError, CapabilityHost, CapabilityRegistry};
use worldos_commands::{
    CommandEnvelope, CommandError, CommandHandler, CommandReceipt, CommandRecord, CommandRegistry,
    CommandSchema, History, Transaction,
};
use worldos_kernel::SearchQuery;
use worldos_kernel::actor::{Actor, Permission};
use worldos_kernel::events::EngineEvent;
use worldos_kernel::ids::{ObjectId, TransactionId};
use worldos_kernel::model::{Object, Relation};
use worldos_kernel::project::Project;
use worldos_kernel::schema as jsonschema;
use worldos_kernel::validation::{ValidationReport, Validator};
use worldos_store::{ProjectStore, Snapshot, SqliteStore};

const EVENT_CAP: usize = 2048;

/// Subscriber callback for engine events.
type EventListener = Box<dyn Fn(&EngineEvent) + Send>;

pub struct Engine {
    project: Project,
    registry: CommandRegistry,
    capabilities: CapabilityRegistry,
    validators: Vec<Box<dyn Validator>>,
    history: History,
    open_txn: Option<Transaction>,
    actor: Actor,
    events: VecDeque<EngineEvent>,
    listeners: Vec<EventListener>,
    path: Option<PathBuf>,
    dirty: bool,
    cad: Option<Arc<worldos_commands::builtin::CadServices>>,
}

impl Engine {
    /// Fresh in-memory project.
    pub fn new(name: impl Into<String>) -> Self {
        let mut e = Self {
            project: Project::new(name),
            registry: worldos_commands::builtin::builtin_registry(),
            capabilities: CapabilityRegistry::new(),
            validators: default_validators(),
            history: History::new(),
            open_txn: None,
            actor: Actor::human("local-user"),
            events: VecDeque::new(),
            listeners: Vec::new(),
            path: None,
            dirty: false,
            cad: None,
        };
        for cap in worldos_capability::builtin::builtins() {
            e.capabilities.register(cap);
        }
        e
    }

    /// Create a new project backed by a `.worldos` file (created on save).
    pub fn create(name: impl Into<String>, path: impl AsRef<Path>) -> Result<Self, EngineError> {
        let mut e = Self::new(name);
        e.path = Some(path.as_ref().to_path_buf());
        e.dirty = true;
        // Persist immediately so the file exists and the schema is live.
        e.save()?;
        Ok(e)
    }

    /// Open an existing `.worldos` file.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, EngineError> {
        let store = SqliteStore::open(&path)?;
        let snap = store.load()?;
        let mut e = Self::new(snap.project.name.clone());
        e.project = snap.project;
        e.history = snap.history;
        e.path = Some(path.as_ref().to_path_buf());
        e.dirty = false;
        e.emit(EngineEvent::ProjectLoaded {
            path: path.as_ref().display().to_string(),
        });
        Ok(e)
    }

    /// Persist to the backing file (atomic SQLite transaction).
    pub fn save(&mut self) -> Result<(), EngineError> {
        let path = self.path.clone().ok_or(EngineError::NoBackingFile)?;
        self.save_as(path)
    }

    pub fn save_as(&mut self, path: impl AsRef<Path>) -> Result<(), EngineError> {
        let mut store = SqliteStore::open(&path)?;
        store.save(&Snapshot::new(self.project.clone(), self.history.clone()))?;
        self.path = Some(path.as_ref().to_path_buf());
        self.dirty = false;
        // Artifacts follow the project file: rebind the cad store to the
        // new sidecar (migrating blobs) so reopening finds them.
        if let Some(cad) = &self.cad {
            let sidecar = worldos_artifact::ArtifactStore::for_project(path.as_ref())
                .map_err(|e| EngineError::Other(e.to_string()))?;
            cad.rebind(sidecar)
                .map_err(|e| EngineError::Other(e.to_string()))?;
        }
        self.emit(EngineEvent::ProjectSaved {
            path: path.as_ref().display().to_string(),
        });
        Ok(())
    }

    // ----- cad ------------------------------------------------------------

    /// Attach a CAD kernel: registers the `cad.*` command handlers with
    /// an artifact store derived from the project file (`<file>.artifacts/`
    /// sidecar) or a temp pool for unsaved projects. The services are
    /// rebound to a fresh sidecar on every `save_as`, so artifacts always
    /// live next to the project file they belong to.
    pub fn attach_cad(
        &mut self,
        kernel: Arc<dyn worldos_cad::CadKernel>,
    ) -> Result<Arc<worldos_commands::builtin::CadServices>, EngineError> {
        let store = match &self.path {
            Some(p) => worldos_artifact::ArtifactStore::for_project(p),
            None => worldos_artifact::ArtifactStore::open(
                std::env::temp_dir().join("worldos").join("artifacts"),
            ),
        }
        .map_err(|e| EngineError::Other(e.to_string()))?;
        Ok(self.attach_cad_with_store(kernel, store))
    }

    /// Attach with a caller-provided artifact store (tests, custom
    /// layouts).
    pub fn attach_cad_with_store(
        &mut self,
        kernel: Arc<dyn worldos_cad::CadKernel>,
        store: worldos_artifact::ArtifactStore,
    ) -> Arc<worldos_commands::builtin::CadServices> {
        let services = worldos_commands::builtin::CadServices::new(kernel, store);
        for h in worldos_commands::builtin::cad_handlers(services.clone()) {
            self.registry.register(h);
        }
        self.cad = Some(services.clone());
        services
    }

    /// CAD services when a kernel is attached.
    pub fn cad(&self) -> Option<Arc<worldos_commands::builtin::CadServices>> {
        self.cad.clone()
    }

    // ----- introspection -------------------------------------------------

    pub fn project(&self) -> &Project {
        &self.project
    }
    pub fn path(&self) -> Option<&Path> {
        self.path.as_deref()
    }
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }
    pub fn actor(&self) -> &Actor {
        &self.actor
    }
    pub fn set_actor(&mut self, actor: Actor) {
        self.actor = actor;
    }
    pub fn history(&self) -> &History {
        &self.history
    }
    pub fn command_schemas(&self) -> Vec<CommandSchema> {
        self.registry.schemas()
    }
    pub fn capability_descriptors(&self) -> Vec<worldos_capability::CapabilityDescriptor> {
        self.capabilities.descriptors()
    }
    pub fn register_command(&mut self, h: Arc<dyn CommandHandler>) {
        self.registry.register(h);
    }
    pub fn register_capability(&mut self, c: Arc<dyn Capability>) {
        self.capabilities.register(c);
    }
    pub fn register_validator(&mut self, v: Box<dyn Validator>) {
        self.validators.push(v);
    }
    pub fn on_event(&mut self, f: impl Fn(&EngineEvent) + Send + 'static) {
        self.listeners.push(Box::new(f));
    }
    pub fn drain_events(&mut self) -> Vec<EngineEvent> {
        self.events.drain(..).collect()
    }
    fn emit(&mut self, ev: EngineEvent) {
        for l in &self.listeners {
            l(&ev);
        }
        if self.events.len() >= EVENT_CAP {
            self.events.pop_front();
        }
        self.events.push_back(ev);
    }

    // ----- object read helpers ------------------------------------------

    pub fn get_object(&self, id: ObjectId) -> Option<&Object> {
        self.project.get(id)
    }
    pub fn find_object(&self, name: &str) -> Option<&Object> {
        self.project.find_by_name(name)
    }
    pub fn search(&self, q: &SearchQuery) -> Vec<&Object> {
        worldos_kernel::search(&self.project, q)
    }
    pub fn object_relations(&self, id: ObjectId) -> Vec<&Relation> {
        self.project.relations_of(id).collect()
    }

    // ----- command execution ---------------------------------------------

    pub fn execute(
        &mut self,
        command_type: &str,
        inputs: serde_json::Value,
    ) -> Result<CommandReceipt, EngineError> {
        let actor = self.actor.clone();
        self.execute_as(&actor, command_type, inputs)
    }

    pub fn execute_as(
        &mut self,
        actor: &Actor,
        command_type: &str,
        inputs: serde_json::Value,
    ) -> Result<CommandReceipt, EngineError> {
        let handler = self
            .registry
            .handler(command_type)
            .ok_or_else(|| CommandError::Unknown(command_type.into()))?;
        let schema = handler.schema();
        if !actor
            .permissions
            .is_allowed(&Permission(schema.permission.clone()))
        {
            return Err(CommandError::PermissionDenied {
                command: command_type.into(),
                perm: schema.permission,
            }
            .into());
        }
        let errors = jsonschema::validate(&schema.input_schema, &inputs, "$");
        if !errors.is_empty() {
            return Err(CommandError::Validation {
                command: command_type.into(),
                errors,
            }
            .into());
        }

        let auto = self.open_txn.is_none();
        if auto {
            self.open_txn = Some(Transaction::new(actor.id.clone(), command_type.to_string()));
        }
        let txn = self.open_txn.as_mut().unwrap();
        let txn_id = txn.id;
        let ops_before = txn.ops.len();

        let mut env = CommandEnvelope::new(command_type, actor.id.clone(), inputs.clone());
        env.transaction_id = Some(txn_id);

        let result = {
            let mut ctx = worldos_commands::CommandContext {
                project: &mut self.project,
                ops: &mut txn.ops,
                actor,
                registry: &self.registry,
            };
            handler.execute(&mut ctx, &inputs)
        };

        match result {
            Ok(output) => {
                self.mark_stale_dependents(command_type, actor, ops_before);
                let command_id = env.id;
                self.emit(EngineEvent::CommandExecuted {
                    command_id,
                    command_type: command_type.into(),
                    actor: actor.id.clone(),
                });
                let txn = self.open_txn.as_mut().unwrap();
                txn.commands.push(CommandRecord {
                    envelope: env,
                    ok: true,
                    output: output.clone(),
                    error: None,
                });
                if auto {
                    self.commit_transaction()?;
                }
                Ok(CommandReceipt {
                    command_id,
                    transaction_id: txn_id,
                    output,
                })
            }
            Err(e) => {
                let txn = self.open_txn.as_mut().unwrap();
                txn.commands.push(CommandRecord {
                    envelope: env,
                    ok: false,
                    output: serde_json::Value::Null,
                    error: Some(e.to_string()),
                });
                if auto {
                    // Atomicity: revert whatever this command touched.
                    let _ = self.rollback_transaction();
                }
                Err(e.into())
            }
        }
    }

    /// Dependency-driven staleness: if THIS command touched an object that
    /// a requirement `core:depends-on`, flip that requirement's status to
    /// `stale`. Only ops appended by this command (`ops[ops_before..]`)
    /// count — earlier ops in a shared transaction already marked their
    /// own dependents. Recorded as ops in the SAME transaction, so the
    /// marking is atomic and undoes together with the change that caused it.
    fn mark_stale_dependents(&mut self, command_type: &str, actor: &Actor, ops_before: usize) {
        // evaluate/set_status write fresh status themselves — don't stomp it.
        if matches!(
            command_type,
            "requirement.evaluate" | "requirement.set_status"
        ) {
            return;
        }
        let Some(txn) = self.open_txn.as_ref() else {
            return;
        };
        let touched: std::collections::BTreeSet<worldos_kernel::ObjectId> = txn
            .ops
            .iter()
            .skip(ops_before)
            .filter_map(|op| match op {
                worldos_kernel::StateOp::SetObject { before, after } => after
                    .as_ref()
                    .map(|o| o.id)
                    .or_else(|| before.as_ref().map(|o| o.id)),
                _ => None,
            })
            .collect();
        if touched.is_empty() {
            return;
        }
        let mut req_ids: Vec<worldos_kernel::ObjectId> = self
            .project
            .relations
            .values()
            .filter(|r| {
                r.type_id == worldos_kernel::known::rel::DEPENDS_ON && touched.contains(&r.to)
            })
            .map(|r| r.from)
            .collect();
        // a requirement touched directly (e.g. expression edit) stales too —
        // deleted requirements simply fail the update_object below.
        req_ids.extend(touched.iter().copied());
        req_ids.retain(|id| {
            self.project
                .get(*id)
                .and_then(|o| {
                    o.component_data(worldos_kernel::known::components::REQUIREMENT_STATUS)
                })
                .and_then(|c| c.get("status"))
                .and_then(|s| s.as_str())
                .is_some_and(|s| matches!(s, "pass" | "fail"))
        });
        req_ids.sort();
        req_ids.dedup();
        if req_ids.is_empty() {
            return;
        }
        let txn = self.open_txn.as_mut().unwrap();
        let mut ctx = worldos_commands::CommandContext {
            project: &mut self.project,
            ops: &mut txn.ops,
            actor,
            registry: &self.registry,
        };
        for id in req_ids {
            let _ = ctx.update_object(id, |o| {
                if let Some(c) = o
                    .components
                    .get_mut(worldos_kernel::known::components::REQUIREMENT_STATUS)
                {
                    c.data["status"] = serde_json::json!("stale");
                    let prev = c.data["verdict"].as_str().unwrap_or_default().to_string();
                    c.data["verdict"] =
                        serde_json::json!(format!("{prev} [stale: dependency changed]").trim());
                }
            });
        }
    }

    // ----- transactions ---------------------------------------------------

    pub fn begin_transaction(
        &mut self,
        label: impl Into<String>,
    ) -> Result<TransactionId, EngineError> {
        let actor = self.actor.clone();
        self.begin_transaction_as(&actor, label)
    }

    /// Begin a transaction attributed to a specific actor — agents use
    /// this so history records them, not the session user.
    pub fn begin_transaction_as(
        &mut self,
        actor: &Actor,
        label: impl Into<String>,
    ) -> Result<TransactionId, EngineError> {
        if self.open_txn.is_some() {
            return Err(EngineError::TransactionAlreadyOpen);
        }
        let txn = Transaction::new(actor.id.clone(), label);
        let id = txn.id;
        self.open_txn = Some(txn);
        Ok(id)
    }

    pub fn commit_transaction(&mut self) -> Result<TransactionId, EngineError> {
        let txn = self.open_txn.take().ok_or(EngineError::NoOpenTransaction)?;
        let id = txn.id;
        if !txn.ops.is_empty() || !txn.commands.is_empty() {
            let rec = txn.finish(self.history.next_index());
            let affected = rec
                .ops
                .iter()
                .filter_map(|op| match op {
                    worldos_kernel::StateOp::SetObject { before, after } => after
                        .as_ref()
                        .map(|o| o.id)
                        .or_else(|| before.as_ref().map(|o| o.id)),
                    _ => None,
                })
                .collect::<Vec<_>>();
            let actor = rec.actor.clone();
            let label = rec.label.clone();
            let n = rec.commands.len();
            self.history.push(rec);
            self.dirty = true;
            self.emit(EngineEvent::TransactionCommitted {
                transaction_id: id,
                actor,
                label,
                command_count: n,
                affected,
            });
        }
        Ok(id)
    }

    pub fn rollback_transaction(&mut self) -> Result<(), EngineError> {
        let txn = self.open_txn.take().ok_or(EngineError::NoOpenTransaction)?;
        txn.revert(&mut self.project);
        Ok(())
    }

    pub fn in_transaction(&self) -> bool {
        self.open_txn.is_some()
    }

    // ----- undo / redo ------------------------------------------------------

    pub fn undo(&mut self) -> Result<Option<TransactionId>, EngineError> {
        if self.open_txn.is_some() {
            return Err(EngineError::TransactionOpen { action: "undo" });
        }
        let id = self.history.undo(&mut self.project);
        if let Some(id) = id {
            self.dirty = true;
            self.emit(EngineEvent::TransactionUndone { transaction_id: id });
        }
        Ok(id)
    }

    pub fn redo(&mut self) -> Result<Option<TransactionId>, EngineError> {
        if self.open_txn.is_some() {
            return Err(EngineError::TransactionOpen { action: "redo" });
        }
        let id = self.history.redo(&mut self.project);
        if let Some(id) = id {
            self.dirty = true;
            self.emit(EngineEvent::TransactionRedone { transaction_id: id });
        }
        Ok(id)
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }
    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    // ----- capabilities ----------------------------------------------------

    pub fn run_capability(
        &mut self,
        id: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        let actor = self.actor.clone();
        self.run_capability_as(&actor, id, input)
    }

    pub fn run_capability_as(
        &mut self,
        actor: &Actor,
        id: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, EngineError> {
        let cap = self
            .capabilities
            .get(id)
            .ok_or_else(|| CapabilityError::Unknown(id.into()))?;
        let d = cap.descriptor();
        for perm in &d.permissions {
            if !actor.permissions.is_allowed(&Permission(perm.clone())) {
                return Err(CapabilityError::PermissionDenied {
                    capability: id.into(),
                    perm: perm.clone(),
                }
                .into());
            }
        }
        let errors = jsonschema::validate(&d.input_schema, &input, "$");
        if !errors.is_empty() {
            return Err(CapabilityError::Validation {
                capability: id.into(),
                errors,
            }
            .into());
        }
        Ok(cap.execute(self, &input)?)
    }

    // ----- validation --------------------------------------------------------

    pub fn validate(&self) -> ValidationReport {
        let mut runs = Vec::new();
        let mut diagnostics = Vec::new();
        for v in &self.validators {
            let t0 = std::time::Instant::now();
            let diags = v.validate(&self.project);
            runs.push(worldos_kernel::ValidatorRun {
                validator_id: v.id().into(),
                diagnostics: diags.len(),
                duration_ms: t0.elapsed().as_millis() as u64,
            });
            diagnostics.extend(diags);
        }
        ValidationReport::from_runs(runs, diagnostics)
    }

    // ----- snapshots ---------------------------------------------------------

    pub fn snapshot(&self) -> Snapshot {
        Snapshot::new(self.project.clone(), self.history.clone())
    }
}

impl CapabilityHost for Engine {
    fn project(&self) -> &Project {
        &self.project
    }
    fn actor(&self) -> &Actor {
        &self.actor
    }
    fn run_command(
        &mut self,
        command_type: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, CapabilityError> {
        self.execute(command_type, input)
            .map(|r| r.output)
            .map_err(|e| CapabilityError::Failed(e.to_string()))
    }
    fn run_command_as(
        &mut self,
        actor: &Actor,
        command_type: &str,
        input: serde_json::Value,
    ) -> Result<serde_json::Value, CapabilityError> {
        self.execute_as(actor, command_type, input)
            .map(|r| r.output)
            .map_err(|e| CapabilityError::Failed(e.to_string()))
    }
    fn begin_transaction(&mut self, label: &str) -> Result<(), CapabilityError> {
        Engine::begin_transaction(self, label)
            .map(|_| ())
            .map_err(|e| CapabilityError::Failed(e.to_string()))
    }
    fn begin_transaction_as(&mut self, actor: &Actor, label: &str) -> Result<(), CapabilityError> {
        Engine::begin_transaction_as(self, actor, label)
            .map(|_| ())
            .map_err(|e| CapabilityError::Failed(e.to_string()))
    }
    fn commit_transaction(&mut self) -> Result<(), CapabilityError> {
        Engine::commit_transaction(self)
            .map(|_| ())
            .map_err(|e| CapabilityError::Failed(e.to_string()))
    }
    fn rollback_transaction(&mut self) -> Result<(), CapabilityError> {
        Engine::rollback_transaction(self).map_err(|e| CapabilityError::Failed(e.to_string()))
    }
    fn in_transaction(&self) -> bool {
        Engine::in_transaction(self)
    }
    fn project_path(&self) -> Option<PathBuf> {
        self.path.clone()
    }
    fn command_schemas(&self) -> Vec<CommandSchema> {
        self.registry.schemas()
    }
    fn validate(&self) -> Result<ValidationReport, CapabilityError> {
        Ok(Engine::validate(self))
    }
}

fn default_validators() -> Vec<Box<dyn Validator>> {
    vec![
        Box::new(worldos_kernel::validation::RelationIntegrity),
        Box::new(worldos_kernel::validation::UniqueNames),
        Box::new(RequirementsStatus),
    ]
}

/// Warns when requirements are unevaluated or failing.
struct RequirementsStatus;

impl Validator for RequirementsStatus {
    fn id(&self) -> &'static str {
        "core:requirements"
    }
    fn description(&self) -> &'static str {
        "Requirements should evaluate to pass"
    }
    fn validate(&self, project: &Project) -> Vec<worldos_kernel::Diagnostic> {
        use worldos_kernel::known::{components, types};
        let mut out = Vec::new();
        for req in project.objects_of_type(types::REQUIREMENT) {
            let status = req
                .component_data(components::REQUIREMENT_STATUS)
                .and_then(|d| d.get("status"))
                .and_then(|s| s.as_str())
                .unwrap_or("unknown");
            match status {
                "fail" => out.push(
                    worldos_kernel::Diagnostic::error(
                        "requirement-failed",
                        format!("requirement `{}` fails", req.name),
                    )
                    .at(req.id),
                ),
                "unknown" | "stale" => out.push(
                    worldos_kernel::Diagnostic::warning(
                        "requirement-unevaluated",
                        format!("requirement `{}` is {status}", req.name),
                    )
                    .at(req.id)
                    .hint("run requirement.evaluate"),
                ),
                _ => {}
            }
        }
        out
    }
}
