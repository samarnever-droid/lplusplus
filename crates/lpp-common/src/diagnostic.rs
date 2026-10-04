//! Structured diagnostics shared by every rewrite stage.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::{SourceMap, Span};

/// Stable machine-readable diagnostic code such as `E0001`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DiagnosticCode(String);

impl DiagnosticCode {
    pub fn new(code: impl Into<String>) -> Result<Self, InvalidDiagnosticCode> {
        let code = code.into();
        let bytes = code.as_bytes();
        if bytes.len() == 5
            && bytes[0].is_ascii_uppercase()
            && bytes[1..].iter().all(u8::is_ascii_digit)
        {
            Ok(Self(code))
        } else {
            Err(InvalidDiagnosticCode(code))
        }
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for DiagnosticCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidDiagnosticCode(String);

impl fmt::Display for InvalidDiagnosticCode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "diagnostic code '{}' must be one uppercase letter followed by four digits",
            self.0
        )
    }
}

impl std::error::Error for InvalidDiagnosticCode {}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Error,
    Warning,
    Note,
    Help,
}

impl fmt::Display for Severity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Note => "note",
            Self::Help => "help",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Label {
    pub span: Span,
    pub message: String,
}

impl Label {
    #[must_use]
    pub fn new(span: Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }
}

/// An owned diagnostic. Rendering is deliberately deferred to the driver so
/// stage behavior never depends on prose formatting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diagnostic {
    pub code: DiagnosticCode,
    pub severity: Severity,
    pub message: String,
    pub primary_span: Option<Span>,
    pub labels: Vec<Label>,
    pub notes: Vec<String>,
    pub help: Vec<String>,
}

impl Diagnostic {
    pub fn error(
        code: impl Into<String>,
        message: impl Into<String>,
    ) -> Result<Self, InvalidDiagnosticCode> {
        Ok(Self {
            code: DiagnosticCode::new(code)?,
            severity: Severity::Error,
            message: message.into(),
            primary_span: None,
            labels: Vec::new(),
            notes: Vec::new(),
            help: Vec::new(),
        })
    }

    #[must_use]
    pub fn with_primary_span(mut self, span: Span) -> Self {
        self.primary_span = Some(span);
        self
    }

    #[must_use]
    pub fn with_label(mut self, label: Label) -> Self {
        self.labels.push(label);
        self
    }

    #[must_use]
    pub fn with_note(mut self, note: impl Into<String>) -> Self {
        self.notes.push(note.into());
        self
    }

    #[must_use]
    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help.push(help.into());
        self
    }

    /// Deterministic human renderer used by the new driver boundary.
    #[must_use]
    pub fn render_human(&self, sources: &SourceMap) -> String {
        use std::fmt::Write as _;

        let mut output = String::new();
        let _ = writeln!(output, "{}[{}]: {}", self.severity, self.code, self.message);
        if let Some(span) = self.primary_span
            && let Ok(file) = sources.file(span.file)
            && let Ok((line, column)) = file.line_column(span.start)
        {
            let _ = writeln!(output, "  --> {}:{line}:{column}", file.name());
            if let Ok(source_line) = file.line_at(span.start) {
                let padding = " ".repeat(line.to_string().len());
                let _ = writeln!(output, "   {padding} |");
                let _ = writeln!(output, "{line} | {source_line}");
                let caret_padding = " ".repeat(column.saturating_sub(1));
                let _ = writeln!(output, "   {padding} | {caret_padding}^");
            }
        }
        for label in &self.labels {
            let _ = writeln!(output, "  = label: {}", label.message);
        }
        for note in &self.notes {
            let _ = writeln!(output, "  = note: {note}");
        }
        for help in &self.help {
            let _ = writeln!(output, "  = help: {help}");
        }
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_unstable_code_shapes() {
        assert!(DiagnosticCode::new("E0001").is_ok());
        assert!(DiagnosticCode::new("error-1").is_err());
        assert!(DiagnosticCode::new("e0001").is_err());
    }

    #[test]
    fn renders_with_source_location() {
        let mut sources = SourceMap::new();
        let file = sources
            .add_file("src/main.lpp", "def main():\n    missing()\n")
            .unwrap();
        let start = "def main():\n    ".len();
        let span = sources.span(file, start, start + "missing".len()).unwrap();
        let diagnostic = Diagnostic::error("E0003", "unknown function 'missing'")
            .unwrap()
            .with_primary_span(span)
            .with_help("import or define the function");

        assert_eq!(
            diagnostic.render_human(&sources),
            "error[E0003]: unknown function 'missing'\n  --> src/main.lpp:2:5\n     |\n2 |     missing()\n     |     ^\n  = help: import or define the function\n"
        );
    }
}
