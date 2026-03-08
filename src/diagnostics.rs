use crate::span::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Option<Span>,
}

#[derive(Default, Debug)]
pub struct Diagnostics {
    items: Vec<Diagnostic>,
}

impl Diagnostics {
    pub fn warning(&mut self, span: Option<Span>, message: impl Into<String>) {
        self.items.push(Diagnostic {
            severity: Severity::Warning,
            message: message.into(),
            span,
        });
    }

    pub fn error(&mut self, span: Option<Span>, message: impl Into<String>) {
        self.items.push(Diagnostic {
            severity: Severity::Error,
            message: message.into(),
            span,
        });
    }

    pub fn has_errors(&self) -> bool {
        self.items
            .iter()
            .any(|diagnostic| diagnostic.severity == Severity::Error)
    }

    pub fn errors_count(&self) -> usize {
        self.items
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Error)
            .count()
    }

    pub fn warnings_count(&self) -> usize {
        self.items
            .iter()
            .filter(|diagnostic| diagnostic.severity == Severity::Warning)
            .count()
    }

    pub fn items(&self) -> &[Diagnostic] {
        &self.items
    }
}
