//! Requirements as first-class, evaluable data.
//!
//! A requirement object (`core:requirement`) carries a `core:requirement-expr`
//! component with a small declarative expression. v0.1 supports:
//!
//!   `exists(<type>)`                 — at least one object of type exists
//!   `exists_named(<name>)`           — object with exact name exists
//!   `count(<type>) >=|<=|==|>|< N`   — cardinality constraints
//!   `object(<name>).<component>.<prop> >=|<=|==|>|<|!= value`
//!
//! Status is computed by `evaluate` and stored in
//! `core:requirement-status` (`pass` | `fail` | `unknown` | `stale`).

use crate::error::KernelError;
use crate::model::Object;
use crate::project::Project;
use serde::{Deserialize, Serialize};

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
    let expr = req
        .component_data(crate::known::components::REQUIREMENT_EXPR)
        .and_then(|d| d.get("expression"))
        .and_then(|e| e.as_str())
        .unwrap_or("");
    if expr.trim().is_empty() {
        return (RequirementStatus::Unknown, "no expression".into());
    }
    match eval_expr(project, expr.trim()) {
        Ok(true) => (RequirementStatus::Pass, format!("`{expr}` satisfied")),
        Ok(false) => (RequirementStatus::Fail, format!("`{expr}` not satisfied")),
        Err(e) => (
            RequirementStatus::Unknown,
            format!("cannot evaluate `{expr}`: {e}"),
        ),
    }
}

fn eval_expr(project: &Project, expr: &str) -> Result<bool, KernelError> {
    let (lhs, op, rhs) = split_cmp(expr)?;
    let lhs_val = eval_term(project, lhs.trim())?;
    let rhs_val = eval_term(project, rhs.trim())?;
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

fn split_cmp(expr: &str) -> Result<(&str, &str, &str), KernelError> {
    for op in [">=", "<=", "==", "!=", ">", "<"] {
        if let Some(i) = expr.find(op) {
            return Ok((&expr[..i], op, &expr[i + op.len()..]));
        }
    }
    // No comparator: treat whole expression as a boolean term.
    Ok((expr, "==", "true"))
}

/// Evaluate a term to f64 (1.0 = true, 0.0 = false for boolean forms).
fn eval_term(project: &Project, term: &str) -> Result<f64, KernelError> {
    let term = term.trim();
    if let Ok(n) = term.parse::<f64>() {
        return Ok(n);
    }
    if term == "true" {
        return Ok(1.0);
    }
    if term == "false" {
        return Ok(0.0);
    }
    if let Some(inner) = term
        .strip_prefix("exists(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let t = inner.trim().trim_matches('"').trim_matches('\'');
        return Ok(if project.objects_of_type(t).next().is_some() {
            1.0
        } else {
            0.0
        });
    }
    if let Some(inner) = term
        .strip_prefix("exists_named(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let n = inner.trim().trim_matches('"').trim_matches('\'');
        return Ok(if project.find_by_name(n).is_some() {
            1.0
        } else {
            0.0
        });
    }
    if let Some(inner) = term
        .strip_prefix("count(")
        .and_then(|s| s.strip_suffix(')'))
    {
        let t = inner.trim().trim_matches('"').trim_matches('\'');
        return Ok(project.objects_of_type(t).count() as f64);
    }
    if let Some(inner) = term.strip_prefix("object(") {
        // object(<name>).<component>.<prop-path>
        let (name, rest) = inner
            .split_once(')')
            .ok_or_else(|| KernelError::InvalidInput("bad object() term".into()))?;
        let name = name.trim().trim_matches('"').trim_matches('\'');
        let obj = project
            .find_by_name(name)
            .ok_or_else(|| KernelError::ObjectNotFound(name.into()))?;
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
