use super::{TYPST_IMPORT, typst_expression};
use crate::{
    org_syntax::math::Formula,
    typst_runtime::{CompiledFragment, TypstEngine},
};
use std::fmt::Write;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MathStyle {
    // Integer keys avoid floating-point equality and preserve subpixel font sizes.
    font_size_100: u32,
    foreground: u32,
}

impl MathStyle {
    pub(crate) fn new(font_size: f32, foreground: u32) -> Self {
        Self {
            font_size_100: (font_size.max(1.0) * 100.0).round() as u32,
            foreground,
        }
    }
    pub(crate) fn font_size(self) -> f32 {
        self.font_size_100 as f32 / 100.0
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) struct MathKey {
    pub(crate) formula: Formula,
    pub(crate) style: MathStyle,
}

pub(super) fn render_batch(
    engine: &TypstEngine,
    keys: &[MathKey],
) -> Vec<Result<CompiledFragment, String>> {
    if keys.is_empty() {
        return Vec::new();
    }
    let mut source = String::from(
        "#set page(width: auto, height: auto, margin: 0pt, fill: none)\n#set text(top-edge: \"bounds\", bottom-edge: \"bounds\")\n#set block(above: 0pt, below: 0pt)\n",
    );
    source.push_str(TYPST_IMPORT);
    for (index, key) in keys.iter().enumerate() {
        if index > 0 {
            source.push_str("\n#pagebreak()\n");
        }
        let _ = writeln!(
            source,
            "#set text(size: {}pt, fill: rgb(\"#{:06x}\"))\n#box({})",
            key.style.font_size() * 0.75,
            key.style.foreground,
            typst_expression(&key.formula)
        );
    }
    match engine.compile_fragments(source) {
        Ok(fragments) if fragments.len() == keys.len() => fragments.into_iter().map(Ok).collect(),
        // Isolate invalid formulas without discarding valid neighbors. Successful batches
        // take one compilation; only a failing batch is split.
        Err(_) if keys.len() > 1 => {
            let middle = keys.len() / 2;
            let mut results = render_batch(engine, &keys[..middle]);
            results.extend(render_batch(engine, &keys[middle..]));
            results
        }
        Err(error) => vec![Err(error.to_string())],
        Ok(_) => keys
            .iter()
            .map(|_| Err("unexpected formula page count".into()))
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::org_syntax::math::MathMode;

    #[test]
    fn math_batch_has_complete_bounds_baselines_and_isolates_invalid_formulas() {
        let keys = [
            (r"x^2", MathMode::Inline),
            (r"\frac{a}{b}", MathMode::Inline),
            (r"\begin{pmatrix}a&b\\c&d\end{pmatrix}", MathMode::Display),
        ]
        .map(|(body, mode)| MathKey {
            formula: Formula::new(body, mode),
            style: MathStyle::new(16.0, 0x202020),
        });
        let output = render_batch(crate::typst_runtime::shared_engine(), &keys);
        assert_eq!(output.len(), keys.len());
        for result in &output {
            let fragment = result.as_ref().unwrap();
            assert!(fragment.width > 0.0 && fragment.height > 0.0);
            assert!(fragment.baseline > 0.0 && fragment.baseline <= fragment.height);
            assert!(std::str::from_utf8(&fragment.svg).unwrap().contains("<svg"));
        }
        let fraction = output[1].as_ref().unwrap();
        assert!(
            fraction.height > 10.0,
            "fraction must not be cropped to the font's cap height"
        );
        assert!(
            fraction.baseline < fraction.height,
            "the denominator extends below the baseline"
        );
        let mut invalid = keys[0].clone();
        invalid.formula.body = r"\definitelyUnsupportedCommand{x}".into();
        let output = render_batch(
            crate::typst_runtime::shared_engine(),
            &[keys[0].clone(), invalid, keys[2].clone()],
        );
        assert!(output[0].is_ok() && output[1].is_err() && output[2].is_ok());
    }
}
