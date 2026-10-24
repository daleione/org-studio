//! Shared LaTeX rendering. Source syntax remains in `org_syntax::math`.
mod render;
mod service;

pub(crate) use render::{MathKey, MathStyle};
pub(crate) use service::{MathDemand, MathService, RenderedFormula, cached_formula};

use crate::org_syntax::math::{Formula, MathMode};

pub(crate) const TYPST_IMPORT: &str =
    "#import \"@preview/mitex:0.2.7\": mimath as org-studio-math\n";

pub(crate) fn typst_expression(formula: &Formula) -> String {
    format!(
        "org-studio-math({}, block: {})",
        crate::typst_runtime::string_literal(&formula.body),
        formula.mode == MathMode::Display
    )
}
