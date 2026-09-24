//! Requirements as first-class, evaluable data.
//!
//! A requirement object (`core:requirement`) carries a `core:requirement-expr`
//! component with a small declarative expression. Grammar:
//!
//! ```text
//! expr        := or_expr
//! or_expr     := and_expr ("or" and_expr)*
//! and_expr    := not_expr ("and" not_expr)*
//! not_expr    := "not" not_expr | "not" "(" expr ")" | "(" expr ")" | cmp
//! cmp         := term (">="|"<="|"=="|"!="|">"|"<" term)?
//! term        := number | "true" | "false"
//!              | "exists(" type ")"            — ≥1 object of type exists
//!              | "exists_named(" name ")"      — object with name exists
//!              | "count(" type ")"             — cardinality of a type
//!              | "object(" name ")." comp "." path — numeric field access
//!              | "volume(" name ")"            — analytic or mesh volume
//!              | "area(" name ")"              — analytic or mesh area
//!              | "distance(" name "," name ")" — position distance
//! ```
//!
//! `evaluate` reports which objects the expression touched so callers can
//! trace `core:depends-on` edges from the requirement to its inputs.
//! Status is stored in `core:requirement-status` (`pass` | `fail` |
//! `unknown` | `stale`).

use crate::error::KernelError;
use crate::model::Object;
use crate::project::Project;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequirementStatus {
    Pass,
    Fail,
    Unknown,
    Stale,
}

/// Evaluate a requirement object against the project. Returns status and
/// a short human-readable verdict.
pub fn evaluate(project: &Project, req: &Object) -> (RequirementStatus, String) {
    evaluate_traced(project, req).0
}

/// Like [`evaluate`] but also returns the ids of every object the
/// expression referenced — used to maintain `core:depends-on` edges.
pub fn evaluate_traced(
    project: &Project,
    req: &Object,
) -> ((RequirementStatus, String), Vec<crate::ObjectId>) {
    let mut refs = BTreeSet::new();
    let out = evaluate_inner(project, req, &mut refs);
    (out, refs.into_iter().collect())
}

fn evaluate_inner(
    project: &Project,
    req: &Object,
    refs: &mut BTreeSet<crate::ObjectId>,
) -> (RequirementStatus, String) {
    let expr = req
        .component_data(crate::known::components::REQUIREMENT_EXPR)
        .and_then(|d| d.get("expression"))
        .and_then(|e| e.as_str())
        .unwrap_or("");
    if expr.trim().is_empty() {
        return (RequirementStatus::Unknown, "no expression".into());
    }
    match eval_expr(project, expr.trim(), refs) {
        Ok(true) => (RequirementStatus::Pass, format!("`{expr}` satisfied")),
        Ok(false) => (RequirementStatus::Fail, format!("`{expr}` not satisfied")),
        Err(e) => (
            RequirementStatus::Unknown,
            format!("cannot evaluate `{expr}`: {e}"),
        ),
    }
}

// ------------------------------------------------------------------ grammar

/// Bound on recursive descent: `not`/paren nesting deeper than this fails
/// with `InvalidInput` instead of overflowing the stack on hostile input.
const MAX_EXPR_DEPTH: usize = 128;

fn eval_expr(
    project: &Project,
    expr: &str,
    refs: &mut BTreeSet<crate::ObjectId>,
) -> Result<bool, KernelError> {
    eval_or(project, expr.trim(), refs, 0)
}

/// `a or b or …` — split at top-level ` or ` only (not inside parens/quotes).
fn eval_or(
    project: &Project,
    s: &str,
    refs: &mut BTreeSet<crate::ObjectId>,
    depth: usize,
) -> Result<bool, KernelError> {
    for part in split_top(s, " or ") {
        if eval_and(project, part, refs, depth)? {
            return Ok(true);
        }
    }
    Ok(false)
}

fn eval_and(
    project: &Project,
    s: &str,
    refs: &mut BTreeSet<crate::ObjectId>,
    depth: usize,
) -> Result<bool, KernelError> {
    for part in split_top(s, " and ") {
        if !eval_not(project, part, refs, depth)? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn eval_not(
    project: &Project,
    s: &str,
    refs: &mut BTreeSet<crate::ObjectId>,
    depth: usize,
) -> Result<bool, KernelError> {
    if depth > MAX_EXPR_DEPTH {
        return Err(KernelError::InvalidInput(
            "expression nested too deeply".into(),
        ));
    }
    let s = s.trim();
    if let Some(rest) = s.strip_prefix("not ") {
        return Ok(!eval_not(project, rest, refs, depth + 1)?);
    }
    if let Some(rest) = s.strip_prefix("not(")
        && let Some(inner) = rest.strip_suffix(')')
        && balanced(inner)
    {
        return Ok(!eval_or(project, inner.trim(), refs, depth + 1)?);
    }
    if s.starts_with('(') && s.ends_with(')') && balanced(&s[1..s.len() - 1]) {
        return eval_or(project, s[1..s.len() - 1].trim(), refs, depth + 1);
    }
    eval_cmp(project, s, refs)
}

/// Split `s` on `sep` occurring at paren-depth 0 outside quotes.
/// `i` only ever lands on char boundaries: it advances by whole chars,
/// or by `sep.len()` on a match (separators are ASCII).
fn split_top<'a>(s: &'a str, sep: &str) -> Vec<&'a str> {
    let mut parts = Vec::new();
    let mut depth = 0i32;
    let mut quote = None;
    let mut start = 0;
    let mut i = 0;
    while i < s.len() {
        let c = s[i..].chars().next().unwrap();
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '(' => depth += 1,
            None if c == ')' => depth -= 1,
            None if depth == 0 && s[i..].starts_with(sep) => {
                parts.push(&s[start..i]);
                i += sep.len();
                start = i;
                continue;
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    parts.push(&s[start..]);
    parts
}

/// True when the substring is balanced parenthesis-wise (used to confirm
/// outer parens wrap the whole expression).
fn balanced(s: &str) -> bool {
    let mut depth = 0i32;
    let mut quote = None;
    for c in s.chars() {
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '(' => depth += 1,
            None if c == ')' => {
                depth -= 1;
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0 && quote.is_none()
}

// ------------------------------------------------------------------ compare

fn eval_cmp(
    project: &Project,
    expr: &str,
    refs: &mut BTreeSet<crate::ObjectId>,
) -> Result<bool, KernelError> {
    let (lhs, op, rhs) = split_cmp(expr)?;
    let lhs_val = eval_term(project, lhs.trim(), refs)?;
    let rhs_val = eval_term(project, rhs.trim(), refs)?;
    Ok(match op {
        ">=" => lhs_val >= rhs_val,
        "<=" => lhs_val <= rhs_val,
        ">" => lhs_val > rhs_val,
        "<" => lhs_val < rhs_val,
        "==" => (lhs_val - rhs_val).abs() < f64::EPSILON,
        "!=" => (lhs_val - rhs_val).abs() >= f64::EPSILON,
        _ => return Err(KernelError::InvalidInput(format!("bad operator {op}"))),
    })
}

/// Find a top-level comparison operator (skipping parens/quotes so
/// `!=` inside `object(...)` args can't split early).
/// `i` only ever lands on char boundaries (same discipline as
/// [`split_top`]); multi-byte terms can't panic the slice.
fn split_cmp(expr: &str) -> Result<(&str, &str, &str), KernelError> {
    let mut depth = 0i32;
    let mut quote = None;
    let mut i = 0;
    while i < expr.len() {
        let c = expr[i..].chars().next().unwrap();
        match quote {
            Some(q) if c == q => quote = None,
            Some(_) => {}
            None if c == '"' || c == '\'' => quote = Some(c),
            None if c == '(' => depth += 1,
            None if c == ')' => depth -= 1,
            None if depth == 0 => {
                for op in [">=", "<=", "==", "!=", ">", "<"] {
                    if expr[i..].starts_with(op) {
                        return Ok((&expr[..i], op, &expr[i + op.len()..]));
                    }
                }
            }
            _ => {}
        }
        i += c.len_utf8();
    }
    Ok((expr, "==", "true"))
}

// ------------------------------------------------------------------ terms

/// Evaluate a term to f64 (1.0 = true, 0.0 = false for boolean forms).
fn eval_term(
    project: &Project,
    term: &str,
    refs: &mut BTreeSet<crate::ObjectId>,
) -> Result<f64, KernelError> {
    let term = term.trim();
    if let Ok(n) = term.parse::<f64>() {
        return Ok(n);
    }
    match term {
        "true" => return Ok(1.0),
        "false" => return Ok(0.0),
        _ => {}
    }
    if let Some(args) = call_args(term, "exists") {
        let t = unquote(args);
        let objs: Vec<_> = project.objects_of_type(t).collect();
        refs.extend(objs.iter().map(|o| o.id));
        return Ok((!objs.is_empty()) as i32 as f64);
    }
    if let Some(args) = call_args(term, "exists_named") {
        let n = unquote(args);
        return Ok(match project.find_by_name(n) {
            Some(o) => {
                refs.insert(o.id);
                1.0
            }
            None => 0.0,
        });
    }
    if let Some(args) = call_args(term, "count") {
        let t = unquote(args);
        let objs: Vec<_> = project.objects_of_type(t).collect();
        refs.extend(objs.iter().map(|o| o.id));
        return Ok(objs.len() as f64);
    }
    if let Some(args) = call_args(term, "volume") {
        let obj = find_named(project, unquote(args), refs)?;
        return Ok(crate::measure::object_measures(obj)
            .ok_or_else(|| KernelError::InvalidInput("no geometry".into()))?
            .0);
    }
    if let Some(args) = call_args(term, "area") {
        let obj = find_named(project, unquote(args), refs)?;
        return Ok(crate::measure::object_measures(obj)
            .ok_or_else(|| KernelError::InvalidInput("no geometry".into()))?
            .1);
    }
    if let Some(args) = call_args(term, "distance") {
        let (a, b) = args
            .split_once(',')
            .ok_or_else(|| KernelError::InvalidInput("distance() needs two names".into()))?;
        let oa = find_named(project, unquote(a.trim()), refs)?;
        let ob = find_named(project, unquote(b.trim()), refs)?;
        return Ok(crate::measure::distance(oa, ob));
    }
    if let Some(inner) = term.strip_prefix("object(") {
        // object(<name>).<component>.<prop-path>
        let (name, rest) = inner
            .split_once(')')
            .ok_or_else(|| KernelError::InvalidInput("bad object() term".into()))?;
        let obj = find_named(project, unquote(name), refs)?;
        let path = rest.trim_start_matches('.');
        let mut segments = path.split('.');
        let component = segments
            .next()
            .ok_or_else(|| KernelError::InvalidInput("missing component".into()))?;
        let mut val = obj
            .component_data(component)
            .ok_or_else(|| KernelError::InvalidInput(format!("no component {component}")))?;
        for seg in segments {
            val = val
                .get(seg)
                .ok_or_else(|| KernelError::InvalidInput(format!("no path {path}")))?;
        }
        return val
            .as_f64()
            .ok_or_else(|| KernelError::InvalidInput(format!("{path} is not numeric")));
    }
    Err(KernelError::InvalidInput(format!("unknown term `{term}`")))
}

/// `<fn>(args)` — returns the inner arg string for a bare call term.
fn call_args<'a>(term: &'a str, f: &str) -> Option<&'a str> {
    term.strip_prefix(f)
        .and_then(|r| r.strip_prefix('('))
        .and_then(|r| r.strip_suffix(')'))
}

fn unquote(s: &str) -> &str {
    s.trim().trim_matches('"').trim_matches('\'')
}

fn find_named<'a>(
    project: &'a Project,
    name: &str,
    refs: &mut BTreeSet<crate::ObjectId>,
) -> Result<&'a Object, KernelError> {
    let obj = project
        .find_by_name(name)
        .ok_or_else(|| KernelError::ObjectNotFound(name.into()))?;
    refs.insert(obj.id);
    Ok(obj)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::known::{components, types};

    fn probe(expr: &str) -> Object {
        let actor = crate::ids::ActorId::new("test");
        let mut req = Object::new(types::REQUIREMENT, "req", &actor);
        req.set_component(crate::Component::new(
            components::REQUIREMENT_EXPR,
            serde_json::json!({"expression": expr}),
        ));
        req
    }

    #[test]
    fn deep_parens_fail_instead_of_overflowing() {
        let project = Project::new("t");
        let expr = format!("{}true{}", "(".repeat(5000), ")".repeat(5000));
        let (status, verdict) = evaluate(&project, &probe(&expr));
        assert_eq!(status, RequirementStatus::Unknown);
        assert!(verdict.contains("nested too deeply"), "{verdict}");
    }

    #[test]
    fn deep_not_chain_fails_instead_of_overflowing() {
        let project = Project::new("t");
        let expr = format!("{}true", "not ".repeat(5000));
        let (status, verdict) = evaluate(&project, &probe(&expr));
        assert_eq!(status, RequirementStatus::Unknown);
        assert!(verdict.contains("nested too deeply"), "{verdict}");
    }

    #[test]
    fn multibyte_input_does_not_panic() {
        // Regression: split_top/split_cmp used to walk raw bytes and
        // slice `s[i..]` mid-char — any multi-byte term panicked.
        let project = Project::new("t");
        for expr in [
            "�",
            "é >= 1",
            "exists(geom:cube) or 日本語",
            "count(geom:cube) and caf\u{00e9}",
            "'💥' == 1",
        ] {
            let _ = evaluate(&project, &probe(expr));
        }
    }

    #[test]
    fn moderate_nesting_still_evaluates() {
        let project = Project::new("t");
        let expr = format!("{}1 >= 1{}", "(".repeat(64), ")".repeat(64));
        let (status, _) = evaluate(&project, &probe(&expr));
        assert_eq!(status, RequirementStatus::Pass);
    }
}
