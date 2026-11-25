//! Go template action scanning, `block` lowering, `define` extraction, and
//! HTML auto-escape rewriting.

use std::collections::{BTreeMap, BTreeSet};

use crate::TemplateError;
use crate::html::AUTO_ESCAPE_FN;

/// Result of decomposing a template source for [`crate::TemplateSet`] or
/// standalone compilation.
#[derive(Debug, Clone)]
pub(crate) struct DecomposedTemplate {
    /// Main template body with top-level `{{define}}` blocks removed and
    /// `{{block}}` actions lowered to `{{template}}` calls.
    pub main_body: String,
    /// Explicit `{{define "name"}}...{{end}}` definitions found in the source.
    pub defines: BTreeMap<String, TemplatePart>,
    /// Default bodies from `{{block "name" expr}}...{{end}}` actions.
    pub block_defaults: BTreeMap<String, TemplatePart>,
    /// Template names invoked via `{{template "name" ...}}` or `{{block "name" ...}}`
    /// in the main body.
    pub main_calls: BTreeSet<String>,
}

/// A named template body (from `define` or `block`) along with its own calls.
#[derive(Debug, Clone)]
pub(crate) struct TemplatePart {
    pub body: String,
    pub calls: BTreeSet<String>,
}

#[derive(Debug, Clone, Copy)]
enum Token<'a> {
    Text(&'a str),
    Comment(&'a str),
    Action(Action<'a>),
}

#[derive(Debug, Clone, Copy)]
struct Action<'a> {
    start: usize,
    end: usize,
    trim_left: bool,
    trim_right: bool,
    body: &'a str,
}

fn is_template_space(ch: char) -> bool {
    matches!(ch, ' ' | '\t' | '\r' | '\n')
}

fn trim_template_space(s: &str) -> &str {
    s.trim_matches(is_template_space)
}

fn has_left_trim_marker(input: &str) -> bool {
    let bytes = input.as_bytes();
    bytes.len() >= 2 && bytes[0] == b'-' && is_template_space(char::from(bytes[1]))
}

fn has_right_trim_marker(input: &str) -> bool {
    let bytes = input.as_bytes();
    bytes.len() >= 2 && is_template_space(char::from(bytes[0])) && bytes[1] == b'-'
}

fn has_right_trim_suffix(input: &str) -> bool {
    let bytes = input.as_bytes();
    bytes.len() >= 2
        && is_template_space(char::from(bytes[bytes.len() - 2]))
        && bytes[bytes.len() - 1] == b'-'
}

fn find_comment_end(source: &str, comment_start: usize) -> Result<usize, TemplateError> {
    let body_start = comment_start + 2;
    let Some(rel_end) = source[body_start..].find("*/") else {
        return Err(TemplateError::Parse(
            "unterminated template comment".to_owned(),
        ));
    };
    let after_comment = body_start + rel_end + 2;
    let rest = &source[after_comment..];
    let trim_right = rest.len() >= 2 && has_right_trim_marker(rest) && rest[2..].starts_with("}}");
    if !trim_right && !rest.starts_with("}}") {
        return Err(TemplateError::Parse(
            "unterminated template comment".to_owned(),
        ));
    }
    let delimiter_start = after_comment + if trim_right { 2 } else { 0 };
    let mut end = delimiter_start + 2;
    if trim_right {
        let following = &source[end..];
        let trimmed = following.trim_start_matches(is_template_space);
        end += following.len() - trimmed.len();
    }
    Ok(end)
}

fn find_action_end(input: &str) -> Result<usize, TemplateError> {
    let mut quote: Option<char> = None;
    let mut escaped = false;
    for (idx, ch) in input.char_indices() {
        if let Some(active) = quote {
            if active == '`' {
                if ch == '`' {
                    quote = None;
                }
            } else {
                if ch == '\n' {
                    return Err(TemplateError::Parse(
                        "invalid quoted template literal".to_owned(),
                    ));
                }
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == active {
                    quote = None;
                }
            }
            continue;
        }
        match ch {
            '"' | '`' | '\'' => quote = Some(ch),
            '}' if input[idx..].starts_with("}}") => return Ok(idx),
            _ => {}
        }
    }
    Err(TemplateError::Parse(
        "unterminated template action".to_owned(),
    ))
}

fn scan_tokens(source: &str) -> Result<Vec<Token<'_>>, TemplateError> {
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < source.len() {
        let Some(rel_open) = source[pos..].find("{{") else {
            tokens.push(Token::Text(&source[pos..]));
            break;
        };
        let open_start = pos + rel_open;
        if open_start > pos {
            tokens.push(Token::Text(&source[pos..open_start]));
        }

        let after_open = open_start + 2;
        let trim_left = has_left_trim_marker(&source[after_open..]);
        let action_start = after_open + if trim_left { 2 } else { 0 };
        if !trim_left && source[action_start..].starts_with('-') {
            find_action_end(&source[action_start..])?;
            return Err(TemplateError::Parse(
                "malformed template trim marker".to_owned(),
            ));
        }

        if source[action_start..].starts_with("/*") {
            let comment_end = find_comment_end(source, action_start)?;
            tokens.push(Token::Comment(&source[open_start..comment_end]));
            pos = comment_end;
            continue;
        }

        let end_rel = find_action_end(&source[action_start..])?;
        let raw_action = &source[action_start..action_start + end_rel];
        let trim_right = has_right_trim_suffix(raw_action);
        if raw_action.ends_with('-') && !trim_right {
            return Err(TemplateError::Parse(
                "malformed template trim marker".to_owned(),
            ));
        }
        let body_end = raw_action.len() - if trim_right { 2 } else { 0 };
        let body = trim_template_space(&raw_action[..body_end]);
        let close_end = action_start + end_rel + 2;

        tokens.push(Token::Action(Action {
            start: open_start,
            end: close_end,
            trim_left,
            trim_right,
            body,
        }));
        pos = close_end;
    }
    Ok(tokens)
}

fn control_expression<'a>(action: &'a str, keyword: &str) -> Option<&'a str> {
    let rest = action.strip_prefix(keyword)?;
    let first = rest.chars().next()?;
    is_template_space(first).then(|| rest.trim_start_matches(is_template_space))
}

fn read_radix_escape(
    chars: &mut std::str::CharIndices<'_>,
    count: usize,
    radix: u32,
) -> Option<u32> {
    let mut value = 0_u32;
    for _ in 0..count {
        let (_, ch) = chars.next()?;
        let digit = ch.to_digit(radix).filter(|_| ch.is_ascii_hexdigit())?;
        value = value * radix + digit;
    }
    Some(value)
}

fn append_unicode_escape(
    out: &mut Vec<u8>,
    chars: &mut std::str::CharIndices<'_>,
    count: usize,
) -> Option<()> {
    let value = read_radix_escape(chars, count, 16)?;
    let ch = char::from_u32(value)?;
    let mut encoded = [0_u8; 4];
    out.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
    Some(())
}

fn decode_quoted_bytes(after_quote: &str) -> Option<(String, &str)> {
    let mut out = Vec::new();
    let mut chars = after_quote.char_indices();
    while let Some((idx, ch)) = chars.next() {
        if ch == '\n' {
            return None;
        }
        if ch == '"' {
            let decoded = String::from_utf8(out).ok()?;
            return Some((decoded, &after_quote[idx + 1..]));
        }
        if ch != '\\' {
            let mut encoded = [0_u8; 4];
            out.extend_from_slice(ch.encode_utf8(&mut encoded).as_bytes());
            continue;
        }
        match chars.next()?.1 {
            'a' => out.push(0x07),
            'b' => out.push(0x08),
            'f' => out.push(0x0c),
            'n' => out.push(b'\n'),
            'r' => out.push(b'\r'),
            't' => out.push(b'\t'),
            'v' => out.push(0x0b),
            '\\' => out.push(b'\\'),
            '"' => out.push(b'"'),
            'x' => out.push(u8::try_from(read_radix_escape(&mut chars, 2, 16)?).ok()?),
            'u' => append_unicode_escape(&mut out, &mut chars, 4)?,
            'U' => append_unicode_escape(&mut out, &mut chars, 8)?,
            first @ '0'..='7' => {
                let mut value = u32::from(first as u8 - b'0');
                for _ in 0..2 {
                    let (_, digit) = chars.next()?;
                    if !matches!(digit, '0'..='7') {
                        return None;
                    }
                    value = value * 8 + u32::from(digit as u8 - b'0');
                }
                out.push(u8::try_from(value).ok()?);
            }
            _ => return None,
        }
    }
    None
}

fn parse_first_string_literal(input: &str) -> Result<(String, &str), TemplateError> {
    let trimmed = input.trim_start_matches(is_template_space);
    if let Some(after_quote) = trimmed.strip_prefix('"') {
        decode_quoted_bytes(after_quote)
            .ok_or_else(|| TemplateError::Parse("invalid quoted template literal".to_owned()))
    } else if let Some(after_tick) = trimmed.strip_prefix('`') {
        let Some(end_idx) = after_tick.find('`') else {
            return Err(TemplateError::Parse(
                "invalid quoted template literal".to_owned(),
            ));
        };
        Ok((after_tick[..end_idx].to_owned(), &after_tick[end_idx + 1..]))
    } else {
        Err(TemplateError::Parse(format!(
            "expected quoted template name in `{trimmed}`"
        )))
    }
}

/// Formats a template name as a Go double-quoted string literal inside `{{define "..."}}`.
pub(crate) fn format_go_string(name: &str) -> String {
    let mut out = String::with_capacity(name.len() + 2);
    out.push('"');
    for ch in name.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

fn format_template_call(name: &str, expr: &str, trim_left: bool, trim_right: bool) -> String {
    let left = if trim_left { "{{- " } else { "{{ " };
    let right = if trim_right { " -}}" } else { " }}" };
    format!("{left}template {} {expr}{right}", format_go_string(name))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockOutcome {
    End { trim_left: bool, trim_right: bool },
    Eof,
}

struct ParserState {
    defines: BTreeMap<String, TemplatePart>,
    block_defaults: BTreeMap<String, TemplatePart>,
    extract_defines: bool,
    max_nesting_depth: usize,
}

fn check_nesting(depth: usize, max_nesting_depth: usize) -> Result<(), TemplateError> {
    if depth > max_nesting_depth {
        return Err(TemplateError::LimitExceeded(
            "template resource limit exceeded".to_owned(),
        ));
    }
    Ok(())
}

fn trim_body_edges(body: &mut String, trim_start: bool, trim_end: bool) {
    if trim_start {
        let trimmed = body.trim_start_matches(is_template_space).to_owned();
        *body = trimmed;
    }
    if trim_end {
        let trimmed_len = body.trim_end_matches(is_template_space).len();
        body.truncate(trimmed_len);
    }
}

fn handle_block_action<'a>(
    source: &'a str,
    tokens: &[Token<'a>],
    pos: &mut usize,
    action: Action<'a>,
    rest: &str,
    depth: usize,
    state: &mut ParserState,
) -> Result<(String, String), TemplateError> {
    check_nesting(depth + 1, state.max_nesting_depth)?;
    let (name, expr_raw) = parse_first_string_literal(rest)?;
    let expr = trim_template_space(expr_raw);
    if expr.is_empty() {
        return Err(TemplateError::Parse(
            "block action requires a pipeline expression".to_owned(),
        ));
    }

    let (mut body, body_calls, outcome) =
        parse_segment(source, tokens, pos, true, true, depth + 1, state)?;
    let BlockOutcome::End {
        trim_left: end_trim_left,
        trim_right: end_trim_right,
    } = outcome
    else {
        return Err(TemplateError::Parse(
            "unterminated block action: missing `end`".to_owned(),
        ));
    };

    trim_body_edges(&mut body, action.trim_right, end_trim_left);
    if state.block_defaults.contains_key(&name) {
        return Err(TemplateError::Parse(format!(
            "duplicate template definition: {name}"
        )));
    }
    state.block_defaults.insert(
        name.clone(),
        TemplatePart {
            body,
            calls: body_calls,
        },
    );
    let call_site = format_template_call(&name, expr, action.trim_left, end_trim_right);
    Ok((name, call_site))
}

#[allow(clippy::too_many_arguments)]
fn handle_define_action<'a>(
    source: &'a str,
    tokens: &[Token<'a>],
    pos: &mut usize,
    action: Action<'a>,
    rest: &str,
    depth: usize,
    state: &mut ParserState,
    output: &mut String,
) -> Result<bool, TemplateError> {
    check_nesting(depth + 1, state.max_nesting_depth)?;
    let (name, trailing) = parse_first_string_literal(rest)?;
    if !trim_template_space(trailing).is_empty() {
        return Err(TemplateError::Parse(format!(
            "unsupported template action: {}",
            action.body
        )));
    }

    let (mut body, body_calls, outcome) =
        parse_segment(source, tokens, pos, true, true, depth + 1, state)?;
    let BlockOutcome::End {
        trim_left: end_trim_left,
        trim_right: end_trim_right,
    } = outcome
    else {
        return Err(TemplateError::Parse(
            "unterminated define action: missing `end`".to_owned(),
        ));
    };

    if state.defines.contains_key(&name) {
        return Err(TemplateError::Parse(format!(
            "duplicate template definition: {name}"
        )));
    }

    if state.extract_defines {
        if action.trim_left {
            let trimmed_len = output.trim_end_matches(is_template_space).len();
            output.truncate(trimmed_len);
        }
        trim_body_edges(&mut body, action.trim_right, end_trim_left);
        state.defines.insert(
            name,
            TemplatePart {
                body,
                calls: body_calls,
            },
        );
        return Ok(end_trim_right);
    }

    state.defines.insert(
        name.clone(),
        TemplatePart {
            body: body.clone(),
            calls: body_calls,
        },
    );
    let open = if action.trim_left { "{{- " } else { "{{ " };
    let open_end = if action.trim_right { " -}}" } else { " }}" };
    let close = if end_trim_left { "{{- " } else { "{{ " };
    let close_end = if end_trim_right { " -}}" } else { " }}" };
    output.push_str(open);
    output.push_str("define ");
    output.push_str(&format_go_string(&name));
    output.push_str(open_end);
    output.push_str(&body);
    output.push_str(close);
    output.push_str("end");
    output.push_str(close_end);
    Ok(false)
}

fn format_action_slice(source: &str, action: Action<'_>, trim_left: bool) -> String {
    if trim_left == action.trim_left {
        return source[action.start..action.end].to_owned();
    }
    let left = if trim_left { "{{- " } else { "{{ " };
    let right = if action.trim_right { " -}}" } else { " }}" };
    format!("{left}{}{right}", action.body)
}

#[allow(clippy::too_many_arguments)]
fn handle_control_block<'a>(
    source: &'a str,
    tokens: &[Token<'a>],
    pos: &mut usize,
    action: Action<'a>,
    effective_trim_left: bool,
    depth: usize,
    state: &mut ParserState,
    calls: &mut BTreeSet<String>,
    output: &mut String,
) -> Result<(), TemplateError> {
    check_nesting(depth + 1, state.max_nesting_depth)?;
    output.push_str(&format_action_slice(source, action, effective_trim_left));
    let (inner, inner_calls, outcome) =
        parse_segment(source, tokens, pos, true, false, depth + 1, state)?;
    calls.extend(inner_calls);
    output.push_str(&inner);
    if let BlockOutcome::End {
        trim_left,
        trim_right,
    } = outcome
    {
        let left = if trim_left { "{{- " } else { "{{ " };
        let right = if trim_right { " -}}" } else { " }}" };
        output.push_str(left);
        output.push_str("end");
        output.push_str(right);
    }
    Ok(())
}

fn is_control_block(body: &str) -> bool {
    control_expression(body, "if").is_some()
        || control_expression(body, "range").is_some()
        || control_expression(body, "with").is_some()
}

#[allow(clippy::too_many_arguments)]
fn handle_regular_action<'a>(
    source: &'a str,
    tokens: &[Token<'a>],
    pos: &mut usize,
    action: Action<'a>,
    effective_trim_left: bool,
    depth: usize,
    state: &mut ParserState,
    calls: &mut BTreeSet<String>,
    output: &mut String,
) -> Result<(bool, Option<usize>), TemplateError> {
    if let Some(rest) = control_expression(action.body, "block") {
        let mut adjusted = action;
        adjusted.trim_left = effective_trim_left;
        let (name, call_site) =
            handle_block_action(source, tokens, pos, adjusted, rest, depth, state)?;
        calls.insert(name);
        output.push_str(&call_site);
        return Ok((false, None));
    }
    if let Some(rest) = control_expression(action.body, "define") {
        let mut adjusted = action;
        adjusted.trim_left = effective_trim_left;
        let trim_next =
            handle_define_action(source, tokens, pos, adjusted, rest, depth, state, output)?;
        let boundary = state.extract_defines.then_some(output.len());
        return Ok((trim_next, boundary));
    }
    if is_control_block(action.body) {
        handle_control_block(
            source,
            tokens,
            pos,
            action,
            effective_trim_left,
            depth,
            state,
            calls,
            output,
        )?;
        return Ok((false, None));
    }
    if let Some(rest) = control_expression(action.body, "template")
        && let Ok((name, _)) = parse_first_string_literal(rest)
    {
        calls.insert(name);
    }
    output.push_str(&format_action_slice(source, action, effective_trim_left));
    Ok((false, None))
}

fn parse_segment<'a>(
    source: &'a str,
    tokens: &[Token<'a>],
    pos: &mut usize,
    stop_on_end: bool,
    disallow_else: bool,
    depth: usize,
    state: &mut ParserState,
) -> Result<(String, BTreeSet<String>, BlockOutcome), TemplateError> {
    let mut output = String::new();
    let mut calls = BTreeSet::new();
    let mut trim_next_leading_space = false;
    // Byte offset in `output` immediately before an extracted `{{define}}` block.
    // A subsequent `{{- ...}}` action must not trim whitespace before this boundary.
    let mut protected_len: Option<usize> = None;

    while *pos < tokens.len() {
        let token = tokens[*pos];
        *pos += 1;

        match token {
            Token::Text(text) => {
                let slice = if trim_next_leading_space {
                    trim_next_leading_space = false;
                    text.trim_start_matches(is_template_space)
                } else {
                    text
                };
                output.push_str(slice);
                if !slice.chars().all(is_template_space) {
                    protected_len = None;
                }
            }
            Token::Comment(raw) => {
                trim_next_leading_space = false;
                output.push_str(raw);
            }
            Token::Action(action) => {
                trim_next_leading_space = false;
                let mut effective_trim_left = action.trim_left;

                if effective_trim_left && let Some(bound) = protected_len {
                    output.truncate(bound);
                    effective_trim_left = false;
                }
                protected_len = None;

                if action.body == "end" {
                    if stop_on_end {
                        return Ok((
                            output,
                            calls,
                            BlockOutcome::End {
                                trim_left: effective_trim_left,
                                trim_right: action.trim_right,
                            },
                        ));
                    }
                    output.push_str(&format_action_slice(source, action, effective_trim_left));
                    continue;
                }

                if action.body == "else" {
                    if disallow_else {
                        return Err(TemplateError::Parse(
                            "unexpected `else` in block or define action".to_owned(),
                        ));
                    }
                    output.push_str(&format_action_slice(source, action, effective_trim_left));
                    continue;
                }

                (trim_next_leading_space, protected_len) = handle_regular_action(
                    source,
                    tokens,
                    pos,
                    action,
                    effective_trim_left,
                    depth,
                    state,
                    &mut calls,
                    &mut output,
                )?;
            }
        }
    }

    Ok((output, calls, BlockOutcome::Eof))
}

/// Decomposes a template source into its main body, explicit `define` blocks,
/// `block` defaults, and `template` calls.
pub(crate) fn decompose_template(
    source: &str,
    extract_defines: bool,
    max_nesting_depth: usize,
) -> Result<DecomposedTemplate, TemplateError> {
    let tokens = scan_tokens(source)?;
    let mut pos = 0;
    let mut state = ParserState {
        defines: BTreeMap::new(),
        block_defaults: BTreeMap::new(),
        extract_defines,
        max_nesting_depth,
    };
    let (main_body, main_calls, _) =
        parse_segment(source, &tokens, &mut pos, false, false, 0, &mut state)?;
    Ok(DecomposedTemplate {
        main_body,
        defines: state.defines,
        block_defaults: state.block_defaults,
        main_calls,
    })
}

/// Lowers `{{block "name" expr}}...{{end}}` in a standalone template into
/// `{{template "name" expr}}` plus non-overridden `{{define "name"}}...{{end}}`.
pub(crate) fn lower_standalone(
    source: &str,
    max_nesting_depth: usize,
) -> Result<String, TemplateError> {
    let decomposed = decompose_template(source, false, max_nesting_depth)?;
    let mut combined = String::new();
    for (name, part) in decomposed.block_defaults {
        if !decomposed.defines.contains_key(&name) {
            combined.push_str("{{define ");
            combined.push_str(&format_go_string(&name));
            combined.push_str("}}");
            combined.push_str(&part.body);
            combined.push_str("{{end}}");
        }
    }
    combined.push_str(&decomposed.main_body);
    Ok(combined)
}

/// Rewrites output expression actions (`{{ expr }}`) to pipe through the
/// internal HTML auto-escaping helper (`{{ expr | __neo_auto_escape }}`).
///
/// Control structures (`if`, `else`, `end`, `range`, `with`, `define`,
/// `template`, `block`), comments, and variable assignments (`$x := ...`) are
/// preserved untouched so comparisons, loops, and assignments operate on raw
/// typed values.
pub(crate) fn rewrite_html_escape(source: &str) -> Result<String, TemplateError> {
    let tokens = scan_tokens(source)?;
    let mut out = String::with_capacity(source.len());

    for token in tokens {
        match token {
            Token::Text(text) => out.push_str(text),
            Token::Comment(comment) => out.push_str(comment),
            Token::Action(action) => {
                if should_wrap_auto_escape(action.body) {
                    let left = if action.trim_left { "{{- " } else { "{{ " };
                    let right = if action.trim_right { " -}}" } else { " }}" };
                    out.push_str(left);
                    out.push_str(action.body);
                    out.push_str(" | ");
                    out.push_str(AUTO_ESCAPE_FN);
                    out.push_str(right);
                } else {
                    out.push_str(&source[action.start..action.end]);
                }
            }
        }
    }

    Ok(out)
}

fn should_wrap_auto_escape(body: &str) -> bool {
    if body.is_empty()
        || matches!(
            body,
            "else" | "end" | "if" | "range" | "with" | "define" | "template" | "block"
        )
        || body.starts_with("/*")
    {
        return false;
    }

    for kw in ["if", "range", "with", "define", "template", "block"] {
        if control_expression(body, kw).is_some() {
            return false;
        }
    }

    !(body.starts_with('$') && body.contains(":="))
}

#[cfg(test)]
mod tests {
    use super::{decompose_template, lower_standalone, rewrite_html_escape};

    #[test]
    fn lowers_block_with_trim_markers_and_nested_block() {
        let src = "A{{- block \"outer\" . -}}\n  hello {{- block \"inner\" . -}}!{{- end -}}  \n{{- end -}}B";
        let decomposed = decompose_template(src, true, 100).unwrap();
        assert_eq!(decomposed.main_body, "A{{- template \"outer\" . -}}B");
        assert_eq!(
            decomposed.block_defaults["outer"].body,
            "hello {{- template \"inner\" . -}}"
        );
        assert_eq!(decomposed.block_defaults["inner"].body, "!");
    }

    #[test]
    fn handles_closing_braces_inside_quoted_and_raw_strings() {
        let src = r#"{{ "a }} b" }}|{{ `c }} d` }}"#;
        let rewritten = rewrite_html_escape(src).unwrap();
        assert_eq!(
            rewritten,
            r#"{{ "a }} b" | __neo_auto_escape }}|{{ `c }} d` | __neo_auto_escape }}"#
        );
    }

    #[test]
    fn standalone_define_overrides_block_default() {
        let src = "{{define \"x\"}}override{{end}}{{block \"x\" .}}default{{end}}";
        let lowered = lower_standalone(src, 100).unwrap();
        assert!(!lowered.contains("default"));
        assert!(lowered.contains("override"));
    }

    #[test]
    fn preserves_boundary_when_extracting_define_before_left_trim() {
        let src = "A  {{define \"x\"}}X{{end}}  {{- .name}}";
        let decomposed = decompose_template(src, true, 100).unwrap();
        assert_eq!(decomposed.main_body, "A  {{ .name }}");
    }

    #[test]
    fn decodes_hex_unicode_and_octal_escapes_in_names() {
        let src = "{{define \"\\x41\\u0042\\U00000043\\104\"}}ok{{end}}";
        let decomposed = decompose_template(src, true, 100).unwrap();
        assert!(decomposed.defines.contains_key("ABCD"));
    }
}
