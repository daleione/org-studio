use std::ops::Range;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExportSeverity {
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct ExportDiagnostic {
    pub severity: ExportSeverity,
    pub code: &'static str,
    pub message: String,
    pub source: Option<Range<usize>>,
}

impl ExportDiagnostic {
    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: ExportSeverity::Warning,
            code,
            message: message.into(),
            source: None,
        }
    }

    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: ExportSeverity::Error,
            code,
            message: message.into(),
            source: None,
        }
    }
}
