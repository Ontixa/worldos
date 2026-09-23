//! Tool-use loop tests: bounded observe→plan→act→inspect iterations with
//! a declared goal predicate verified against live engine state.

use serde_json::json;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use worldos_agent::{
    AgentRuntime, Budget, Observation, PlanError, PlannedStep, Planner, RunSpec, RunStatus,
};
use worldos_engine::Engine;

fn step(command: &str, input: serde_json::Value) -> PlannedStep {
    PlannedStep {
        command: command.into(),
        input,
        note: command.to_string(),
    }
}

/// Scripted planner: every `plan_turn` pops the next queued batch. Once
/// the queue is empty it serves `on_empty` forever (default: nothing) —
/// like a stateless planner that has run out of ideas.
struct QueuePlanner {
    batches: Mutex<VecDeque<Result<Vec<PlannedStep>, PlanError>>>,
    on_empty: Vec<PlannedStep>,
}

impl QueuePlanner {
    fn of(batches: Vec<Vec<PlannedStep>>) -> Self {
        Self {
            batches: Mutex::new(batches.into_iter().map(Ok).collect()),
            on_empty: vec![],
        }
    }
    /// Repeats `batch` on every turn — a planner that cannot converge.
    fn repeating(batch: Vec<PlannedStep>) -> Self {
        Self {
            batches: Mutex::new(VecDeque::new()),
            on_empty: batch,
        }
    }
}

impl Planner for QueuePlanner {
    fn plan(
        &self,
        goal: &str,
        host: &dyn worldos_capability::CapabilityHost,
    ) -> Result<Vec<PlannedStep>, PlanError> {
        self.plan_turn(goal, host, &Observation::default())
    }
    fn plan_turn(
        &self,
        _goal: &str,
        _host: &dyn worldos_capability::CapabilityHost,
        _obs: &Observation<'_>,
    ) -> Result<Vec<PlannedStep>, PlanError> {
        Ok(self
            .batches
            .lock()
            .unwrap()
            .pop_front()
            .transpose()?
            .unwrap_or_else(|| self.on_empty.clone()))
    }
}

fn create_cube(name: &str) -> PlannedStep {
    step(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": name}),
    )
}

fn create_note(name: &str) -> PlannedStep {
    step("object.create", json!({"type": "core:note", "name": name}))
}

#[test]
fn goal_reached_over_multiple_iterations_commits_once() {
    let mut e = Engine::new("t");
    // The predicate needs two cubes; the planner delivers one per turn.
    let planner = QueuePlanner::of(vec![vec![create_cube("c1")], vec![create_cube("c2")]]);
    let rt = AgentRuntime::new(Box::new(planner));
    let spec = RunSpec::new("make two cubes").with_done_when("count(geom:cube) >= 2");
    let report = rt.run_spec(&mut e, &spec, "loop-bot", None);

    assert_eq!(report.status, RunStatus::Succeeded, "{}", report.summary);
    assert_eq!(report.iterations, 2, "two act cycles, then predicate pass");
    assert_eq!(report.steps.len(), 2);
    assert_eq!(report.done_when.as_deref(), Some("count(geom:cube) >= 2"));
    assert!(
        report
            .verification
            .iter()
            .any(|v| v.contains("goal predicate")),
        "verification should record the predicate pass: {:?}",
        report.verification
    );
    assert!(e.find_object("c1").is_some() && e.find_object("c2").is_some());
    // The whole run is ONE attributed, undoable transaction.
    assert_eq!(e.history().records.len(), 1);
    assert_eq!(e.history().records[0].actor.0, "agent:loop-bot");
    e.undo().unwrap();
    assert!(e.find_object("c1").is_none() && e.find_object("c2").is_none());
}

#[test]
fn predicate_checked_before_acting() {
    // Goal already true → the loop exits without ever consulting the
    // planner: observe precedes act.
    let mut e = Engine::new("t");
    e.execute(
        "geometry.create_primitive",
        json!({"kind": "cube", "name": "already"}),
    )
    .unwrap();
    let planner = QueuePlanner::repeating(vec![create_cube("should-not-exist")]);
    let rt = AgentRuntime::new(Box::new(planner));
    let spec = RunSpec::new("there must be a cube").with_done_when("exists_named(\"already\")");
    let report = rt.run_spec(&mut e, &spec, "t", None);

    assert_eq!(report.status, RunStatus::Succeeded, "{}", report.summary);
    assert_eq!(report.iterations, 0);
    assert!(report.steps.is_empty());
    assert!(e.find_object("should-not-exist").is_none());
}

#[test]
fn iteration_cap_exceeded_is_honest_failure_and_rolls_back() {
    let mut e = Engine::new("t");
    e.execute(
        "object.create",
        json!({"type": "core:note", "name": "keep"}),
    )
    .unwrap();
    let before = serde_json::to_value(e.project()).unwrap();
    let history_before = e.history().records.len();

    // Planner can only ever add one cube per turn; the goal needs 99.
    let planner = QueuePlanner::repeating(vec![create_cube("churn")]);
    let rt = AgentRuntime::new(Box::new(planner)).with_budget(Budget {
        max_commands: 32,
        max_iterations: 3,
    });
    let spec = RunSpec::new("reach 99 cubes").with_done_when("count(geom:cube) >= 99");
    let report = rt.run_spec(&mut e, &spec, "t", None);

    assert_eq!(report.status, RunStatus::Failed);
    assert_eq!(report.iterations, 3);
    assert!(
        report.summary.contains("iteration cap 3"),
        "{}",
        report.summary
    );
    // executed work is preserved as evidence even though it rolled back
    assert_eq!(report.steps.len(), 3);
    assert!(report.steps.iter().all(|s| s.ok));
    assert!(report.created_objects.is_empty());
    // …and the rollback is exact: project + history untouched.
    assert_eq!(serde_json::to_value(e.project()).unwrap(), before);
    assert_eq!(e.history().records.len(), history_before);
    assert!(!worldos_capability::CapabilityHost::in_transaction(&e));
}

#[test]
fn command_cap_exceeded_fails_before_partial_batch() {
    let mut e = Engine::new("t");
    let before = serde_json::to_value(e.project()).unwrap();

    let planner = QueuePlanner::of(vec![vec![
        create_note("n1"),
        create_note("n2"),
        create_note("n3"),
    ]]);
    let rt = AgentRuntime::new(Box::new(planner)).with_budget(Budget {
        max_commands: 2,
        max_iterations: 8,
    });
    let spec = RunSpec::new("fill it").with_done_when("count(core:note) >= 3");
    let report = rt.run_spec(&mut e, &spec, "t", None);

    assert_eq!(report.status, RunStatus::Failed);
    assert!(
        report.summary.contains("more commands"),
        "{}",
        report.summary
    );
    // The batch was refused up front — nothing ran, nothing lingers.
    assert!(report.steps.is_empty());
    assert_eq!(serde_json::to_value(e.project()).unwrap(), before);
    assert!(e.history().records.is_empty());
}

#[test]
fn command_cap_counts_across_iterations() {
    let mut e = Engine::new("t");
    let before = serde_json::to_value(e.project()).unwrap();

    // Two commands on turn 1 fit the budget; the next batch cannot.
    let planner = QueuePlanner::of(vec![
        vec![create_note("n1"), create_note("n2")],
        vec![create_note("n3"), create_note("n4")],
    ]);
    let rt = AgentRuntime::new(Box::new(planner)).with_budget(Budget {
        max_commands: 3,
        max_iterations: 8,
    });
    let spec = RunSpec::new("fill it").with_done_when("count(core:note) >= 4");
    let report = rt.run_spec(&mut e, &spec, "t", None);

    assert_eq!(report.status, RunStatus::Failed, "{}", report.summary);
    assert_eq!(report.iterations, 1);
    assert_eq!(report.steps.len(), 2, "committed steps remain as evidence");
    assert_eq!(serde_json::to_value(e.project()).unwrap(), before);
    assert!(e.history().records.is_empty());
}

/// A command that mutates the project but reports a bogus reference.
struct IncorrectReference(serde_json::Value);

impl worldos_commands::CommandHandler for IncorrectReference {
    fn schema(&self) -> worldos_commands::CommandSchema {
        worldos_commands::CommandSchema::write(
            "test.incorrect_reference",
            "test",
            "Create an object but report an incorrect reference",
            json!({"type": "object"}),
        )
    }
    fn execute(
        &self,
        ctx: &mut worldos_commands::CommandContext,
        _input: &serde_json::Value,
    ) -> Result<serde_json::Value, worldos_commands::CommandError> {
        ctx.run_sub(
            "object.create",
            json!({"type": "core:note", "name": "unverified"}),
        )?;
        Ok(json!({"id": self.0}))
    }
}

#[test]
fn invalid_reference_in_later_iteration_fails_and_rolls_back() {
    let mut e = Engine::new("t");
    e.execute(
        "object.create",
        json!({"type": "core:note", "name": "keep"}),
    )
    .unwrap();
    let before = serde_json::to_value(e.project()).unwrap();
    e.register_command(Arc::new(IncorrectReference(json!(
        worldos_kernel::ObjectId::new().to_string()
    ))));

    // Iteration 1 does real work; iteration 2 reports a dead id.
    let planner = QueuePlanner::of(vec![
        vec![create_note("first")],
        vec![step("test.incorrect_reference", json!({}))],
    ]);
    let rt = AgentRuntime::new(Box::new(planner));
    let spec = RunSpec::new("link it").with_done_when("exists_named(\"never\")");
    let report = rt.run_spec(&mut e, &spec, "t", None);

    assert_eq!(report.status, RunStatus::Failed);
    assert!(
        report.summary.contains("verification failed"),
        "{}",
        report.summary
    );
    assert!(
        report.summary.contains("does not exist"),
        "{}",
        report.summary
    );
    assert_eq!(report.steps.len(), 2);
    assert!(
        report.steps.iter().all(|s| s.ok),
        "commands ran — verification is what failed"
    );
    // Atomic: even iteration 1's legitimate work is gone — only the
    // setup command's record remains in history.
    assert_eq!(serde_json::to_value(e.project()).unwrap(), before);
    assert_eq!(e.history().records.len(), 1);
    assert!(!worldos_capability::CapabilityHost::in_transaction(&e));
}

#[test]
fn forbidden_command_is_denied_inside_the_loop() {
    // A read-only agent whose planner proposes a write: the capability
    // layer denies it — the run fails, nothing mutates.
    let mut e = Engine::new("t");
    let before = serde_json::to_value(e.project()).unwrap();
    let planner = QueuePlanner::repeating(vec![create_cube("denied-cube")]);
    let rt = AgentRuntime::new(Box::new(planner));
    let spec = RunSpec::new("build it").with_done_when("exists_named(\"denied-cube\")");
    let report = rt.run_spec(
        &mut e,
        &spec,
        "ro-bot",
        Some(&["project.read".into(), "project.search".into()]),
    );

    assert_eq!(report.status, RunStatus::Failed);
    assert!(
        report.summary.contains("permission denied"),
        "{}",
        report.summary
    );
    assert_eq!(report.steps.len(), 1);
    assert!(!report.steps[0].ok);
    assert_eq!(serde_json::to_value(e.project()).unwrap(), before);
    assert!(e.history().records.is_empty());
    assert!(e.find_object("denied-cube").is_none());
}

#[test]
fn unknown_command_from_planner_fails_the_run() {
    // A planner hallucinating an unregistered command fails at the
    // command boundary, not by corrupting state.
    let mut e = Engine::new("t");
    let before = serde_json::to_value(e.project()).unwrap();
    let planner = QueuePlanner::repeating(vec![step("system.format_disk", json!({}))]);
    let rt = AgentRuntime::new(Box::new(planner));
    let spec = RunSpec::new("wipe it").with_done_when("count(geom:cube) >= 1");
    let report = rt.run_spec(&mut e, &spec, "t", None);

    assert_eq!(report.status, RunStatus::Failed);
    assert!(
        report.summary.contains("unknown command"),
        "{}",
        report.summary
    );
    assert_eq!(serde_json::to_value(e.project()).unwrap(), before);
}

#[test]
fn dead_end_planner_fails_honestly() {
    // Predicate unsatisfied + planner has nothing left to propose →
    // immediate honest failure (no point burning the iteration cap).
    let mut e = Engine::new("t");
    let before = serde_json::to_value(e.project()).unwrap();
    let planner = QueuePlanner::of(vec![]);
    let rt = AgentRuntime::new(Box::new(planner));
    let spec = RunSpec::new("conjure a cube").with_done_when("count(geom:cube) >= 1");
    let report = rt.run_spec(&mut e, &spec, "t", None);

    assert_eq!(report.status, RunStatus::Failed);
    assert!(
        report.summary.contains("no further steps"),
        "{}",
        report.summary
    );
    assert_eq!(report.iterations, 0);
    assert_eq!(serde_json::to_value(e.project()).unwrap(), before);
}

#[test]
fn planner_error_mid_loop_rolls_back() {
    let mut e = Engine::new("t");
    let before = serde_json::to_value(e.project()).unwrap();
    let planner = QueuePlanner {
        batches: Mutex::new(VecDeque::from(vec![
            Ok(vec![create_note("wip")]),
            Err(PlanError::Failed("planner exploded".into())),
        ])),
        on_empty: vec![],
    };
    let rt = AgentRuntime::new(Box::new(planner));
    let spec = RunSpec::new("keep going").with_done_when("count(core:note) >= 5");
    let report = rt.run_spec(&mut e, &spec, "t", None);

    assert_eq!(report.status, RunStatus::Failed);
    assert!(
        report.summary.contains("planner exploded"),
        "{}",
        report.summary
    );
    assert_eq!(report.steps.len(), 1, "iteration-1 work kept as evidence");
    assert_eq!(serde_json::to_value(e.project()).unwrap(), before);
    assert!(e.history().records.is_empty());
}

#[test]
fn cross_iteration_references_resolve() {
    // $N refs span iterations: turn 2 creates a child of what turn 1 made.
    let mut e = Engine::new("t");
    let planner = QueuePlanner::of(vec![
        vec![create_note("folder-x")],
        vec![step(
            "object.create",
            json!({"type": "core:note", "name": "child", "parent": "$0.id"}),
        )],
    ]);
    let rt = AgentRuntime::new(Box::new(planner));
    let spec = RunSpec::new("create then nest").with_done_when("exists_named(\"child\")");
    let report = rt.run_spec(&mut e, &spec, "t", None);

    assert_eq!(report.status, RunStatus::Succeeded, "{}", report.summary);
    assert_eq!(report.iterations, 2);
    let parent = e.find_object("folder-x").unwrap();
    let child = e.find_object("child").unwrap();
    assert!(
        e.project()
            .children(parent.id)
            .iter()
            .any(|o| o.id == child.id),
        "child must be contained by the object created last iteration"
    );
}

#[test]
fn no_predicate_runs_single_pass() {
    // Compat mode: without done_when the run is one plan→act→verify pass
    // even though the planner would have more to say.
    let mut e = Engine::new("t");
    let planner = QueuePlanner::of(vec![vec![create_cube("one")], vec![create_cube("two")]]);
    let rt = AgentRuntime::new(Box::new(planner));
    let report = rt.run(&mut e, "make cubes", "t");

    assert_eq!(report.status, RunStatus::Succeeded, "{}", report.summary);
    assert_eq!(report.iterations, 1);
    assert_eq!(report.steps.len(), 1);
    assert!(e.find_object("one").is_some());
    assert!(e.find_object("two").is_none(), "second batch never ran");
}

#[test]
fn planner_observes_prior_steps_and_verdict() {
    // The observation handed to plan_turn carries iteration index, prior
    // records, the last predicate verdict and the predicate itself.
    #[derive(Debug, PartialEq)]
    struct Seen {
        iteration: usize,
        prior_steps: usize,
        has_verdict: bool,
        has_goal_expr: bool,
    }
    struct Sensing(Arc<Mutex<Vec<Seen>>>);
    impl Planner for Sensing {
        fn plan(
            &self,
            _g: &str,
            _h: &dyn worldos_capability::CapabilityHost,
        ) -> Result<Vec<PlannedStep>, PlanError> {
            unreachable!("the loop must call plan_turn")
        }
        fn plan_turn(
            &self,
            _g: &str,
            _h: &dyn worldos_capability::CapabilityHost,
            obs: &Observation<'_>,
        ) -> Result<Vec<PlannedStep>, PlanError> {
            self.0.lock().unwrap().push(Seen {
                iteration: obs.iteration,
                prior_steps: obs.steps.len(),
                has_verdict: obs.verdict.is_some(),
                has_goal_expr: obs.goal_expr.is_some(),
            });
            Ok(match obs.iteration {
                0 => vec![create_note("seen")],
                _ => vec![],
            })
        }
    }
    let mut e = Engine::new("t");
    let seen = Arc::new(Mutex::new(Vec::new()));
    let rt = AgentRuntime::new(Box::new(Sensing(seen.clone())));
    let spec = RunSpec::new("sense me").with_done_when("exists_named(\"seen\")");
    let report = rt.run_spec(&mut e, &spec, "t", None);

    assert_eq!(report.status, RunStatus::Succeeded, "{}", report.summary);
    assert_eq!(report.iterations, 1);
    // Planner consulted exactly once — at iteration 0 with no prior
    // steps but the predicate and its first (unsatisfied) verdict.
    assert_eq!(
        seen.lock().unwrap().as_slice(),
        &[Seen {
            iteration: 0,
            prior_steps: 0,
            has_verdict: true,
            has_goal_expr: true,
        }]
    );
}

#[test]
fn run_via_capability_accepts_done_when_and_caps() {
    // End-to-end through the agent.run capability the way CLI/RPC call it.
    let mut e = Engine::new("t");
    e.register_capability(Arc::new(worldos_agent::AgentRun));
    let out = e
        .run_capability(
            "agent.run",
            json!({
                "goal": "create a cube named via-cap",
                "agent": "cap-bot",
                "done_when": "exists_named(\"via-cap\")",
                "max_iterations": 4,
                "max_commands": 8
            }),
        )
        .unwrap();
    assert_eq!(out["status"], "succeeded", "{}", out["summary"]);
    assert_eq!(out["iterations"], 1);
    assert_eq!(out["done_when"], "exists_named(\"via-cap\")");
    assert!(e.find_object("via-cap").is_some());
}

#[test]
fn run_via_capability_reports_cap_failure() {
    let mut e = Engine::new("t");
    e.register_capability(Arc::new(worldos_agent::AgentRun));
    let before = serde_json::to_value(e.project()).unwrap();
    let out = e
        .run_capability(
            "agent.run",
            json!({
                "goal": "create a cube named x",
                "done_when": "count(geom:cube) >= 50",
                "max_iterations": 2,
                "max_commands": 4
            }),
        )
        .unwrap();
    // The rule planner adds one cube per pass; caps stop it honestly.
    assert_eq!(out["status"], "failed");
    assert!(out["summary"].as_str().unwrap().contains("cap"));
    assert_eq!(serde_json::to_value(e.project()).unwrap(), before);
    assert!(e.history().records.is_empty());
}
