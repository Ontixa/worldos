#![no_main]

//! Fuzz the requirement-expression parser/evaluator: arbitrary bytes
//! become the `expression` of a `core:requirement` object evaluated
//! against a small deterministic project. Must never panic or recurse
//! without bound (depth is capped inside `eval_not`).

use libfuzzer_sys::fuzz_target;
use worldos_kernel::ids::ActorId;
use worldos_kernel::known::{components, types};
use worldos_kernel::{Component, Object, Project};

fn fixture() -> Project {
    let actor = ActorId::new("fuzz");
    let mut p = Project::new("fuzz-project");
    let mut cube = Object::new(types::CUBE, "box", &actor);
    cube.set_component(Component::new(
        components::GEOMETRY,
        serde_json::json!({"kind": "cube", "size": [2.0, 2.0, 2.0]}),
    ));
    cube.set_component(Component::new(
        components::TRANSFORM,
        serde_json::json!({"position": [0.0, 0.0, 0.0]}),
    ));
    p.objects.insert(cube.id, cube);
    let mut sphere = Object::new(types::SPHERE, "ball", &actor);
    sphere.set_component(Component::new(
        components::GEOMETRY,
        serde_json::json!({"kind": "sphere", "size": [1.0, 1.0, 1.0]}),
    ));
    sphere.set_component(Component::new(
        components::TRANSFORM,
        serde_json::json!({"position": [3.0, 0.0, 0.0]}),
    ));
    p.objects.insert(sphere.id, sphere);
    let doc = Object::new(types::NOTE, "readme", &actor);
    p.objects.insert(doc.id, doc);
    p
}

fn probe(expr: &str) -> Object {
    let actor = ActorId::new("agent:goal-probe");
    let mut req = Object::new(types::REQUIREMENT, "agent-goal", &actor);
    req.set_component(Component::new(
        components::REQUIREMENT_EXPR,
        serde_json::json!({"expression": expr}),
    ));
    req
}

fuzz_target!(|data: &[u8]| {
    let expr = String::from_utf8_lossy(data);
    let project = fixture();
    let req = probe(&expr);
    let _ = worldos_kernel::requirement::evaluate(&project, &req);
    let _ = worldos_kernel::requirement::evaluate_traced(&project, &req);
});
