//! Centralized tolerance policy (DECISIONS.md: no scattered `1e-6`
//! literals in domain code — every tolerance lives here).

/// Positional tolerance in mm. OCCT's `Precision::Confusion` default is
/// 1e-7; we hold a stricter-than-visible but achievable 1e-6 for
/// "are these the same point/length" comparisons in domain code.
pub const LINEAR_TOLERANCE_MM: f64 = 1e-6;

/// Volume comparisons in mm³.
pub const VOLUME_TOLERANCE_MM3: f64 = 1e-3;

/// Area comparisons in mm².
pub const AREA_TOLERANCE_MM2: f64 = 1e-3;

/// Angular tolerance in radians.
pub const ANGULAR_TOLERANCE_RAD: f64 = 1e-12;

/// Relative tolerance for "same measure" checks across regeneration
/// (kernel noise is allowed to wiggle the last decimals).
pub const RELATIVE_MEASURE_TOLERANCE: f64 = 1e-6;

/// `a ≈ b` within [`LINEAR_TOLERANCE_MM`].
pub fn approx_mm(a: f64, b: f64) -> bool {
    (a - b).abs() <= LINEAR_TOLERANCE_MM
}

/// `a ≈ b` within [`RELATIVE_MEASURE_TOLERANCE`] of the larger magnitude.
pub fn approx_relative(a: f64, b: f64) -> bool {
    let scale = a.abs().max(b.abs()).max(1.0);
    (a - b).abs() <= RELATIVE_MEASURE_TOLERANCE * scale
}
