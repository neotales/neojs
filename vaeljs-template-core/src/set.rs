//! Multi-template composition (`TemplateSet`) supporting layouts and partials.

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::syntax::{
    DecomposedTemplate, TemplatePart, decompose_template, format_go_string, rewrite_html_escape,
};
use crate::value::{build_func_map, json_object_to_map};
use crate::{TemplateError, TemplateOptions};

/// A named collection of templates supporting Go `{{define}}`, `{{template}}`,
/// and `{{block}}` layouts and partials.
///
/// Templates may be added in any order. Each template is validated for syntax
/// and nesting bounds on [`TemplateSet::add`], and composed deterministically
/// when [`TemplateSet::render`] is called for a named entry template.
///
/// ```
/// use vaeljs_template_core::{TemplateOptions, TemplateSet};
/// use serde_json::json;
///
/// let mut set = TemplateSet::new(TemplateOptions::default());
/// set.add(
///     "layout",
///     "<html><title>{{block \"title\" .}}Site{{end}}</title><body>{{template \"content\" .}}</body></html>",
/// )?;
/// set.add(
///     "home",
///     "{{define \"title\"}}Home{{end}}{{define \"content\"}}<h1>Hi {{.name}}</h1>{{end}}{{template \"layout\" .}}",
/// )?;
///
/// let html = set.render("home", &json!({"name": "Vael"}))?;
/// assert_eq!(
///     html,
///     "<html><title>Home</title><body><h1>Hi Vael</h1></body></html>"
/// );
/// # Ok::<(), vaeljs_template_core::TemplateError>(())
/// ```
#[derive(Debug, Clone)]
pub struct TemplateSet {
    options: TemplateOptions,
    entries: BTreeMap<String, DecomposedTemplate>,
}

impl Default for TemplateSet {
    fn default() -> Self {
        Self::new(TemplateOptions::default())
    }
}

impl TemplateSet {
    /// Creates an empty template set configured with `options`.
    #[must_use]
    pub fn new(options: TemplateOptions) -> Self {
        Self {
            options,
            entries: BTreeMap::new(),
        }
    }

    /// Returns the options used by this template set.
    #[must_use]
    pub fn options(&self) -> &TemplateOptions {
        &self.options
    }

    /// Validates and registers a named template in the set, replacing any
    /// previous entry with the same `name`.
    ///
    /// Supports Go `{{define "name"}}...{{end}}`, `{{template "name" .}}`, and
    /// `{{block "name" .}}...{{end}}` actions.
    ///
    /// # Errors
    ///
    /// Returns:
    /// - [`TemplateError::InvalidName`] if `name` is empty, whitespace-only, or
    ///   contains ASCII control characters.
    /// - [`TemplateError::LimitExceeded`] if `source.len()` exceeds
    ///   [`TemplateOptions::max_source_length`] or nesting exceeds
    ///   [`TemplateOptions::max_nesting_depth`].
    /// - [`TemplateError::Parse`] if `source` has invalid syntax or duplicate
    ///   definitions within the same template.
    pub fn add(&mut self, name: &str, source: &str) -> Result<&mut Self, TemplateError> {
        validate_template_name(name)?;
        if source.len() > self.options.max_source_length {
            return Err(TemplateError::LimitExceeded(format!(
                "template source length {} exceeds limit {}",
                source.len(),
                self.options.max_source_length
            )));
        }

        // Validate full syntax, nesting depth, and self-recursion immediately.
        let standalone = crate::syntax::lower_standalone(source, self.options.max_nesting_depth)?;
        let parse_opts = self
            .options
            .to_parse_options(standalone.len().max(self.options.max_source_length));
        neotales_gotmpl::Template::parse_with_options(&standalone, parse_opts)?;

        let decomposed = decompose_template(source, true, self.options.max_nesting_depth)?;
        self.entries.insert(name.to_owned(), decomposed);
        Ok(self)
    }

    /// Returns the registered template names in sorted order.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.entries.keys().cloned().collect()
    }

    /// Renders the registered template named `name` against `data`.
    ///
    /// Named partials and layouts registered in the set (both top-level entries
    /// and `{{define}}` blocks) are composed deterministically. Explicit
    /// `{{define}}` blocks in the entry template override `{{block}}` defaults
    /// in referenced layouts without colliding with sibling pages.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError::TemplateNotFound`] if `name` (or a referenced
    /// partial) is missing, [`TemplateError::InvalidData`] if `data` is not a
    /// JSON object, or [`TemplateError::Parse`]/[`TemplateError::Render`]/
    /// [`TemplateError::LimitExceeded`] on compilation or runtime failures.
    pub fn render(&self, name: &str, data: &serde_json::Value) -> Result<String, TemplateError> {
        let entry = self
            .entries
            .get(name)
            .ok_or_else(|| TemplateError::TemplateNotFound(name.to_owned()))?;
        let typed_data = json_object_to_map(data)?;

        let root_body = entry
            .defines
            .get(name)
            .map_or(entry.main_body.as_str(), |part| part.body.as_str());
        let resolved_defs = self.resolve_definitions(name, entry, root_body)?;
        let combined = self.compose_combined_source(root_body, &resolved_defs)?;

        let parse_opts = if self.options.html_escape {
            self.options
                .to_parse_options(combined.len().max(self.options.max_source_length))
        } else {
            self.options
                .to_parse_options(self.options.max_source_length)
        };
        let functions = build_func_map(&self.options);
        let compiled = neotales_gotmpl::Template::parse_with_options(&combined, parse_opts)?
            .with_funcs(functions);

        let rendered =
            compiled.render_typed_with_options(&typed_data, self.options.to_render_options())?;
        Ok(rendered)
    }

    fn resolve_definitions(
        &self,
        entry_name: &str,
        entry: &DecomposedTemplate,
        root_body: &str,
    ) -> Result<BTreeMap<String, String>, TemplateError> {
        let mut defs: BTreeMap<String, String> = BTreeMap::new();
        let mut queue: VecDeque<String> = VecDeque::new();
        let mut visited_entries: BTreeSet<String> = BTreeSet::new();
        visited_entries.insert(entry_name.to_owned());

        // 1. Entry template's own explicit `{{define}}` blocks take precedence.
        // Skip emitting a duplicate self-definition for `entry_name` itself
        // since `root_body` is already rendered as the root template (it will
        // still be materialized below if actually invoked by `{{template}}`).
        for (def_name, part) in &entry.defines {
            if def_name == entry_name {
                enqueue_calls(&part.calls, &mut queue);
            } else {
                defs.insert(def_name.clone(), part.body.clone());
                enqueue_calls(&part.calls, &mut queue);
            }
        }
        if !entry.defines.contains_key(entry_name) {
            enqueue_calls(&entry.main_calls, &mut queue);
        }

        // Track block defaults gathered from the entry and reached templates so
        // they fill in gaps after explicit defines on reached templates, without
        // letting unrelated sibling pages override a layout's block default.
        let mut available_block_defaults: BTreeMap<String, TemplatePart> =
            entry.block_defaults.clone();

        while let Some(callee) = queue.pop_front() {
            if defs.contains_key(&callee) {
                continue;
            }

            if callee == entry_name {
                defs.insert(callee, root_body.to_owned());
                continue;
            }

            if let Some(target_entry) = self.entries.get(&callee) {
                visited_entries.insert(callee.clone());

                // A template's explicit `define` matching its own name wins over
                // its main body; otherwise its main body is the named template.
                let body_part = target_entry.defines.get(&callee).map_or_else(
                    || {
                        (
                            target_entry.main_body.clone(),
                            target_entry.main_calls.clone(),
                        )
                    },
                    |p| (p.body.clone(), p.calls.clone()),
                );
                defs.insert(callee.clone(), body_part.0);
                enqueue_calls(&body_part.1, &mut queue);

                // Also expose any auxiliary `{{define}}` blocks declared inside
                // the referenced template (unless overridden by `entry`).
                for (sub_name, sub_part) in &target_entry.defines {
                    if !defs.contains_key(sub_name) {
                        defs.insert(sub_name.clone(), sub_part.body.clone());
                        enqueue_calls(&sub_part.calls, &mut queue);
                    }
                }
                for (blk_name, blk_part) in &target_entry.block_defaults {
                    available_block_defaults
                        .entry(blk_name.clone())
                        .or_insert_with(|| blk_part.clone());
                }
            } else if let Some(blk_part) = available_block_defaults.get(&callee).cloned() {
                defs.insert(callee.clone(), blk_part.body);
                enqueue_calls(&blk_part.calls, &mut queue);
            } else if let Some((bundle_name, bundle, part)) =
                self.find_define_in_other_entries(&callee, &visited_entries)?
            {
                visited_entries.insert(bundle_name.to_owned());
                defs.insert(callee.clone(), part.body.clone());
                enqueue_calls(&part.calls, &mut queue);

                for (sub_name, sub_part) in &bundle.defines {
                    if sub_name != entry_name && !defs.contains_key(sub_name) {
                        defs.insert(sub_name.clone(), sub_part.body.clone());
                        enqueue_calls(&sub_part.calls, &mut queue);
                    }
                }
                for (blk_name, blk_part) in &bundle.block_defaults {
                    available_block_defaults
                        .entry(blk_name.clone())
                        .or_insert_with(|| blk_part.clone());
                }
            }
        }

        // Materialize any remaining block defaults from reached templates (e.g.
        // if a layout's block default calls another block).
        for (blk_name, blk_part) in available_block_defaults {
            defs.entry(blk_name).or_insert(blk_part.body);
        }

        Ok(defs)
    }

    fn find_define_in_other_entries<'a>(
        &'a self,
        def_name: &str,
        visited_entries: &BTreeSet<String>,
    ) -> Result<Option<(&'a str, &'a DecomposedTemplate, &'a TemplatePart)>, TemplateError> {
        let mut found: Option<(&'a str, &'a DecomposedTemplate, &'a TemplatePart)> = None;
        for (entry_name, decomposed) in &self.entries {
            if visited_entries.contains(entry_name) {
                continue;
            }
            if let Some(part) = decomposed.defines.get(def_name) {
                if found.is_some() {
                    return Err(TemplateError::Parse(format!(
                        "ambiguous template definition: {def_name}"
                    )));
                }
                found = Some((entry_name.as_str(), decomposed, part));
            }
        }
        Ok(found)
    }

    fn compose_combined_source(
        &self,
        main_body: &str,
        defs: &BTreeMap<String, String>,
    ) -> Result<String, TemplateError> {
        let mut pre_escape_len = main_body.len();
        for (def_name, def_body) in defs {
            pre_escape_len = pre_escape_len
                .saturating_add("{{define ".len())
                .saturating_add(format_go_string(def_name).len())
                .saturating_add("}}".len())
                .saturating_add(def_body.len())
                .saturating_add("{{end}}".len());
        }
        if pre_escape_len > self.options.max_source_length {
            return Err(TemplateError::LimitExceeded(format!(
                "composed template source length {pre_escape_len} exceeds limit {}",
                self.options.max_source_length
            )));
        }

        let mut combined = String::with_capacity(pre_escape_len);
        for (def_name, def_body) in defs {
            let processed_body = if self.options.html_escape {
                rewrite_html_escape(def_body)?
            } else {
                def_body.clone()
            };
            combined.push_str("{{define ");
            combined.push_str(&format_go_string(def_name));
            combined.push_str("}}");
            combined.push_str(&processed_body);
            combined.push_str("{{end}}");
        }

        if self.options.html_escape {
            combined.push_str(&rewrite_html_escape(main_body)?);
        } else {
            combined.push_str(main_body);
        }
        Ok(combined)
    }
}

fn enqueue_calls(calls: &BTreeSet<String>, queue: &mut VecDeque<String>) {
    for call in calls {
        queue.push_back(call.clone());
    }
}

fn validate_template_name(name: &str) -> Result<(), TemplateError> {
    if name.trim().is_empty() {
        return Err(TemplateError::InvalidName(
            "template name must not be empty".to_owned(),
        ));
    }
    if name.chars().any(char::is_control) {
        return Err(TemplateError::InvalidName(
            "template name must not contain control characters".to_owned(),
        ));
    }
    Ok(())
}
