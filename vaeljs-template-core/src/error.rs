//! Structured errors returned by template compilation, composition, and rendering.

/// Errors produced while parsing, composing, or rendering templates.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TemplateError {
    /// Template syntax or structure could not be parsed.
    #[error("template parse error: {0}")]
    Parse(String),
    /// Template evaluation or helper execution failed.
    #[error("template render error: {0}")]
    Render(String),
    /// A configured safety or resource bound was exceeded.
    #[error("template limit exceeded: {0}")]
    LimitExceeded(String),
    /// A named template or partial was not found.
    #[error("template not found: {0}")]
    TemplateNotFound(String),
    /// A template name was empty or invalid.
    #[error("invalid template name: {0}")]
    InvalidName(String),
    /// Input data could not be converted into template context values.
    #[error("invalid template data: {0}")]
    InvalidData(String),
}

impl From<neotales_gotmpl::Error> for TemplateError {
    fn from(error: neotales_gotmpl::Error) -> Self {
        match error {
            neotales_gotmpl::Error::UnterminatedAction
            | neotales_gotmpl::Error::UnterminatedComment
            | neotales_gotmpl::Error::EmptyAction
            | neotales_gotmpl::Error::MalformedTrimMarker
            | neotales_gotmpl::Error::InvalidQuotedString
            | neotales_gotmpl::Error::CharacterLiteral
            | neotales_gotmpl::Error::UnsupportedAction(_)
            | neotales_gotmpl::Error::UnexpectedControl(_)
            | neotales_gotmpl::Error::DuplicateTemplate(_)
            | neotales_gotmpl::Error::RecursiveTemplate(_) => Self::Parse(error.to_string()),
            neotales_gotmpl::Error::MissingTemplate(name) => Self::TemplateNotFound(name),
            neotales_gotmpl::Error::ResourceLimitExceeded => Self::LimitExceeded(error.to_string()),
            neotales_gotmpl::Error::MissingValue(_)
            | neotales_gotmpl::Error::UnknownFunction(_)
            | neotales_gotmpl::Error::Function { .. }
            | neotales_gotmpl::Error::EmptyGroupName
            | neotales_gotmpl::Error::MissingVariable(_)
            | neotales_gotmpl::Error::Cancelled => Self::Render(error.to_string()),
        }
    }
}
