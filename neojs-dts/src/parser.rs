use crate::DeclaredModuleExports;

#[derive(Debug, Clone, PartialEq, Eq)]
enum Token {
    Ident(String),
    StringLit(String),
    Punct(char),
}

fn is_ident_start(byte: u8) -> bool {
    byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$'
}

fn is_ident_continue(byte: u8) -> bool {
    is_ident_start(byte) || byte.is_ascii_digit()
}

fn tokenize(input: &str) -> Vec<Token> {
    let bytes = input.as_bytes();
    let len = bytes.len();
    let mut i = 0;
    let mut tokens = Vec::new();

    while i < len {
        let b = bytes[i];

        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }

        // Line comment `// ...`
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'/' {
            i += 2;
            while i < len && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }

        // Block comment `/* ... */`
        if b == b'/' && i + 1 < len && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(len);
            continue;
        }

        // String / template literal
        if b == b'"' || b == b'\'' || b == b'`' {
            let quote = b;
            i += 1;
            let mut value = String::new();
            while i < len {
                let c = bytes[i];
                if c == b'\\' {
                    if let Some(ch) = input[i + 1..].chars().next() {
                        value.push(ch);
                        i += 1 + ch.len_utf8();
                    } else {
                        i += 1;
                    }
                    continue;
                }
                if c == quote {
                    i += 1;
                    break;
                }
                // Advance one UTF-8 scalar cleanly.
                let ch = input[i..].chars().next().unwrap_or('\0');
                value.push(ch);
                i += ch.len_utf8();
            }
            tokens.push(Token::StringLit(value));
            continue;
        }

        if is_ident_start(b) {
            let start = i;
            i += 1;
            while i < len && is_ident_continue(bytes[i]) {
                i += 1;
            }
            tokens.push(Token::Ident(input[start..i].to_owned()));
            continue;
        }

        if b.is_ascii_digit() {
            i += 1;
            while i < len && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                i += 1;
            }
            continue;
        }

        let ch = input[i..].chars().next().unwrap_or('\0');
        tokens.push(Token::Punct(ch));
        i += ch.len_utf8();
    }

    tokens
}

fn push_unique(list: &mut Vec<String>, name: String) {
    if !list.iter().any(|existing| existing == &name) {
        list.push(name);
    }
}

fn parse_export_clause(
    tokens: &[Token],
    mut idx: usize,
    module: &mut DeclaredModuleExports,
) -> usize {
    // Expects idx pointing just after `{`.
    let mut item: Vec<&str> = Vec::new();

    while idx < tokens.len() {
        match &tokens[idx] {
            Token::Punct('}') => {
                process_export_clause_item(&item, module);
                return idx + 1;
            }
            Token::Punct(',') => {
                process_export_clause_item(&item, module);
                item.clear();
                idx += 1;
            }
            Token::Ident(ident) => {
                item.push(ident.as_str());
                idx += 1;
            }
            _ => {
                idx += 1;
            }
        }
    }
    idx
}

fn process_export_clause_item(item: &[&str], module: &mut DeclaredModuleExports) {
    if item.is_empty() {
        return;
    }
    // `export { type Foo }` or `export { type Foo as Bar }` is type-only.
    if item[0] == "type" && item.len() >= 2 && item[1] != "as" {
        return;
    }
    let exported = if item.len() >= 3 && item[1] == "as" {
        item[2]
    } else {
        item[0]
    };
    if exported == "default" {
        module.has_default = true;
    } else {
        push_unique(&mut module.exports, exported.to_owned());
    }
}

/// Parses every `declare module "<specifier>" { ... }` block in `dts` and
/// returns its declared runtime value exports.
///
/// Type-only exports (`export type`, `export interface`, `export const enum`,
/// and `export { type ... }`) are ignored. Overloaded function signatures are
/// deduplicated while preserving declaration order.
pub(crate) fn parse_declared_exports(dts: &str) -> Vec<DeclaredModuleExports> {
    let tokens = tokenize(dts);
    let mut modules = Vec::new();
    let mut idx = 0;

    while idx < tokens.len() {
        if matches!(&tokens[idx], Token::Ident(id) if id == "declare")
            && matches!(tokens.get(idx + 1), Some(Token::Ident(id)) if id == "module")
            && let Some(Token::StringLit(specifier)) = tokens.get(idx + 2)
            && matches!(tokens.get(idx + 3), Some(Token::Punct('{')))
        {
            let mut module = DeclaredModuleExports {
                specifier: specifier.clone(),
                exports: Vec::new(),
                has_default: false,
            };
            idx += 4;
            let mut brace_depth: usize = 1;
            let mut paren_depth: usize = 0;

            while idx < tokens.len() && brace_depth > 0 {
                match &tokens[idx] {
                    Token::Punct('{') => {
                        brace_depth += 1;
                        idx += 1;
                    }
                    Token::Punct('}') => {
                        brace_depth -= 1;
                        idx += 1;
                    }
                    Token::Punct('(') => {
                        paren_depth += 1;
                        idx += 1;
                    }
                    Token::Punct(')') => {
                        paren_depth = paren_depth.saturating_sub(1);
                        idx += 1;
                    }
                    Token::Ident(id) if brace_depth == 1 && paren_depth == 0 && id == "export" => {
                        idx += 1;
                        if matches!(tokens.get(idx), Some(Token::Ident(k)) if k == "default") {
                            module.has_default = true;
                            idx += 1;
                            continue;
                        }
                        if matches!(
                            tokens.get(idx),
                            Some(Token::Ident(k)) if k == "type" || k == "interface" || k == "namespace"
                        ) {
                            idx += 1;
                            continue;
                        }
                        if matches!(tokens.get(idx), Some(Token::Punct('{'))) {
                            idx = parse_export_clause(&tokens, idx + 1, &mut module);
                            continue;
                        }
                        // Skip optional modifiers `declare`, `async`, `abstract`.
                        while matches!(
                            tokens.get(idx),
                            Some(Token::Ident(k))
                                if k == "declare" || k == "async" || k == "abstract"
                        ) {
                            idx += 1;
                        }
                        match tokens.get(idx) {
                            Some(Token::Ident(k)) if k == "const" => {
                                if matches!(tokens.get(idx + 1), Some(Token::Ident(n)) if n == "enum")
                                {
                                    idx += 2;
                                } else if let Some(Token::Ident(name)) = tokens.get(idx + 1) {
                                    push_unique(&mut module.exports, name.clone());
                                    idx += 2;
                                } else {
                                    idx += 1;
                                }
                            }
                            Some(Token::Ident(k))
                                if k == "function"
                                    || k == "let"
                                    || k == "var"
                                    || k == "class"
                                    || k == "enum" =>
                            {
                                if let Some(Token::Ident(name)) = tokens.get(idx + 1) {
                                    push_unique(&mut module.exports, name.clone());
                                    idx += 2;
                                } else {
                                    idx += 1;
                                }
                            }
                            _ => {}
                        }
                    }
                    _ => {
                        idx += 1;
                    }
                }
            }

            modules.push(module);
            continue;
        }
        idx += 1;
    }

    modules
}
