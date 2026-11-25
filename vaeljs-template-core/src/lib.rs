//! Engine-neutral Go `text/template` and Sprig template core.
//!
//! This crate wraps [`neotales_gotmpl`] and [`neotales_sprig`] behind a pure
//! Rust API shared by V8 and `QuickJS` runtimes, with zero JavaScript engine
//! dependencies (`v8`, `deno_core`, `rquickjs`) and zero `tokio` dependencies.
//!
//! # Features
//!
//! * **Go template syntax**: dot paths, `if`/`else`, `range`, `with`,
//!   pipelines, variables, `define`/`template`/`block`, and `len`/`index`.
//! * **Safe Sprig helpers**: full deterministic [`neotales_sprig`] helper set
//!   with optional [`TemplateOptions::hermetic`] mode omitting clock- and
//!   environment-reading helpers (`now`, `ago`, `env`, etc.).
//! * **XSS-safe HTML helpers**: [`escape_html`], `escapeHtml` / `htmlEscape`
//!   template functions, `safeHtml` marker handling, and optional
//!   [`TemplateOptions::html_escape`] auto-escaping mode.
//! * **Bounded execution**: finite defaults for source length, nesting depth,
//!   output length, loop iterations, and helper function calls.
//! * **Layouts & partials**: [`TemplateSet`] composes named templates,
//!   partials, and `block` overrides deterministically.
//!
//! # Example
//!
//! ```
//! use vaeljs_template_core::{TemplateOptions, render};
//! use serde_json::json;
//!
//! let out = render(
//!     "Hello, {{ .name | upper }}!",
//!     &json!({ "name": "world" }),
//!     &TemplateOptions::default(),
//! )?;
//! assert_eq!(out, "Hello, WORLD!");
//! # Ok::<(), vaeljs_template_core::TemplateError>(())
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod html;
mod options;
mod set;
mod syntax;
mod value;

pub use error::TemplateError;
pub use html::escape_html;
pub use options::TemplateOptions;
pub use set::TemplateSet;
pub use value::{json_to_template_value, sprig_call};

use syntax::{lower_standalone, rewrite_html_escape};
use value::{build_func_map, json_object_to_map};

/// A pre-compiled template ready for repeated rendering against JSON data.
///
/// ```
/// use vaeljs_template_core::{CompiledTemplate, TemplateOptions};
/// use serde_json::json;
///
/// let tmpl = CompiledTemplate::compile("Hi {{ .user }}!", &TemplateOptions::default())?;
/// assert_eq!(tmpl.render(&json!({ "user": "Ada" }))?, "Hi Ada!");
/// assert_eq!(tmpl.render(&json!({ "user": "Grace" }))?, "Hi Grace!");
/// # Ok::<(), vaeljs_template_core::TemplateError>(())
/// ```
#[derive(Debug, Clone)]
pub struct CompiledTemplate {
    inner: neotales_gotmpl::Template,
    options: TemplateOptions,
}

impl CompiledTemplate {
    /// Compiles `source` using `options`.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::LimitExceeded`] if `source.len()` or control
    /// nesting exceeds the configured bounds, or [`TemplateError::Parse`] if
    /// `source` contains invalid template syntax.
    pub fn compile(source: &str, options: &TemplateOptions) -> Result<Self, TemplateError> {
        if source.len() > options.max_source_length {
            return Err(TemplateError::LimitExceeded(format!(
                "template source length {} exceeds limit {}",
                source.len(),
                options.max_source_length
            )));
        }

        let lowered = lower_standalone(source, options.max_nesting_depth)?;
        let effective = if options.html_escape {
            rewrite_html_escape(&lowered)?
        } else {
            lowered
        };

        let parse_opts = options.to_parse_options(effective.len().max(options.max_source_length));
        let functions = build_func_map(options);
        let inner = neotales_gotmpl::Template::parse_with_options(&effective, parse_opts)?
            .with_funcs(functions);

        Ok(Self {
            inner,
            options: options.clone(),
        })
    }

    /// Renders this compiled template against a JSON object `data`.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::InvalidData`] if `data` is not a JSON object,
    /// [`TemplateError::LimitExceeded`] if runtime bounds are exceeded,
    /// [`TemplateError::TemplateNotFound`] if a `{{template}}` target is missing,
    /// or [`TemplateError::Render`] if evaluation or a helper fails.
    pub fn render(&self, data: &serde_json::Value) -> Result<String, TemplateError> {
        let typed_data = json_object_to_map(data)?;
        let rendered = self
            .inner
            .render_typed_with_options(&typed_data, self.options.to_render_options())?;
        Ok(rendered)
    }

    /// Returns the options this template was compiled with.
    #[must_use]
    pub fn options(&self) -> &TemplateOptions {
        &self.options
    }
}

/// Compiles and renders `source` once against JSON object `data` using `options`.
///
/// For rendering the same template multiple times, prefer [`CompiledTemplate::compile`].
///
/// # Errors
///
/// Returns [`TemplateError`] if parsing, data conversion, or rendering fails.
///
/// ```
/// use vaeljs_template_core::{TemplateOptions, render};
/// use serde_json::json;
///
/// let options = TemplateOptions {
///     html_escape: true,
///     ..TemplateOptions::default()
/// };
/// let rendered = render(
///     "<p>{{ .msg }}</p>{{ .trusted | safeHtml }}",
///     &json!({ "msg": "<b>hi</b>", "trusted": "<em>ok</em>" }),
///     &options,
/// )?;
/// assert_eq!(rendered, "<p>&lt;b&gt;hi&lt;/b&gt;</p><em>ok</em>");
/// # Ok::<(), vaeljs_template_core::TemplateError>(())
/// ```
pub fn render(
    source: &str,
    data: &serde_json::Value,
    options: &TemplateOptions,
) -> Result<String, TemplateError> {
    CompiledTemplate::compile(source, options)?.render(data)
}
