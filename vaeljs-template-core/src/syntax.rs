//! Go template action scanning, `block` lowering, `define` extraction, and
//! HTML auto-escape rewriting.

use std::collections::{BTreeMap, BTreeSet};

use crate::TemplateError;
use crate::html::{
    AUTO_ESCAPE_COMMENT_FN, AUTO_ESCAPE_CSS_FN, AUTO_ESCAPE_CSS_UNQUOTED_FN, AUTO_ESCAPE_FN,
    AUTO_ESCAPE_JS_ATTR_RAW_FN, AUTO_ESCAPE_JS_ATTR_RAW_UNQUOTED_FN, AUTO_ESCAPE_JS_ATTR_STR_FN,
    AUTO_ESCAPE_JS_ATTR_STR_UNQUOTED_FN, AUTO_ESCAPE_JS_ATTR_TPL_FN,
    AUTO_ESCAPE_JS_ATTR_TPL_UNQUOTED_FN, AUTO_ESCAPE_JS_RAW_FN, AUTO_ESCAPE_JS_STR_FN,
    AUTO_ESCAPE_JS_TPL_FN, AUTO_ESCAPE_UNQUOTED_FN, AUTO_ESCAPE_URL_CONT_NO_COLON_FN,
    AUTO_ESCAPE_URL_CONT_NO_COLON_UNQUOTED_FN, AUTO_ESCAPE_URL_START_FN,
    AUTO_ESCAPE_URL_START_UNQUOTED_FN,
};

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
/// context-appropriate internal HTML auto-escaping helper.
///
/// Control structures (`if`, `else`, `else if`, `else with`, `end`, `break`,
/// `continue`, `range`, `with`, `define`, `template`, `block`), comments, and
/// variable declarations/reassignments (`$x := ...`, `$k, $v := ...`,
/// `$x = ...`) are preserved untouched so comparisons, loops, and assignments
/// operate on raw typed values.
pub(crate) fn rewrite_html_escape(source: &str) -> Result<String, TemplateError> {
    let tokens = scan_tokens(source)?;
    let trimmed_texts = compute_trimmed_texts(&tokens);
    let block_map = build_block_map(&tokens);
    let mut out = String::with_capacity(source.len());
    let mut scanner = HtmlScanner::default();
    let mut block_stack: Vec<BlockOrigin> = Vec::new();

    let mut idx = 0;
    while idx < tokens.len() {
        match tokens[idx] {
            Token::Text(_) | Token::Comment(_) => {
                let run_start = idx;
                while idx < tokens.len()
                    && matches!(tokens[idx], Token::Text(_) | Token::Comment(_))
                {
                    match tokens[idx] {
                        Token::Text(text) => out.push_str(text),
                        Token::Comment(comment) => out.push_str(comment),
                        Token::Action(_) => unreachable!(),
                    }
                    idx += 1;
                }
                advance_static_run(
                    &mut scanner,
                    &tokens[run_start..idx],
                    &trimmed_texts[run_start..idx],
                )?;
            }
            Token::Action(action) => {
                if scanner.in_srcdoc() {
                    return Err(TemplateError::Parse(
                        "dynamic template actions are not allowed inside `srcdoc` attributes"
                            .to_owned(),
                    ));
                }
                if is_template_or_block_call(action.body)
                    && !scanner.allows_template_or_block_call()
                {
                    return Err(TemplateError::Parse(
                        "template/block invocation is only allowed in HTML text context".to_owned(),
                    ));
                }
                if control_expression(action.body, "define").is_some()
                    && !scanner.allows_template_or_block_call()
                {
                    return Err(TemplateError::Parse(
                        "`define` block is only allowed in HTML text context".to_owned(),
                    ));
                }
                scanner.validate_action_in_context(
                    action.body,
                    &tokens,
                    &trimmed_texts,
                    &block_map,
                    idx,
                )?;
                update_block_stack(&mut block_stack, &mut scanner, action.body)?;
                if should_wrap_auto_escape(action.body) {
                    let helper = scanner.escape_fn_for_action()?;
                    let left = if action.trim_left { "{{- " } else { "{{ " };
                    let right = if action.trim_right { " -}}" } else { " }}" };
                    out.push_str(left);
                    out.push_str(action.body);
                    out.push_str(" | ");
                    out.push_str(helper);
                    out.push_str(right);
                } else {
                    out.push_str(&source[action.start..action.end]);
                }
                idx += 1;
            }
        }
    }

    if !block_stack.is_empty() {
        return Err(TemplateError::Parse(
            "unterminated template control block".to_owned(),
        ));
    }
    if !matches!(scanner.state, HtmlState::Text) {
        return Err(TemplateError::Parse(
            "template must end in HTML text context".to_owned(),
        ));
    }

    Ok(out)
}

fn advance_static_run(
    scanner: &mut HtmlScanner,
    run_tokens: &[Token<'_>],
    run_trimmed: &[&str],
) -> Result<(), TemplateError> {
    if run_tokens.len() == 1 {
        if !run_trimmed[0].is_empty() {
            scanner.advance_text(run_trimmed[0])?;
        }
        return Ok(());
    }
    let total_len: usize = run_trimmed.iter().map(|s| s.len()).sum();
    if total_len == 0 {
        return Ok(());
    }
    let mut coalesced = String::with_capacity(total_len);
    for slice in run_trimmed {
        coalesced.push_str(slice);
    }
    scanner.advance_text(&coalesced)
}

fn comment_has_left_trim(raw: &str) -> bool {
    has_left_trim_marker(&raw[2..])
}

fn comment_has_right_trim(raw: &str) -> bool {
    let trimmed = raw.trim_end_matches(is_template_space);
    let Some(before_close) = trimmed.strip_suffix("}}") else {
        return false;
    };
    has_right_trim_suffix(before_close)
}

fn compute_trimmed_texts<'a>(tokens: &[Token<'a>]) -> Vec<&'a str> {
    tokens
        .iter()
        .enumerate()
        .map(|(idx, token)| {
            let Token::Text(mut text) = *token else {
                return "";
            };

            let mut left_trimmed = false;
            let mut j = idx;
            while j > 0 {
                j -= 1;
                match tokens[j] {
                    Token::Comment(raw) => {
                        if comment_has_right_trim(raw) {
                            left_trimmed = true;
                            break;
                        }
                    }
                    Token::Action(prev) => {
                        left_trimmed = prev.trim_right;
                        break;
                    }
                    Token::Text(_) => break,
                }
            }
            if left_trimmed {
                text = text.trim_start_matches(is_template_space);
            }

            let mut right_trimmed = false;
            let mut k = idx + 1;
            while k < tokens.len() {
                match tokens[k] {
                    Token::Comment(raw) => {
                        if comment_has_left_trim(raw) {
                            right_trimmed = true;
                            break;
                        }
                        k += 1;
                    }
                    Token::Action(next) => {
                        right_trimmed = next.trim_left;
                        break;
                    }
                    Token::Text(_) => break,
                }
            }
            if right_trimmed {
                text = text.trim_end_matches(is_template_space);
            }

            text
        })
        .collect()
}

struct BlockMap {
    arms: Vec<Vec<usize>>,
    end_idx: Vec<usize>,
    has_bare_else: Vec<bool>,
    owner_end: Vec<usize>,
    enclosing_loop: Vec<Option<(usize, usize)>>,
}

fn build_block_map(tokens: &[Token<'_>]) -> BlockMap {
    struct Pending {
        start: usize,
        is_range: bool,
        arms: Vec<usize>,
        has_bare_else: bool,
    }

    let n = tokens.len();
    let mut arms = vec![Vec::new(); n];
    let mut end_idx = vec![n; n];
    let mut has_bare_else = vec![false; n];
    let mut owner_end = vec![n; n];
    let mut enclosing_loop = vec![None; n];

    let mut stack: Vec<Pending> = Vec::new();
    let mut range_stack: Vec<(usize, usize)> = Vec::new();

    for (idx, token) in tokens.iter().enumerate() {
        let Token::Action(action) = token else {
            continue;
        };
        if let Some(&(loop_start, _)) = range_stack.last() {
            enclosing_loop[idx] = Some((loop_start, n));
        }
        if is_control_block(action.body) || control_expression(action.body, "define").is_some() {
            let is_range = control_expression(action.body, "range").is_some();
            stack.push(Pending {
                start: idx,
                is_range,
                arms: Vec::new(),
                has_bare_else: false,
            });
            if is_range {
                range_stack.push((idx, n));
            }
        } else if action.body == "else" {
            if let Some(top) = stack.last_mut() {
                top.arms.push(idx);
                top.has_bare_else = true;
            }
        } else if control_expression(action.body, "else").is_some() {
            if let Some(top) = stack.last_mut() {
                top.arms.push(idx);
            }
        } else if action.body == "end"
            && let Some(closed) = stack.pop()
        {
            end_idx[closed.start] = idx;
            has_bare_else[closed.start] = closed.has_bare_else;
            for arm_idx in &closed.arms {
                owner_end[*arm_idx] = idx;
            }
            arms[closed.start] = closed.arms;
            if closed.is_range {
                range_stack.pop();
                enclosing_loop[idx] = Some((closed.start, idx));
                for loop_info in &mut enclosing_loop[closed.start + 1..idx] {
                    if let Some((s, end_slot)) = loop_info
                        && *s == closed.start
                    {
                        *end_slot = idx;
                    }
                }
            }
        }
    }

    BlockMap {
        arms,
        end_idx,
        has_bare_else,
        owner_end,
        enclosing_loop,
    }
}

fn is_branch_or_end_action(body: &str) -> bool {
    body == "end" || body == "else" || control_expression(body, "else").is_some()
}

fn comment_action_near_close(
    tokens: &[Token<'_>],
    trimmed_texts: &[&str],
    block_map: &BlockMap,
    action_idx: usize,
) -> bool {
    let n = tokens.len();
    // State index: `idx * 3 + prefix_slot`, where `prefix_slot` is:
    // 0 = no leading char yet, 1 = saw single `-`, 2 = saw single `!`.
    let mut visited = vec![false; (n + 1) * 3];
    let mut stack: Vec<(usize, u8)> = vec![(action_idx + 1, 0)];

    while let Some((idx, prefix)) = stack.pop() {
        if idx >= n {
            continue;
        }
        let state_id = idx * 3 + usize::from(prefix);
        if visited[state_id] {
            continue;
        }
        visited[state_id] = true;

        match tokens[idx] {
            Token::Text(_) => {
                let t = trimmed_texts[idx];
                if t.is_empty() {
                    stack.push((idx + 1, prefix));
                    continue;
                }
                match prefix {
                    0 => {
                        if t.starts_with('>') || t.starts_with("->") || t.starts_with("!>") {
                            return true;
                        }
                        if t == "-" {
                            stack.push((idx + 1, 1));
                        } else if t == "!" {
                            stack.push((idx + 1, 2));
                        }
                    }
                    1 | 2 if t.starts_with('>') => {
                        return true;
                    }
                    _ => {}
                }
            }
            Token::Comment(_) => {
                stack.push((idx + 1, prefix));
            }
            Token::Action(action) => {
                if action.body == "end" {
                    stack.push((idx + 1, prefix));
                    if let Some((loop_start, _)) = block_map.enclosing_loop[idx]
                        && block_map.end_idx[loop_start] == idx
                    {
                        stack.push((loop_start + 1, prefix));
                    }
                } else if action.body == "else" || control_expression(action.body, "else").is_some()
                {
                    let end = block_map.owner_end[idx];
                    if end < n {
                        stack.push((end + 1, prefix));
                    }
                } else if is_control_block(action.body) {
                    stack.push((idx + 1, prefix));
                    for &arm in &block_map.arms[idx] {
                        stack.push((arm + 1, prefix));
                    }
                    let is_range = control_expression(action.body, "range").is_some();
                    if (!block_map.has_bare_else[idx] || is_range) && block_map.end_idx[idx] < n {
                        stack.push((block_map.end_idx[idx] + 1, prefix));
                    }
                } else if control_expression(action.body, "define").is_some() {
                    if block_map.end_idx[idx] < n {
                        stack.push((block_map.end_idx[idx] + 1, prefix));
                    }
                } else if action.body == "break" {
                    if let Some((_, loop_end)) = block_map.enclosing_loop[idx]
                        && loop_end < n
                    {
                        stack.push((loop_end + 1, prefix));
                    }
                } else if action.body == "continue" {
                    if let Some((loop_start, loop_end)) = block_map.enclosing_loop[idx] {
                        stack.push((loop_start + 1, prefix));
                        if loop_end < n {
                            stack.push((loop_end + 1, prefix));
                        }
                    }
                } else {
                    stack.push((idx + 1, prefix));
                }
            }
        }
    }

    false
}

fn is_template_or_block_call(body: &str) -> bool {
    control_expression(body, "template").is_some()
        || control_expression(body, "block").is_some()
        || matches!(body, "template" | "block")
}

#[derive(Debug, Clone)]
enum BlockOrigin {
    Conditional {
        entry: HtmlState,
        branches: Vec<HtmlState>,
        has_bare_else: bool,
    },
    Define(HtmlState),
}

fn update_block_stack(
    block_stack: &mut Vec<BlockOrigin>,
    scanner: &mut HtmlScanner,
    body: &str,
) -> Result<(), TemplateError> {
    if control_expression(body, "if").is_some()
        || control_expression(body, "range").is_some()
        || control_expression(body, "with").is_some()
    {
        block_stack.push(BlockOrigin::Conditional {
            entry: scanner.state.clone(),
            branches: Vec::new(),
            has_bare_else: false,
        });
    } else if body == "else" {
        if let Some(BlockOrigin::Conditional {
            entry,
            branches,
            has_bare_else,
        }) = block_stack.last_mut()
        {
            branches.push(scanner.state.clone());
            *has_bare_else = true;
            scanner.state = entry.clone();
        }
    } else if control_expression(body, "else").is_some() {
        if let Some(BlockOrigin::Conditional {
            entry, branches, ..
        }) = block_stack.last_mut()
        {
            branches.push(scanner.state.clone());
            scanner.state = entry.clone();
        }
    } else if control_expression(body, "define").is_some() {
        block_stack.push(BlockOrigin::Define(scanner.state.clone()));
        scanner.state = HtmlState::Text;
    } else if body == "end"
        && let Some(origin) = block_stack.pop()
    {
        match origin {
            BlockOrigin::Conditional {
                entry,
                mut branches,
                has_bare_else,
            } => {
                branches.push(scanner.state.clone());
                if !has_bare_else {
                    branches.push(entry);
                }
                let mut converged = branches[0].clone();
                for branch_state in &branches[1..] {
                    converged = converge_html_states(&converged, branch_state)?;
                }
                scanner.state = converged;
            }
            BlockOrigin::Define(saved) => {
                if !matches!(scanner.state, HtmlState::Text) {
                    return Err(TemplateError::Parse(
                        "`define` block must end in HTML text context".to_owned(),
                    ));
                }
                scanner.state = saved;
            }
        }
    }
    Ok(())
}

fn converge_html_states(a: &HtmlState, b: &HtmlState) -> Result<HtmlState, TemplateError> {
    match (a, b) {
        (HtmlState::Text, HtmlState::Text) => Ok(HtmlState::Text),
        (HtmlState::Comment(a_sub), HtmlState::Comment(b_sub)) if a_sub == b_sub => {
            Ok(HtmlState::Comment(*a_sub))
        }
        (HtmlState::BogusComment(a_p), HtmlState::BogusComment(b_p)) if a_p == b_p => {
            Ok(HtmlState::BogusComment(*a_p))
        }
        (HtmlState::Style { end_tag_match: a_m }, HtmlState::Style { end_tag_match: b_m })
            if a_m == b_m =>
        {
            Ok(HtmlState::Style {
                end_tag_match: *a_m,
            })
        }
        (
            HtmlState::Rcdata {
                tag_name: a_t,
                end_tag_match: a_m,
            },
            HtmlState::Rcdata {
                tag_name: b_t,
                end_tag_match: b_m,
            },
        ) if a_t == b_t && a_m == b_m => Ok(HtmlState::Rcdata {
            tag_name: a_t,
            end_tag_match: *a_m,
        }),
        (
            HtmlState::RawText {
                tag_name: a_t,
                end_tag_match: a_m,
            },
            HtmlState::RawText {
                tag_name: b_t,
                end_tag_match: b_m,
            },
        ) if a_t == b_t && a_m == b_m => Ok(HtmlState::RawText {
            tag_name: a_t,
            end_tag_match: *a_m,
        }),
        (
            HtmlState::Script {
                script_escape: a_se,
                end_tag_match: a_etm,
                open_script_match: a_osm,
                js: a_js,
            },
            HtmlState::Script {
                script_escape: b_se,
                end_tag_match: b_etm,
                open_script_match: b_osm,
                js: b_js,
            },
        ) if a_se == b_se && a_etm == b_etm && a_osm == b_osm && a_js.structurally_eq(b_js) => {
            Ok(HtmlState::Script {
                script_escape: *a_se,
                end_tag_match: *a_etm,
                open_script_match: *a_osm,
                js: a_js.join(b_js),
            })
        }
        (
            HtmlState::Tag {
                sub: a_sub,
                tag_name: a_t,
                is_end_tag: a_e,
                attr_name: a_a,
            },
            HtmlState::Tag {
                sub: b_sub,
                tag_name: b_t,
                is_end_tag: b_e,
                attr_name: b_a,
            },
        ) if a_t == b_t && a_e == b_e && a_a == b_a => {
            let sub = converge_tag_sub_states(a_sub, b_sub)?;
            Ok(HtmlState::Tag {
                sub,
                tag_name: a_t.clone(),
                is_end_tag: *a_e,
                attr_name: a_a.clone(),
            })
        }
        _ => Err(TemplateError::Parse(
            "divergent HTML context across template control branches".to_owned(),
        )),
    }
}

fn converge_tag_sub_states(a: &TagSubState, b: &TagSubState) -> Result<TagSubState, TemplateError> {
    match (a, b) {
        (
            TagSubState::AttrValue {
                quote: a_q,
                kind: a_k,
                url: a_url,
                js: a_js,
            },
            TagSubState::AttrValue {
                quote: b_q,
                kind: b_k,
                url: b_url,
                js: b_js,
            },
        ) if a_q == b_q && a_k == b_k && a_js.structurally_eq(b_js) => {
            let url = converge_url_states(a_url, b_url);
            Ok(TagSubState::AttrValue {
                quote: *a_q,
                kind: *a_k,
                url,
                js: a_js.join(b_js),
            })
        }
        _ if a == b => Ok(a.clone()),
        _ => Err(TemplateError::Parse(
            "divergent HTML tag/attribute context across template control branches".to_owned(),
        )),
    }
}

fn converge_url_states(a: &UrlPrefixState, b: &UrlPrefixState) -> UrlPrefixState {
    match (a, b) {
        (UrlPrefixState::Safe, UrlPrefixState::Safe) => UrlPrefixState::Safe,
        (UrlPrefixState::UnsafeScheme, _) | (_, UrlPrefixState::UnsafeScheme) => {
            UrlPrefixState::UnsafeScheme
        }
        (UrlPrefixState::DynamicNoColon, _) | (_, UrlPrefixState::DynamicNoColon) => {
            UrlPrefixState::DynamicNoColon
        }
        (UrlPrefixState::Initial(a_p), UrlPrefixState::Initial(b_p)) => {
            if a_p == b_p {
                UrlPrefixState::Initial(a_p.clone())
            } else {
                UrlPrefixState::DynamicNoColon
            }
        }
        (UrlPrefixState::Safe, UrlPrefixState::Initial(p))
        | (UrlPrefixState::Initial(p), UrlPrefixState::Safe) => {
            if p.trim_start_matches(is_url_leading_ws_or_control)
                .is_empty()
            {
                UrlPrefixState::Initial(p.clone())
            } else {
                UrlPrefixState::DynamicNoColon
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AttrKind {
    Normal,
    Url,
    EventHandler,
    Style,
    Srcdoc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ScriptEscapeState {
    #[default]
    Data,
    Escaped,
    DoubleEscaped,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CommentSubState {
    Start,
    StartDash,
    Body,
    Dash,
    DoubleDash,
    EndBang,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BogusCommentPrefix {
    Other,
    Bang,
    BangDash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum JsSlashContext {
    #[default]
    RegexAllowed,
    DivisionOnly,
    AfterAmbiguousCloser,
    AmbiguousSlash,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum LineStartState {
    #[default]
    True,
    False,
    Ambiguous,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum JsLexMode {
    #[default]
    Code,
    String {
        quote: char,
        escaped: bool,
    },
    Template {
        escaped: bool,
        after_dollar: bool,
    },
    LineComment,
    BlockComment {
        after_star: bool,
    },
    Regex {
        escaped: bool,
        in_class: bool,
    },
}

const JS_REGEX_KEYWORDS: &[&str] = &[
    "return",
    "typeof",
    "void",
    "delete",
    "throw",
    "case",
    "in",
    "instanceof",
    "new",
    "else",
    "do",
    "await",
    "yield",
    "of",
    "break",
    "continue",
    "debugger",
    "default",
];

fn is_regex_keyword_prefix(s: &str) -> bool {
    JS_REGEX_KEYWORDS.iter().any(|kw| kw.starts_with(s))
}

fn ident_buffers_equivalent(a: &str, b: &str) -> bool {
    if a == b {
        return true;
    }
    if a.is_empty() || b.is_empty() {
        return false;
    }
    !is_regex_keyword_prefix(a) && !is_regex_keyword_prefix(b)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DotState {
    #[default]
    None,
    MemberAccess,
    DecimalFraction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum StmtAsiState {
    #[default]
    None,
    AfterBreakContinue,
    AfterLabel,
}

fn is_js_ident_char(ch: char) -> bool {
    ch.is_alphanumeric() || ch == '_' || ch == '$' || matches!(ch, '\u{200C}' | '\u{200D}')
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct JsState {
    mode: JsLexMode,
    slash_ctx: JsSlashContext,
    at_line_start: LineStartState,
    template_braces: Vec<usize>,
    ident_buf: String,
    dot_state: DotState,
    consecutive_dots: u8,
    stmt_asi: StmtAsiState,
    prev_sign: Option<char>,
}

impl JsState {
    fn quote(&self) -> Option<char> {
        match self.mode {
            JsLexMode::String { quote, .. } => Some(quote),
            JsLexMode::Template { .. } => Some('`'),
            _ => None,
        }
    }

    fn is_ambiguous_slash(&self) -> bool {
        self.slash_ctx == JsSlashContext::AmbiguousSlash
    }

    fn in_unsafe_escape_or_regex(&self) -> bool {
        matches!(
            self.mode,
            JsLexMode::Regex { .. }
                | JsLexMode::String { escaped: true, .. }
                | JsLexMode::Template { escaped: true, .. }
                | JsLexMode::Template {
                    after_dollar: true,
                    ..
                }
                | JsLexMode::BlockComment { after_star: true }
        )
    }

    fn set_slash_ctx(&mut self, next: JsSlashContext) {
        if self.slash_ctx != JsSlashContext::AmbiguousSlash {
            self.slash_ctx = next;
        }
    }

    fn structurally_eq(&self, other: &Self) -> bool {
        self.mode == other.mode
            && self.slash_ctx == other.slash_ctx
            && self.dot_state == other.dot_state
            && self.consecutive_dots == other.consecutive_dots
            && self.stmt_asi == other.stmt_asi
            && self.prev_sign == other.prev_sign
            && self.template_braces == other.template_braces
            && ident_buffers_equivalent(&self.ident_buf, &other.ident_buf)
    }

    fn join(&self, other: &Self) -> Self {
        Self {
            mode: self.mode,
            slash_ctx: self.slash_ctx,
            at_line_start: if self.at_line_start == other.at_line_start {
                self.at_line_start
            } else {
                LineStartState::Ambiguous
            },
            template_braces: self.template_braces.clone(),
            ident_buf: if self.ident_buf == other.ident_buf {
                self.ident_buf.clone()
            } else {
                "_".to_owned()
            },
            dot_state: self.dot_state,
            consecutive_dots: self.consecutive_dots,
            stmt_asi: self.stmt_asi,
            prev_sign: self.prev_sign,
        }
    }

    fn finish_ident(&mut self) {
        if self.ident_buf.is_empty() {
            return;
        }
        let is_member = self.dot_state == DotState::MemberAccess;
        let ident = self.ident_buf.as_str();
        let is_kw = !is_member && JS_REGEX_KEYWORDS.contains(&ident);
        let is_break_continue = !is_member && matches!(ident, "break" | "continue");
        self.set_slash_ctx(if is_kw {
            JsSlashContext::RegexAllowed
        } else {
            JsSlashContext::DivisionOnly
        });
        self.stmt_asi = if is_break_continue {
            StmtAsiState::AfterBreakContinue
        } else if !is_member && self.stmt_asi == StmtAsiState::AfterBreakContinue {
            StmtAsiState::AfterLabel
        } else {
            StmtAsiState::None
        };
        self.ident_buf.clear();
        self.dot_state = DotState::None;
    }

    fn advance_str(&mut self, text: &str, decode_entities: bool) {
        let mut i = 0;
        while i < text.len() {
            i += self.step_slice(&text[i..], decode_entities);
        }
    }

    fn step_slice(&mut self, rest: &str, decode_entities: bool) -> usize {
        let (ch, len) = next_js_char(rest, decode_entities);
        self.step(rest, ch, len, decode_entities)
    }

    fn step(&mut self, rest: &str, ch: char, len: usize, decode_entities: bool) -> usize {
        let after = &rest[len..];
        let next_ch = (!after.is_empty()).then(|| next_js_char(after, decode_entities).0);
        match self.mode {
            JsLexMode::LineComment | JsLexMode::BlockComment { .. } | JsLexMode::Regex { .. } => {
                self.step_comment_or_regex(ch, len, after, next_ch, decode_entities)
            }
            JsLexMode::String { .. } | JsLexMode::Template { .. } => {
                self.step_quoted(ch, len, after, next_ch, decode_entities)
            }
            JsLexMode::Code => self.step_code(rest, ch, len, after, next_ch, decode_entities),
        }
    }

    fn step_comment_or_regex(
        &mut self,
        ch: char,
        len: usize,
        after: &str,
        next_ch: Option<char>,
        decode_entities: bool,
    ) -> usize {
        let is_newline = matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}');
        match self.mode {
            JsLexMode::LineComment => {
                if is_newline {
                    self.mode = JsLexMode::Code;
                    self.at_line_start = LineStartState::True;
                    if self.stmt_asi != StmtAsiState::None {
                        self.set_slash_ctx(JsSlashContext::RegexAllowed);
                        self.stmt_asi = StmtAsiState::None;
                    }
                }
                len
            }
            JsLexMode::BlockComment { .. } => {
                if ch == '*' && next_ch == Some('/') {
                    let (_, slash_len) = next_js_char(after, decode_entities);
                    self.mode = JsLexMode::Code;
                    return len + slash_len;
                }
                if is_newline {
                    self.at_line_start = LineStartState::True;
                    if self.stmt_asi != StmtAsiState::None {
                        self.set_slash_ctx(JsSlashContext::RegexAllowed);
                        self.stmt_asi = StmtAsiState::None;
                    }
                }
                self.mode = JsLexMode::BlockComment {
                    after_star: ch == '*',
                };
                len
            }
            JsLexMode::Regex {
                mut escaped,
                mut in_class,
            } => {
                if is_newline {
                    self.mode = JsLexMode::Code;
                    self.at_line_start = LineStartState::True;
                    return len;
                }
                if escaped {
                    escaped = false;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == '[' {
                    in_class = true;
                } else if ch == ']' && in_class {
                    in_class = false;
                } else if ch == '/' && !in_class {
                    self.mode = JsLexMode::Code;
                    self.set_slash_ctx(JsSlashContext::DivisionOnly);
                    return len;
                }
                self.mode = JsLexMode::Regex { escaped, in_class };
                len
            }
            _ => len,
        }
    }

    fn step_quoted(
        &mut self,
        ch: char,
        len: usize,
        after: &str,
        next_ch: Option<char>,
        decode_entities: bool,
    ) -> usize {
        let is_newline = matches!(ch, '\n' | '\r' | '\u{2028}' | '\u{2029}');
        match self.mode {
            JsLexMode::Template {
                mut escaped,
                mut after_dollar,
            } => {
                if escaped {
                    if ch == '\r' && next_ch == Some('\n') {
                        let (_, lf_len) = next_js_char(after, decode_entities);
                        self.mode = JsLexMode::Template {
                            escaped: false,
                            after_dollar: false,
                        };
                        return len + lf_len;
                    }
                    escaped = false;
                    after_dollar = false;
                } else if after_dollar && ch == '{' {
                    self.template_braces.push(1);
                    self.mode = JsLexMode::Code;
                    self.transition_code_token(JsSlashContext::RegexAllowed);
                    return len;
                } else if ch == '\\' {
                    escaped = true;
                    after_dollar = false;
                } else if ch == '$' && next_ch == Some('{') {
                    let (_, brace_len) = next_js_char(after, decode_entities);
                    self.template_braces.push(1);
                    self.mode = JsLexMode::Code;
                    self.transition_code_token(JsSlashContext::RegexAllowed);
                    return len + brace_len;
                } else if ch == '$' {
                    after_dollar = true;
                } else if ch == '`' {
                    self.mode = JsLexMode::Code;
                    self.transition_code_token(JsSlashContext::DivisionOnly);
                    return len;
                } else {
                    after_dollar = false;
                }
                self.mode = JsLexMode::Template {
                    escaped,
                    after_dollar,
                };
                len
            }
            JsLexMode::String { quote, mut escaped } => {
                if escaped {
                    if ch == '\r' && next_ch == Some('\n') {
                        let (_, lf_len) = next_js_char(after, decode_entities);
                        self.mode = JsLexMode::String {
                            quote,
                            escaped: false,
                        };
                        return len + lf_len;
                    }
                    escaped = false;
                } else if is_newline {
                    self.mode = JsLexMode::Code;
                    self.at_line_start = LineStartState::True;
                    self.set_slash_ctx(JsSlashContext::RegexAllowed);
                    return len;
                } else if ch == '\\' {
                    escaped = true;
                } else if ch == quote {
                    self.mode = JsLexMode::Code;
                    self.set_slash_ctx(JsSlashContext::DivisionOnly);
                    return len;
                }
                self.mode = JsLexMode::String { quote, escaped };
                len
            }
            _ => len,
        }
    }

    fn consume_raw_action_value(&mut self) {
        if matches!(self.mode, JsLexMode::Code) {
            self.transition_code_token(JsSlashContext::DivisionOnly);
        }
    }

    fn step_code_comment_prefix(
        &mut self,
        rest: &str,
        ch: char,
        len: usize,
        after: &str,
        next_ch: Option<char>,
        decode_entities: bool,
    ) -> Option<usize> {
        if ch == '/' && next_ch == Some('/') {
            let (_, slash_len) = next_js_char(after, decode_entities);
            self.finish_ident();
            self.mode = JsLexMode::LineComment;
            self.consecutive_dots = 0;
            self.prev_sign = None;
            return Some(len + slash_len);
        }
        if ch == '/' && next_ch == Some('*') {
            let (_, star_len) = next_js_char(after, decode_entities);
            self.finish_ident();
            self.mode = JsLexMode::BlockComment { after_star: false };
            self.consecutive_dots = 0;
            self.prev_sign = None;
            return Some(len + star_len);
        }
        if let Some(comment_len) = starts_with_decoded_chars(rest, "<!--", decode_entities) {
            self.finish_ident();
            self.mode = JsLexMode::LineComment;
            self.consecutive_dots = 0;
            self.prev_sign = None;
            return Some(comment_len);
        }
        if self.at_line_start != LineStartState::False
            && let Some(comment_len) = starts_with_decoded_chars(rest, "-->", decode_entities)
        {
            self.finish_ident();
            if self.at_line_start == LineStartState::Ambiguous {
                self.slash_ctx = JsSlashContext::AmbiguousSlash;
            }
            self.mode = JsLexMode::LineComment;
            self.consecutive_dots = 0;
            self.prev_sign = None;
            return Some(comment_len);
        }
        None
    }

    fn step_code_slash(&mut self) {
        self.finish_ident();
        match self.slash_ctx {
            JsSlashContext::AmbiguousSlash => {}
            JsSlashContext::AfterAmbiguousCloser => {
                self.slash_ctx = JsSlashContext::AmbiguousSlash;
            }
            JsSlashContext::RegexAllowed => {
                self.mode = JsLexMode::Regex {
                    escaped: false,
                    in_class: false,
                };
            }
            JsSlashContext::DivisionOnly => {
                self.slash_ctx = JsSlashContext::RegexAllowed;
            }
        }
        self.at_line_start = LineStartState::False;
        self.dot_state = DotState::None;
        self.consecutive_dots = 0;
        self.stmt_asi = StmtAsiState::None;
        self.prev_sign = None;
    }

    fn transition_code_token(&mut self, next_slash: JsSlashContext) {
        self.finish_ident();
        self.set_slash_ctx(next_slash);
        self.at_line_start = LineStartState::False;
        self.dot_state = DotState::None;
        self.consecutive_dots = 0;
        self.stmt_asi = StmtAsiState::None;
        self.prev_sign = None;
    }

    fn step_code_dot(&mut self) {
        if self.consecutive_dots == 2 {
            // Spread operator `...`: reset dot tracking cleanly.
            self.transition_code_token(JsSlashContext::RegexAllowed);
            return;
        }
        let starts_with_digit = self
            .ident_buf
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_digit);
        if self.dot_state == DotState::None && starts_with_digit {
            self.transition_code_token(JsSlashContext::DivisionOnly);
            self.dot_state = DotState::DecimalFraction;
            self.consecutive_dots = 1;
        } else {
            let next_dots = self.consecutive_dots.saturating_add(1);
            self.transition_code_token(JsSlashContext::DivisionOnly);
            self.dot_state = DotState::MemberAccess;
            self.consecutive_dots = next_dots;
        }
    }

    fn step_code(
        &mut self,
        rest: &str,
        ch: char,
        len: usize,
        after: &str,
        next_ch: Option<char>,
        decode_entities: bool,
    ) -> usize {
        if let Some(comment_len) =
            self.step_code_comment_prefix(rest, ch, len, after, next_ch, decode_entities)
        {
            return comment_len;
        }

        match ch {
            '/' => self.step_code_slash(),
            '"' | '\'' => {
                self.mode = JsLexMode::String {
                    quote: ch,
                    escaped: false,
                };
                self.transition_code_token(JsSlashContext::DivisionOnly);
            }
            '`' => {
                self.mode = JsLexMode::Template {
                    escaped: false,
                    after_dollar: false,
                };
                self.transition_code_token(JsSlashContext::DivisionOnly);
            }
            '{' => {
                if let Some(depth) = self.template_braces.last_mut() {
                    *depth += 1;
                }
                self.transition_code_token(JsSlashContext::RegexAllowed);
            }
            '}' => {
                if let Some(depth) = self.template_braces.last_mut() {
                    *depth -= 1;
                    if *depth == 0 {
                        self.template_braces.pop();
                        self.mode = JsLexMode::Template {
                            escaped: false,
                            after_dollar: false,
                        };
                        self.transition_code_token(JsSlashContext::DivisionOnly);
                        return len;
                    }
                }
                self.transition_code_token(JsSlashContext::AfterAmbiguousCloser);
            }
            ')' => self.transition_code_token(JsSlashContext::AfterAmbiguousCloser),
            '.' => self.step_code_dot(),
            '+' | '-' => {
                self.finish_ident();
                if self.prev_sign == Some(ch) {
                    self.set_slash_ctx(JsSlashContext::DivisionOnly);
                    self.prev_sign = None;
                } else {
                    self.set_slash_ctx(JsSlashContext::RegexAllowed);
                    self.prev_sign = Some(ch);
                }
                self.at_line_start = LineStartState::False;
                self.dot_state = DotState::None;
                self.consecutive_dots = 0;
                self.stmt_asi = StmtAsiState::None;
            }
            c if is_js_ident_char(c) => {
                if self.slash_ctx == JsSlashContext::AfterAmbiguousCloser {
                    self.slash_ctx = JsSlashContext::DivisionOnly;
                }
                self.ident_buf.push(c);
                self.at_line_start = LineStartState::False;
                self.consecutive_dots = 0;
                self.prev_sign = None;
            }
            '\n' | '\r' | '\u{2028}' | '\u{2029}' => {
                self.finish_ident();
                self.at_line_start = LineStartState::True;
                if self.dot_state == DotState::DecimalFraction {
                    self.dot_state = DotState::None;
                }
                self.consecutive_dots = 0;
                if self.stmt_asi != StmtAsiState::None {
                    self.set_slash_ctx(JsSlashContext::RegexAllowed);
                    self.stmt_asi = StmtAsiState::None;
                }
                self.prev_sign = None;
            }
            c if c.is_whitespace() || c == '\u{FEFF}' => {
                self.finish_ident();
                if self.dot_state == DotState::DecimalFraction {
                    self.dot_state = DotState::None;
                }
                self.consecutive_dots = 0;
                self.prev_sign = None;
            }
            '(' | '[' | ',' | ';' | ':' | '?' | '=' | '!' | '&' | '|' | '^' | '~' | '*' | '%'
            | '<' | '>' => self.transition_code_token(JsSlashContext::RegexAllowed),
            _ => self.transition_code_token(JsSlashContext::DivisionOnly),
        }
        len
    }
}

fn next_js_char(rest: &str, decode_entities: bool) -> (char, usize) {
    if decode_entities && let Some(decoded) = decode_next_html_entity(rest) {
        return decoded;
    }
    let ch = rest.chars().next().expect("non-empty slice");
    (ch, ch.len_utf8())
}

fn next_url_char(rest: &str) -> (char, usize, bool) {
    if let Some((ch, len)) = decode_next_html_entity(rest) {
        return (ch, len, false);
    }
    let ch = rest.chars().next().expect("non-empty slice");
    (ch, ch.len_utf8(), true)
}

fn starts_with_decoded_chars(rest: &str, target: &str, decode_entities: bool) -> Option<usize> {
    let mut offset = 0;
    for expected in target.chars() {
        if offset >= rest.len() {
            return None;
        }
        let (ch, len) = next_js_char(&rest[offset..], decode_entities);
        if ch != expected {
            return None;
        }
        offset += len;
    }
    Some(offset)
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum UrlPrefixState {
    Initial(String),
    Safe,
    UnsafeScheme,
    DynamicNoColon,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TagSubState {
    Start,
    EndTagOpen,
    TagName,
    BeforeAttrName,
    AttrName,
    AfterAttrName,
    BeforeAttrValue,
    AttrValue {
        quote: Option<char>,
        kind: AttrKind,
        url: UrlPrefixState,
        js: JsState,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum HtmlState {
    #[default]
    Text,
    Comment(CommentSubState),
    BogusComment(BogusCommentPrefix),
    Tag {
        sub: TagSubState,
        tag_name: String,
        is_end_tag: bool,
        attr_name: String,
    },
    Script {
        script_escape: ScriptEscapeState,
        end_tag_match: u8,
        open_script_match: u8,
        js: JsState,
    },
    Style {
        end_tag_match: u8,
    },
    Rcdata {
        tag_name: &'static str,
        end_tag_match: u8,
    },
    RawText {
        tag_name: &'static str,
        end_tag_match: u8,
    },
}

#[derive(Debug, Default, Clone)]
struct HtmlScanner {
    state: HtmlState,
}

impl HtmlScanner {
    fn in_srcdoc(&self) -> bool {
        match &self.state {
            HtmlState::Tag {
                sub: TagSubState::BeforeAttrValue,
                attr_name,
                ..
            } => classify_attr(attr_name) == AttrKind::Srcdoc,
            HtmlState::Tag {
                sub:
                    TagSubState::AttrValue {
                        kind: AttrKind::Srcdoc,
                        ..
                    },
                ..
            } => true,
            _ => false,
        }
    }

    fn allows_template_or_block_call(&self) -> bool {
        matches!(self.state, HtmlState::Text | HtmlState::Rcdata { .. })
    }

    fn validate_action_in_context(
        &self,
        body: &str,
        tokens: &[Token<'_>],
        trimmed_texts: &[&str],
        block_map: &BlockMap,
        action_idx: usize,
    ) -> Result<(), TemplateError> {
        let is_branch_or_end = is_branch_or_end_action(body);
        match &self.state {
            HtmlState::Comment(sub) => {
                if !is_branch_or_end {
                    if *sub != CommentSubState::Body {
                        return Err(TemplateError::Parse(
                            "dynamic template actions are not allowed at HTML comment boundaries"
                                .to_owned(),
                        ));
                    }
                    if comment_action_near_close(tokens, trimmed_texts, block_map, action_idx) {
                        return Err(TemplateError::Parse(
                            "dynamic template actions are not allowed immediately before HTML comment close tokens"
                                .to_owned(),
                        ));
                    }
                }
            }
            HtmlState::BogusComment(BogusCommentPrefix::Bang | BogusCommentPrefix::BangDash) => {
                if !is_branch_or_end {
                    return Err(TemplateError::Parse(
                        "dynamic template actions are not allowed immediately after `<!`"
                            .to_owned(),
                    ));
                }
            }
            HtmlState::Script {
                end_tag_match,
                open_script_match,
                js,
                ..
            } => {
                if js.is_ambiguous_slash() {
                    return Err(TemplateError::Parse(
                        "dynamic template actions are not allowed after ambiguous `/` following `)` or `}` in script"
                            .to_owned(),
                    ));
                }
                if !is_branch_or_end {
                    if *end_tag_match >= 2 || *open_script_match >= 2 {
                        return Err(TemplateError::Parse(
                            "dynamic template actions are not allowed inside partial `<script>` / `</script>` tags"
                                .to_owned(),
                        ));
                    }
                    if js.in_unsafe_escape_or_regex() {
                        return Err(TemplateError::Parse(
                            "dynamic template actions are not allowed inside JS regex literals or immediately after trailing `\\`, `$`, or `*`"
                                .to_owned(),
                        ));
                    }
                }
            }
            HtmlState::Style { end_tag_match }
            | HtmlState::Rcdata { end_tag_match, .. }
            | HtmlState::RawText { end_tag_match, .. } => {
                if !is_branch_or_end && *end_tag_match >= 2 {
                    return Err(TemplateError::Parse(
                        "dynamic template actions are not allowed inside partial end tags"
                            .to_owned(),
                    ));
                }
            }
            HtmlState::Tag {
                sub: TagSubState::AttrValue { kind, url, js, .. },
                ..
            } => {
                if *kind == AttrKind::EventHandler {
                    if js.is_ambiguous_slash() {
                        return Err(TemplateError::Parse(
                            "dynamic template actions are not allowed after ambiguous `/` following `)` or `}` in event handler"
                                .to_owned(),
                        ));
                    }
                    if !is_branch_or_end && js.in_unsafe_escape_or_regex() {
                        return Err(TemplateError::Parse(
                            "dynamic template actions are not allowed inside JS regex literals or immediately after trailing `\\`, `$`, or `*`"
                                .to_owned(),
                        ));
                    }
                }
                if *kind == AttrKind::Url
                    && *url == UrlPrefixState::UnsafeScheme
                    && !is_branch_or_end
                {
                    return Err(TemplateError::Parse(
                        "dynamic template actions are not allowed inside URLs with unsafe schemes"
                            .to_owned(),
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn advance_text(&mut self, text: &str) -> Result<(), TemplateError> {
        let mut i = 0;
        while i < text.len() {
            let rest = &text[i..];
            match &mut self.state {
                HtmlState::Text => {
                    if rest.starts_with("<!--") {
                        self.state = HtmlState::Comment(CommentSubState::Start);
                        i += 4;
                    } else if rest.starts_with('<') {
                        self.state = HtmlState::Tag {
                            sub: TagSubState::Start,
                            tag_name: String::new(),
                            is_end_tag: false,
                            attr_name: String::new(),
                        };
                        i += 1;
                    } else if let Some(rel) = rest.find('<') {
                        i += rel;
                    } else {
                        break;
                    }
                }
                HtmlState::Comment(_) => {
                    i += self.advance_comment(rest);
                }
                HtmlState::BogusComment(_) => {
                    i += self.advance_bogus_comment(rest);
                }
                HtmlState::Script { .. } => {
                    i += self.advance_script(rest);
                }
                HtmlState::Style { .. } => {
                    i += self.advance_raw_or_rcdata(rest, "style", false);
                }
                HtmlState::Rcdata { tag_name, .. } => {
                    let name = *tag_name;
                    i += self.advance_raw_or_rcdata(rest, name, true);
                }
                HtmlState::RawText { tag_name, .. } => {
                    let name = *tag_name;
                    i += self.advance_raw_or_rcdata(rest, name, false);
                }
                HtmlState::Tag { .. } => {
                    i += self.advance_tag(rest)?;
                }
            }
        }
        Ok(())
    }

    fn advance_comment(&mut self, rest: &str) -> usize {
        let HtmlState::Comment(sub) = &mut self.state else {
            unreachable!();
        };
        let ch = rest.chars().next().expect("non-empty rest");
        match *sub {
            CommentSubState::Start | CommentSubState::StartDash => match ch {
                '>' => self.state = HtmlState::Text,
                '-' => {
                    *sub = if *sub == CommentSubState::Start {
                        CommentSubState::StartDash
                    } else {
                        CommentSubState::DoubleDash
                    };
                }
                _ => *sub = CommentSubState::Body,
            },
            CommentSubState::Body => {
                if ch == '-' {
                    *sub = CommentSubState::Dash;
                }
            }
            CommentSubState::Dash => {
                if ch == '-' {
                    *sub = CommentSubState::DoubleDash;
                } else {
                    *sub = CommentSubState::Body;
                }
            }
            CommentSubState::DoubleDash => match ch {
                '>' => self.state = HtmlState::Text,
                '!' => *sub = CommentSubState::EndBang,
                '-' => {}
                _ => *sub = CommentSubState::Body,
            },
            CommentSubState::EndBang => match ch {
                '>' => self.state = HtmlState::Text,
                '-' => *sub = CommentSubState::Dash,
                _ => *sub = CommentSubState::Body,
            },
        }
        ch.len_utf8()
    }

    fn advance_bogus_comment(&mut self, rest: &str) -> usize {
        let HtmlState::BogusComment(prefix) = &mut self.state else {
            unreachable!();
        };
        match *prefix {
            BogusCommentPrefix::Bang => {
                let ch = rest.chars().next().expect("non-empty rest");
                match ch {
                    '-' => *prefix = BogusCommentPrefix::BangDash,
                    '>' => self.state = HtmlState::Text,
                    _ => *prefix = BogusCommentPrefix::Other,
                }
                ch.len_utf8()
            }
            BogusCommentPrefix::BangDash => {
                let ch = rest.chars().next().expect("non-empty rest");
                match ch {
                    '-' => self.state = HtmlState::Comment(CommentSubState::Start),
                    '>' => self.state = HtmlState::Text,
                    _ => *prefix = BogusCommentPrefix::Other,
                }
                ch.len_utf8()
            }
            BogusCommentPrefix::Other => {
                if let Some(rel) = rest.find('>') {
                    self.state = HtmlState::Text;
                    rel + 1
                } else {
                    rest.len()
                }
            }
        }
    }

    fn advance_raw_or_rcdata(&mut self, rest: &str, tag: &'static str, is_rcdata: bool) -> usize {
        let target_len = 2 + tag.len();
        let current_match = match &self.state {
            HtmlState::Style { end_tag_match }
            | HtmlState::Rcdata { end_tag_match, .. }
            | HtmlState::RawText { end_tag_match, .. } => *end_tag_match,
            _ => unreachable!(),
        };
        let ch = rest.chars().next().expect("non-empty rest");
        if usize::from(current_match) == target_len
            && (ch == '>' || ch == '/' || ch.is_ascii_whitespace())
        {
            self.state = HtmlState::Tag {
                sub: TagSubState::TagName,
                tag_name: tag.to_owned(),
                is_end_tag: true,
                attr_name: String::new(),
            };
            return 0;
        }
        if let Some(tag_len) = starts_with_end_tag_ci(rest, tag) {
            self.state = HtmlState::Tag {
                sub: TagSubState::TagName,
                tag_name: tag.to_owned(),
                is_end_tag: true,
                attr_name: String::new(),
            };
            return tag_len;
        }
        let next_match = update_end_tag_match(current_match, ch, tag);
        match &mut self.state {
            HtmlState::Style { end_tag_match }
            | HtmlState::Rcdata { end_tag_match, .. }
            | HtmlState::RawText { end_tag_match, .. } => *end_tag_match = next_match,
            _ => unreachable!(),
        }
        let _ = is_rcdata;
        ch.len_utf8()
    }

    fn enter_script_end_tag(&mut self) {
        self.state = HtmlState::Tag {
            sub: TagSubState::TagName,
            tag_name: "script".to_owned(),
            is_end_tag: true,
            attr_name: String::new(),
        };
    }

    fn advance_script(&mut self, rest: &str) -> usize {
        let first_ch = rest.chars().next().expect("non-empty rest");
        let HtmlState::Script {
            script_escape,
            end_tag_match,
            open_script_match,
            js,
        } = &mut self.state
        else {
            unreachable!();
        };
        let is_tag_delim = first_ch == '>' || first_ch == '/' || first_ch.is_ascii_whitespace();
        if usize::from(*end_tag_match) == 8 && is_tag_delim {
            match *script_escape {
                ScriptEscapeState::Data | ScriptEscapeState::Escaped => {
                    self.enter_script_end_tag();
                    return 0;
                }
                ScriptEscapeState::DoubleEscaped => {
                    *script_escape = ScriptEscapeState::Escaped;
                    *end_tag_match = 0;
                    *open_script_match = 0;
                }
            }
        }
        if *script_escape == ScriptEscapeState::Escaped
            && usize::from(*open_script_match) == 7
            && is_tag_delim
        {
            *script_escape = ScriptEscapeState::DoubleEscaped;
            *open_script_match = 0;
        }

        if *script_escape != ScriptEscapeState::DoubleEscaped
            && let Some(tag_len) = starts_with_end_tag_ci(rest, "script")
        {
            self.enter_script_end_tag();
            return tag_len;
        }
        if *script_escape == ScriptEscapeState::Data && rest.starts_with("<!--") {
            *script_escape = ScriptEscapeState::Escaped;
            *end_tag_match = 0;
            *open_script_match = 0;
            js.advance_str("<!--", false);
            return 4;
        }
        if *script_escape != ScriptEscapeState::Data && rest.starts_with("-->") {
            *script_escape = ScriptEscapeState::Data;
            *end_tag_match = 0;
            *open_script_match = 0;
            js.advance_str("-->", false);
            return 3;
        }
        if *script_escape == ScriptEscapeState::Escaped
            && let Some(tag_len) = starts_with_open_tag_ci(rest, "script")
        {
            *script_escape = ScriptEscapeState::DoubleEscaped;
            *end_tag_match = 0;
            *open_script_match = 0;
            js.advance_str(&rest[..tag_len], false);
            return tag_len;
        }
        if *script_escape == ScriptEscapeState::DoubleEscaped
            && let Some(tag_len) = starts_with_end_tag_ci(rest, "script")
        {
            *script_escape = ScriptEscapeState::Escaped;
            *end_tag_match = 0;
            *open_script_match = 0;
            js.advance_str(&rest[..tag_len], false);
            return tag_len;
        }

        let consumed = js.step_slice(rest, false);
        for c in rest[..consumed].chars() {
            *end_tag_match = update_end_tag_match(*end_tag_match, c, "script");
            *open_script_match = if *script_escape == ScriptEscapeState::Escaped {
                update_open_tag_match(*open_script_match, c, "script")
            } else {
                0
            };
        }
        consumed
    }

    fn advance_tag(&mut self, rest: &str) -> Result<usize, TemplateError> {
        let HtmlState::Tag {
            sub,
            tag_name,
            is_end_tag,
            attr_name,
        } = &mut self.state
        else {
            unreachable!();
        };
        let ch = rest.chars().next().expect("non-empty rest");
        let mut next_state: Option<HtmlState> = None;
        let consumed = match sub {
            TagSubState::Start => {
                if ch == '/' {
                    *is_end_tag = true;
                    *sub = TagSubState::EndTagOpen;
                } else if ch == '!' {
                    next_state = Some(HtmlState::BogusComment(BogusCommentPrefix::Bang));
                } else if ch == '?' {
                    next_state = Some(HtmlState::BogusComment(BogusCommentPrefix::Other));
                } else if ch == '>' {
                    next_state = Some(HtmlState::Text);
                } else if ch.is_ascii_alphabetic() {
                    tag_name.push(ch.to_ascii_lowercase());
                    *sub = TagSubState::TagName;
                } else {
                    next_state = Some(HtmlState::Text);
                }
                ch.len_utf8()
            }
            TagSubState::EndTagOpen => {
                if ch == '>' {
                    next_state = Some(HtmlState::Text);
                } else if ch.is_ascii_alphabetic() {
                    tag_name.push(ch.to_ascii_lowercase());
                    *sub = TagSubState::TagName;
                } else {
                    next_state = Some(HtmlState::BogusComment(BogusCommentPrefix::Other));
                }
                ch.len_utf8()
            }
            TagSubState::TagName => {
                if ch == '>' {
                    next_state = Some(finish_tag_state(tag_name, *is_end_tag));
                } else if ch.is_ascii_whitespace() || ch == '/' {
                    *sub = TagSubState::BeforeAttrName;
                } else {
                    tag_name.push(ch.to_ascii_lowercase());
                }
                ch.len_utf8()
            }
            TagSubState::BeforeAttrName | TagSubState::AttrName | TagSubState::AfterAttrName => {
                advance_tag_attr_name(ch, sub, tag_name, *is_end_tag, attr_name, &mut next_state)
            }
            TagSubState::BeforeAttrValue => advance_before_attr_value(
                rest,
                ch,
                sub,
                tag_name,
                *is_end_tag,
                attr_name,
                &mut next_state,
            )?,
            TagSubState::AttrValue { .. } => advance_attr_value(
                rest,
                ch,
                sub,
                tag_name,
                *is_end_tag,
                attr_name,
                &mut next_state,
            )?,
        };
        if let Some(state) = next_state {
            self.state = state;
        }
        Ok(consumed)
    }

    fn escape_fn_for_action(&mut self) -> Result<&'static str, TemplateError> {
        match &mut self.state {
            HtmlState::Text | HtmlState::Rcdata { .. } | HtmlState::RawText { .. } => {
                Ok(AUTO_ESCAPE_FN)
            }
            HtmlState::Comment(CommentSubState::Body) | HtmlState::BogusComment(_) => {
                Ok(AUTO_ESCAPE_COMMENT_FN)
            }
            HtmlState::Comment(_) => Err(TemplateError::Parse(
                "dynamic template actions are not allowed at HTML comment boundaries".to_owned(),
            )),
            HtmlState::Style { .. } => Ok(AUTO_ESCAPE_CSS_FN),
            HtmlState::Script { js, .. } => Ok(match js.quote() {
                Some('`') => AUTO_ESCAPE_JS_TPL_FN,
                Some(_) => AUTO_ESCAPE_JS_STR_FN,
                None => {
                    js.consume_raw_action_value();
                    AUTO_ESCAPE_JS_RAW_FN
                }
            }),
            HtmlState::Tag { sub, attr_name, .. } => escape_fn_for_tag(sub, attr_name),
        }
    }
}

fn update_end_tag_match(current: u8, ch: char, tag: &str) -> u8 {
    let target = tag.as_bytes();
    let idx = usize::from(current);
    let lower = ch.to_ascii_lowercase();
    if idx == 0 {
        u8::from(ch == '<')
    } else if idx == 1 {
        if ch == '/' { 2 } else { u8::from(ch == '<') }
    } else if idx - 2 < target.len() && lower.is_ascii() && (lower as u8) == target[idx - 2] {
        current + 1
    } else {
        u8::from(ch == '<')
    }
}

fn update_open_tag_match(current: u8, ch: char, tag: &str) -> u8 {
    let target = tag.as_bytes();
    let idx = usize::from(current);
    let lower = ch.to_ascii_lowercase();
    if idx == 0 {
        u8::from(ch == '<')
    } else if idx - 1 < target.len() && lower.is_ascii() && (lower as u8) == target[idx - 1] {
        current + 1
    } else {
        u8::from(ch == '<')
    }
}

fn advance_tag_attr_name(
    ch: char,
    sub: &mut TagSubState,
    tag_name: &str,
    is_end_tag: bool,
    attr_name: &mut String,
    next_state: &mut Option<HtmlState>,
) -> usize {
    if ch == '>' {
        *next_state = Some(finish_tag_state(tag_name, is_end_tag));
        return ch.len_utf8();
    }
    match sub {
        TagSubState::BeforeAttrName => {
            if !ch.is_ascii_whitespace() && ch != '/' {
                attr_name.clear();
                attr_name.push(ch.to_ascii_lowercase());
                *sub = TagSubState::AttrName;
            }
        }
        TagSubState::AttrName => {
            if ch == '=' {
                *sub = TagSubState::BeforeAttrValue;
            } else if ch.is_ascii_whitespace() {
                *sub = TagSubState::AfterAttrName;
            } else if ch == '/' {
                attr_name.clear();
                *sub = TagSubState::BeforeAttrName;
            } else {
                attr_name.push(ch.to_ascii_lowercase());
            }
        }
        TagSubState::AfterAttrName => {
            if ch == '=' {
                *sub = TagSubState::BeforeAttrValue;
            } else if ch == '/' {
                attr_name.clear();
                *sub = TagSubState::BeforeAttrName;
            } else if !ch.is_ascii_whitespace() {
                attr_name.clear();
                attr_name.push(ch.to_ascii_lowercase());
                *sub = TagSubState::AttrName;
            }
        }
        _ => unreachable!(),
    }
    ch.len_utf8()
}

fn advance_before_attr_value(
    rest: &str,
    ch: char,
    sub: &mut TagSubState,
    tag_name: &str,
    is_end_tag: bool,
    attr_name: &str,
    next_state: &mut Option<HtmlState>,
) -> Result<usize, TemplateError> {
    if ch == '>' {
        *next_state = Some(finish_tag_state(tag_name, is_end_tag));
        return Ok(ch.len_utf8());
    }
    if ch == '"' || ch == '\'' {
        let kind = classify_attr(attr_name);
        *sub = TagSubState::AttrValue {
            quote: Some(ch),
            kind,
            url: UrlPrefixState::Initial(String::new()),
            js: JsState::default(),
        };
        return Ok(ch.len_utf8());
    }
    if ch.is_ascii_whitespace() {
        return Ok(ch.len_utf8());
    }
    let kind = classify_attr(attr_name);
    let mut url = UrlPrefixState::Initial(String::new());
    let mut js = JsState::default();
    let end = rest
        .find(|c: char| c.is_ascii_whitespace() || c == '>')
        .unwrap_or(rest.len());
    let consumed = if kind == AttrKind::Url {
        let (decoded_ch, len, is_literal) = next_url_char(&rest[..end]);
        advance_url_prefix(decoded_ch, is_literal, &mut url)?;
        len
    } else if kind == AttrKind::EventHandler {
        js.step_slice(&rest[..end], true)
    } else {
        ch.len_utf8()
    };
    *sub = TagSubState::AttrValue {
        quote: None,
        kind,
        url,
        js,
    };
    Ok(consumed)
}

fn advance_attr_value(
    rest: &str,
    ch: char,
    sub: &mut TagSubState,
    tag_name: &str,
    is_end_tag: bool,
    attr_name: &mut String,
    next_state: &mut Option<HtmlState>,
) -> Result<usize, TemplateError> {
    let TagSubState::AttrValue {
        quote,
        kind,
        url,
        js,
    } = sub
    else {
        unreachable!();
    };
    if *quote == Some(ch) || (quote.is_none() && ch.is_ascii_whitespace()) {
        attr_name.clear();
        *sub = TagSubState::BeforeAttrName;
        return Ok(ch.len_utf8());
    }
    if quote.is_none() && ch == '>' {
        *next_state = Some(finish_tag_state(tag_name, is_end_tag));
        return Ok(ch.len_utf8());
    }
    if *kind == AttrKind::Url {
        let end = if let Some(q) = *quote {
            rest.find(q).unwrap_or(rest.len())
        } else {
            rest.find(|c: char| c.is_ascii_whitespace() || c == '>')
                .unwrap_or(rest.len())
        };
        let (decoded_ch, len, is_literal) = next_url_char(&rest[..end]);
        advance_url_prefix(decoded_ch, is_literal, url)?;
        return Ok(len);
    }
    if *kind == AttrKind::EventHandler {
        let end = if let Some(q) = *quote {
            rest.find(q).unwrap_or(rest.len())
        } else {
            rest.find(|c: char| c.is_ascii_whitespace() || c == '>')
                .unwrap_or(rest.len())
        };
        return Ok(js.step_slice(&rest[..end], true));
    }
    Ok(ch.len_utf8())
}

fn escape_fn_for_url_attr(
    url: &mut UrlPrefixState,
    unquoted: bool,
) -> Result<&'static str, TemplateError> {
    match url {
        UrlPrefixState::Safe => Ok(if unquoted {
            AUTO_ESCAPE_UNQUOTED_FN
        } else {
            AUTO_ESCAPE_FN
        }),
        UrlPrefixState::Initial(_) => {
            *url = UrlPrefixState::DynamicNoColon;
            Ok(if unquoted {
                AUTO_ESCAPE_URL_START_UNQUOTED_FN
            } else {
                AUTO_ESCAPE_URL_START_FN
            })
        }
        UrlPrefixState::DynamicNoColon => Ok(if unquoted {
            AUTO_ESCAPE_URL_CONT_NO_COLON_UNQUOTED_FN
        } else {
            AUTO_ESCAPE_URL_CONT_NO_COLON_FN
        }),
        UrlPrefixState::UnsafeScheme => Err(TemplateError::Parse(
            "dynamic template actions are not allowed inside URLs with unsafe schemes".to_owned(),
        )),
    }
}

fn escape_fn_for_tag(
    sub: &mut TagSubState,
    attr_name: &str,
) -> Result<&'static str, TemplateError> {
    match sub {
        TagSubState::BeforeAttrValue => {
            let kind = classify_attr(attr_name);
            if kind == AttrKind::Srcdoc {
                return Err(TemplateError::Parse(
                    "dynamic template actions are not allowed inside `srcdoc` attributes"
                        .to_owned(),
                ));
            }
            let mut js = JsState::default();
            if kind == AttrKind::EventHandler {
                js.set_slash_ctx(JsSlashContext::DivisionOnly);
                js.at_line_start = LineStartState::False;
            }
            *sub = TagSubState::AttrValue {
                quote: None,
                kind,
                url: UrlPrefixState::DynamicNoColon,
                js,
            };
            Ok(match kind {
                AttrKind::Normal => AUTO_ESCAPE_UNQUOTED_FN,
                AttrKind::Url => AUTO_ESCAPE_URL_START_UNQUOTED_FN,
                AttrKind::Style => AUTO_ESCAPE_CSS_UNQUOTED_FN,
                AttrKind::EventHandler => AUTO_ESCAPE_JS_ATTR_RAW_UNQUOTED_FN,
                AttrKind::Srcdoc => unreachable!(),
            })
        }
        TagSubState::AttrValue {
            quote,
            kind,
            url,
            js,
        } => {
            let unquoted = quote.is_none();
            match *kind {
                AttrKind::Srcdoc => Err(TemplateError::Parse(
                    "dynamic template actions are not allowed inside `srcdoc` attributes"
                        .to_owned(),
                )),
                AttrKind::Url => escape_fn_for_url_attr(url, unquoted),
                AttrKind::Normal => Ok(if unquoted {
                    AUTO_ESCAPE_UNQUOTED_FN
                } else {
                    AUTO_ESCAPE_FN
                }),
                AttrKind::Style => Ok(if unquoted {
                    AUTO_ESCAPE_CSS_UNQUOTED_FN
                } else {
                    AUTO_ESCAPE_CSS_FN
                }),
                AttrKind::EventHandler => match (unquoted, js.quote()) {
                    (false, Some('`')) => Ok(AUTO_ESCAPE_JS_ATTR_TPL_FN),
                    (false, Some(_)) => Ok(AUTO_ESCAPE_JS_ATTR_STR_FN),
                    (false, None) => {
                        js.consume_raw_action_value();
                        Ok(AUTO_ESCAPE_JS_ATTR_RAW_FN)
                    }
                    (true, Some('`')) => Ok(AUTO_ESCAPE_JS_ATTR_TPL_UNQUOTED_FN),
                    (true, Some(_)) => Ok(AUTO_ESCAPE_JS_ATTR_STR_UNQUOTED_FN),
                    (true, None) => {
                        js.consume_raw_action_value();
                        Ok(AUTO_ESCAPE_JS_ATTR_RAW_UNQUOTED_FN)
                    }
                },
            }
        }
        TagSubState::Start
        | TagSubState::EndTagOpen
        | TagSubState::TagName
        | TagSubState::BeforeAttrName
        | TagSubState::AttrName
        | TagSubState::AfterAttrName => Err(TemplateError::Parse(
            "dynamic template expressions are not allowed in HTML tag names or attribute names"
                .to_owned(),
        )),
    }
}

fn is_url_leading_ws_or_control(ch: char) -> bool {
    ch.is_ascii_whitespace() || ch.is_ascii_control()
}

fn advance_url_prefix(
    ch: char,
    is_literal: bool,
    state: &mut UrlPrefixState,
) -> Result<(), TemplateError> {
    match state {
        UrlPrefixState::Safe | UrlPrefixState::UnsafeScheme => {}
        UrlPrefixState::DynamicNoColon => {
            if is_literal && matches!(ch, '/' | '?' | '#') {
                *state = UrlPrefixState::Safe;
            } else if ch == ':' {
                return Err(TemplateError::Parse(
                    "colon is not allowed after dynamic URL prefix without a preceding `/`, `?`, or `#`"
                        .to_owned(),
                ));
            }
        }
        UrlPrefixState::Initial(prefix) => {
            if is_literal && matches!(ch, '/' | '?' | '#') {
                *state = UrlPrefixState::Safe;
            } else if ch == ':' {
                let trimmed = prefix.trim_start_matches(is_url_leading_ws_or_control);
                let is_safe_scheme = !trimmed.bytes().any(|b| b <= 0x20 || b == 0x7F)
                    && (trimmed.eq_ignore_ascii_case("http")
                        || trimmed.eq_ignore_ascii_case("https")
                        || trimmed.eq_ignore_ascii_case("mailto")
                        || trimmed.eq_ignore_ascii_case("tel"));
                *state = if is_safe_scheme {
                    UrlPrefixState::Safe
                } else {
                    UrlPrefixState::UnsafeScheme
                };
            } else {
                prefix.push(ch);
            }
        }
    }
    Ok(())
}

fn finish_tag_state(tag_name: &str, is_end_tag: bool) -> HtmlState {
    if is_end_tag {
        return HtmlState::Text;
    }
    match tag_name {
        "script" => HtmlState::Script {
            script_escape: ScriptEscapeState::Data,
            end_tag_match: 0,
            open_script_match: 0,
            js: JsState::default(),
        },
        "style" => HtmlState::Style { end_tag_match: 0 },
        "title" => HtmlState::Rcdata {
            tag_name: "title",
            end_tag_match: 0,
        },
        "textarea" => HtmlState::Rcdata {
            tag_name: "textarea",
            end_tag_match: 0,
        },
        "iframe" => HtmlState::RawText {
            tag_name: "iframe",
            end_tag_match: 0,
        },
        "noembed" => HtmlState::RawText {
            tag_name: "noembed",
            end_tag_match: 0,
        },
        "noframes" => HtmlState::RawText {
            tag_name: "noframes",
            end_tag_match: 0,
        },
        "xmp" => HtmlState::RawText {
            tag_name: "xmp",
            end_tag_match: 0,
        },
        "noscript" => HtmlState::RawText {
            tag_name: "noscript",
            end_tag_match: 0,
        },
        _ => HtmlState::Text,
    }
}

fn classify_attr(attr_name: &str) -> AttrKind {
    if attr_name == "srcdoc" {
        AttrKind::Srcdoc
    } else if matches!(
        attr_name,
        "href"
            | "src"
            | "action"
            | "formaction"
            | "poster"
            | "cite"
            | "data"
            | "background"
            | "xlink:href"
    ) {
        AttrKind::Url
    } else if attr_name == "style" {
        AttrKind::Style
    } else if attr_name.starts_with("on") && attr_name.len() > 2 {
        AttrKind::EventHandler
    } else {
        AttrKind::Normal
    }
}

fn starts_with_open_tag_ci(rest: &str, tag: &str) -> Option<usize> {
    let prefix_len = 1 + tag.len();
    if rest.len() <= prefix_len || !rest.starts_with('<') {
        return None;
    }
    let candidate = rest.get(1..prefix_len)?;
    if !candidate.eq_ignore_ascii_case(tag) {
        return None;
    }
    let next = rest[prefix_len..].chars().next()?;
    (next == '>' || next == '/' || next.is_ascii_whitespace()).then_some(prefix_len)
}

fn starts_with_end_tag_ci(rest: &str, tag: &str) -> Option<usize> {
    let prefix_len = 2 + tag.len();
    if rest.len() <= prefix_len {
        return None;
    }
    if !rest.starts_with("</") {
        return None;
    }
    let candidate = rest.get(2..prefix_len)?;
    if !candidate.eq_ignore_ascii_case(tag) {
        return None;
    }
    let next = rest[prefix_len..].chars().next()?;
    (next == '>' || next == '/' || next.is_ascii_whitespace()).then_some(prefix_len)
}

fn decode_numeric_char_ref(bytes: &[u8]) -> Option<(char, usize)> {
    let (mut i, radix) = if matches!(bytes.get(2), Some(b'x' | b'X')) {
        (3, 16_u32)
    } else {
        (2, 10_u32)
    };
    let mut has_digit = false;
    let mut cp = 0_u32;
    while i < bytes.len() {
        let digit = match bytes[i] {
            b'0'..=b'9' => u32::from(bytes[i] - b'0'),
            b'a'..=b'f' if radix == 16 => u32::from(bytes[i] - b'a' + 10),
            b'A'..=b'F' if radix == 16 => u32::from(bytes[i] - b'A' + 10),
            _ => break,
        };
        has_digit = true;
        // Saturate above Unicode max (0x10_FFFF) so arbitrary leading zeros or huge
        // digit counts never overflow or wrap.
        cp = cp
            .saturating_mul(radix)
            .saturating_add(digit)
            .min(0x11_0000);
        i += 1;
    }
    if !has_digit {
        return None;
    }
    if bytes.get(i) == Some(&b';') {
        i += 1;
    }
    let ch = if cp == 0 {
        '\u{FFFD}'
    } else {
        char::from_u32(cp).unwrap_or('\u{FFFD}')
    };
    Some((ch, i))
}

fn decode_next_html_entity(rest: &str) -> Option<(char, usize)> {
    let bytes = rest.as_bytes();
    if bytes.first() != Some(&b'&') {
        return None;
    }
    if bytes.get(1) == Some(&b'#') {
        return decode_numeric_char_ref(bytes);
    }

    // Scan ASCII alphanumeric bytes only, so `1..i` is guaranteed to lie on UTF-8 boundaries.
    let mut i = 1;
    while i < bytes.len() && bytes[i].is_ascii_alphanumeric() {
        i += 1;
    }
    if i == 1 {
        return None;
    }
    let name = &rest[1..i];
    if bytes.get(i) == Some(&b';') {
        let ch = match name {
            "quot" | "QUOT" => '"',
            "apos" => '\'',
            "grave" | "DiacriticalGrave" => '`',
            "sol" => '/',
            "bsol" => '\\',
            "colon" => ':',
            "amp" | "AMP" => '&',
            "lt" | "LT" => '<',
            "gt" | "GT" => '>',
            "Tab" | "tab" => '\t',
            "NewLine" | "newline" => '\n',
            "lbrace" | "lcub" => '{',
            "rbrace" | "rcub" => '}',
            "lpar" => '(',
            "rpar" => ')',
            "lsqb" | "lbrack" => '[',
            "rsqb" | "rbrack" => ']',
            "dollar" => '$',
            "ast" | "midast" => '*',
            "semi" => ';',
            "comma" => ',',
            "equals" => '=',
            "plus" => '+',
            "excl" => '!',
            "quest" => '?',
            "period" => '.',
            "num" => '#',
            "percnt" => '%',
            "verbar" | "vert" | "VerticalLine" => '|',
            "hat" => '^',
            "commat" => '@',
            "lowbar" => '_',
            _ => return None,
        };
        return Some((ch, i + 1));
    }

    // Legacy HTML5 semicolonless entities in attribute values (never decoded before `=`
    // or ASCII alphanumerics, already excluded by the loop above).
    if bytes.get(i) != Some(&b'=') {
        let ch = match name {
            "quot" | "QUOT" => '"',
            "amp" | "AMP" => '&',
            "lt" | "LT" => '<',
            "gt" | "GT" => '>',
            _ => return None,
        };
        return Some((ch, i));
    }

    None
}

fn parse_var_ident(input: &str) -> Option<&str> {
    let after_dollar = input.strip_prefix('$')?;
    let len = after_dollar
        .char_indices()
        .find(|&(_, ch)| !(ch.is_alphanumeric() || ch == '_'))
        .map_or(after_dollar.len(), |(idx, _)| idx);
    if len == 0 {
        return None;
    }
    Some(&after_dollar[len..])
}

fn is_variable_assignment(body: &str) -> bool {
    let Some(mut rest) = parse_var_ident(body) else {
        return false;
    };
    rest = rest.trim_start_matches(is_template_space);
    if let Some(after_comma) = rest.strip_prefix(',') {
        let after_comma = after_comma.trim_start_matches(is_template_space);
        let Some(after_second) = parse_var_ident(after_comma) else {
            return false;
        };
        rest = after_second.trim_start_matches(is_template_space);
    }
    if rest.starts_with(":=") {
        return true;
    }
    rest.starts_with('=') && !rest.starts_with("==")
}

fn should_wrap_auto_escape(body: &str) -> bool {
    if body.is_empty()
        || matches!(
            body,
            "else"
                | "end"
                | "break"
                | "continue"
                | "if"
                | "range"
                | "with"
                | "define"
                | "template"
                | "block"
        )
        || body.starts_with("/*")
    {
        return false;
    }

    for kw in ["if", "else", "range", "with", "define", "template", "block"] {
        if control_expression(body, kw).is_some() {
            return false;
        }
    }

    !is_variable_assignment(body)
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
            r#"{{ "a }} b" | __vael_auto_escape }}|{{ `c }} d` | __vael_auto_escape }}"#
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

    #[test]
    fn should_wrap_auto_escape_exempts_keywords_and_assignments_only() {
        use super::should_wrap_auto_escape;

        for exempt in [
            "",
            "else",
            "else if .a",
            "else with .b",
            "break",
            "continue",
            "end",
            "if .a",
            "range .items",
            "with .user",
            "define \"x\"",
            "template \"x\" .",
            "block \"x\" .",
            "/* comment */",
            "$x := 1",
            "$k, $v := .items",
            "$x = \"new\"",
            "$k, $v = .pair",
        ] {
            assert!(
                !should_wrap_auto_escape(exempt),
                "expected {exempt:?} to be exempt from auto-escape wrapping"
            );
        }

        for wrapped in [
            ".",
            ".x",
            "$x",
            "$x | upper",
            "$x | default \"a=b\"",
            "$x | default \"a:=b\"",
            "$x == 1",
            "eq $x 1",
        ] {
            assert!(
                should_wrap_auto_escape(wrapped),
                "expected {wrapped:?} to be wrapped with auto-escape"
            );
        }

        // Verify full rewrite_html_escape output for control keywords, assignments, and comparisons.
        let src = "{{if .a}}x{{else if .b}}y{{else with .c}}z{{end}}{{range $k, $v := .m}}{{if .skip}}{{continue}}{{end}}{{break}}{{end}}{{$x := 1}}{{$x = 2}}{{$x == 1}}";
        assert_eq!(
            rewrite_html_escape(src).unwrap(),
            "{{if .a}}x{{else if .b}}y{{else with .c}}z{{end}}{{range $k, $v := .m}}{{if .skip}}{{continue}}{{end}}{{break}}{{end}}{{$x := 1}}{{$x = 2}}{{ $x == 1 | __vael_auto_escape }}"
        );
    }
}
