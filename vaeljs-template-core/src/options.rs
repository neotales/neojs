//! Configuration and safety bounds for template parsing and rendering.

/// Options controlling helper registration, HTML escaping, and resource bounds.
///
/// The defaults enable safe Sprig helpers (`sprig: true`), allow all safe
/// helpers (`hermetic: false`), keep plain-text rendering (`html_escape: false`),
/// and mirror [`neotales_gotmpl::ParseOptions`] / [`neotales_gotmpl::RenderOptions`]
/// finite bounds so untrusted templates cannot trigger unbounded memory or CPU use.
///
/// ```
/// use vaeljs_template_core::TemplateOptions;
///
/// let opts = TemplateOptions::default();
/// assert!(opts.sprig);
/// assert!(!opts.hermetic);
/// assert!(!opts.html_escape);
/// assert_eq!(opts.max_source_length, 1_048_576);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateOptions {
    /// Whether to load the safe [`neotales_sprig`] helper set (default: `true`).
    pub sprig: bool,
    /// When `true` and [`Self::sprig`] is `true`, restrict registered Sprig
    /// helpers to [`neotales_sprig::hermetic_names`], omitting clock- and
    /// environment-reading helpers such as `now` and `ago` (default: `false`).
    pub hermetic: bool,
    /// Enable automatic HTML escaping for rendered template expressions while
    /// preserving values marked with `safeHtml` (default: `false`).
    ///
    /// Note that when enabled, each rendered output expression invokes an
    /// internal auto-escape pipeline stage, which counts as one helper
    /// invocation against [`Self::max_function_calls`].
    pub html_escape: bool,
    /// Maximum accepted template source length in UTF-8 bytes (default: `1_048_576`).
    pub max_source_length: usize,
    /// Maximum control-structure and evaluation nesting depth (default: `100`).
    pub max_nesting_depth: usize,
    /// Maximum rendered output length in UTF-8 bytes (default: `4_194_304`).
    pub max_output_length: usize,
    /// Maximum `range` loop iterations in a single render (default: `100_000`).
    pub max_iterations: usize,
    /// Maximum helper function invocations in a single render (default: `100_000`).
    pub max_function_calls: usize,
}

impl Default for TemplateOptions {
    fn default() -> Self {
        let parse = neotales_gotmpl::ParseOptions::default();
        let render = neotales_gotmpl::RenderOptions::default();
        Self {
            sprig: true,
            hermetic: false,
            html_escape: false,
            max_source_length: parse.max_source_length,
            max_nesting_depth: parse.max_nesting_depth,
            max_output_length: render.max_output_length,
            max_iterations: render.max_iterations,
            max_function_calls: render.max_function_calls,
        }
    }
}

impl TemplateOptions {
    /// Creates options with safe defaults.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub(crate) fn to_parse_options(
        &self,
        effective_max_source_length: usize,
    ) -> neotales_gotmpl::ParseOptions {
        neotales_gotmpl::ParseOptions {
            max_source_length: effective_max_source_length,
            max_nesting_depth: self.max_nesting_depth,
        }
    }

    pub(crate) fn to_render_options(&self) -> neotales_gotmpl::RenderOptions<'static> {
        neotales_gotmpl::RenderOptions {
            max_output_length: self.max_output_length,
            max_iterations: self.max_iterations,
            max_function_calls: self.max_function_calls,
            max_nesting_depth: self.max_nesting_depth,
            ..neotales_gotmpl::RenderOptions::default()
        }
    }
}
