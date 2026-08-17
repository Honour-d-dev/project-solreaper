#![allow(unused)]
use anyhow::Context;
use camino::{Utf8Path, Utf8PathBuf};
use lsp_types::Url;
use ropey::Rope;
use serde::Serialize;
use lsp_types::{Position};


pub(crate) fn to_utf8path(uri: &Url) -> anyhow::Result<Utf8PathBuf> {
    uri.to_file_path()
        .ok()
        .and_then(|path| Utf8PathBuf::from_path_buf(path).ok())
        .context("failed to convert uri to path")
}

///Resolves to a new path relative to a base path
pub(crate) fn resolve_path(base: &Utf8PathBuf, rel_path: impl AsRef<Utf8Path>) -> anyhow::Result<Utf8PathBuf> {
    let resolved_path = base.parent().context("Error: can't resolve path, base path has no parent")?
    .join(rel_path)
    .canonicalize_utf8()
    .context("failed to canonicalize resolved path to standard utf8")?;
    Ok(resolved_path)
}

/// Resolves a Solidity import string, checking remappings first, then falling back to relative resolution.
pub(crate) fn resolve_import(
    base: &Utf8PathBuf,
    import_str: &str,
    project_root: &Utf8Path,
    remappings: &[Remapping],
) -> anyhow::Result<Utf8PathBuf> {
    // Relative imports
    if import_str.starts_with("./") || import_str.starts_with("../") {
        return resolve_path(base, import_str);
    }

    // Try remappings (longest prefix first to avoid false matches)
    let mut best_match: Option<(&str, &Utf8Path)> = None;
    for remapping in remappings {
        if import_str.starts_with(&remapping.prefix) {
            if best_match.map_or(true, |(best_prefix, _)| remapping.prefix.len() > best_prefix.len()) {
                best_match = Some((&remapping.prefix, remapping.target.as_path()));
            }
        }
    }

    if let Some((prefix, target)) = best_match {
        let suffix = &import_str[prefix.len()..];
        let resolved = project_root
            .join(target)
            .join(suffix)
            .canonicalize_utf8()
            .with_context(|| format!("failed to resolve remapped import `{import_str}` via {prefix}={target}"))?;
        return Ok(resolved);
    }

    // Fallback: try relative resolution against the base file's parent
    resolve_path(base, import_str)
}

pub(crate) fn get_rope_idx(file: &Rope, pos: Position) -> usize {
    //lsp characters are utf16 code units while rope functions operate on "chars" (unicode code points)
    // so non-BMP characters (like emojis) will have length 2 in utf16 but length 1 in rope
    let mut utf16_count = 0usize;
    let mut char_count = 0usize;

    for ch in file.line(pos.line as usize).chars() {
        if ch == '\n' || utf16_count >= pos.character as usize {
            break;
        }
        utf16_count += ch.len_utf16(); // key conversion step
        char_count += 1;
    }
    
    file.line_to_char(pos.line as usize) + char_count
}


#[inline]
pub(crate) fn to_rope_idx(file: &Rope, pos: Position) -> usize {
    file.line_to_char(pos.line as usize) + file.line(pos.line as usize).utf16_cu_to_char(pos.character as usize)
}

#[inline]
pub(crate) fn to_rope_byte_idx(file: &Rope, pos: Position) -> usize {
    let l = file.line(pos.line as usize);
    let c = l.utf16_cu_to_char(pos.character as usize);
    file.line_to_byte(pos.line as usize) + l.char_to_byte(c)
}

#[inline]
pub(crate) fn position_to_point(r: &Rope, position: Position) -> Point {
    Point { row: position.line as usize, column: r.line(position.line as usize).utf16_cu_to_char(position.character as usize) }
}

#[inline]
pub(crate) fn char_to_point(r: &Rope, char_idx: usize) -> Point {//assumes shar index is in unicode code points not utf16 code units
    let ctl = r.char_to_line(char_idx);
    Point { row: ctl, column: char_idx - r.line_to_char(ctl) }
}

#[inline]
pub(crate) fn byte_to_point(r: &Rope, byte_idx: usize) -> Point {
    let ctl = r.byte_to_line(byte_idx);
    Point { row: ctl, column: byte_idx - r.line_to_byte(ctl) }
}



#[allow(unused)]
pub(crate) fn print_tree_old(node: tree_sitter::Node, indentation: usize, source_code: &str) -> String {
    let indent = "  ".repeat(indentation);
    let kind = node.kind();
    let text = &source_code[node.byte_range()];
    //let snippet = text.lines().next().unwrap_or("").trim();

    //natspec comment is ignored is this from the solidity tree-sitter parser? or where? Its from above line lol
    let mut result = format!("{}  {}: \"{}\"\n", indent, kind, text);

    for child in node.children(&mut node.walk()) {
        result.push_str(&print_tree_old(child, indentation + 1, source_code));
    }

    result
}


#[allow(unused)]
pub(crate) fn print_tree(cursor: &mut tree_sitter::TreeCursor, source_code: &str, indentation: usize) -> String {
    let indent = "  ".repeat(indentation);
    let kind = cursor.node().kind();
    let text = &source_code[cursor.node().byte_range()].replace('\n', &format!("\n {indent} "));

    let mut result = format!("\n{indent}{kind}: \"{text}\"");//add field name and id if present so wee see what its about

    iterate_children!(cursor, {
        result.push_str(&print_tree(cursor, source_code, indentation + 2));
    });
    return result;
}

#[allow(unused)]
pub(crate) fn print_named_tree(cursor: &mut tree_sitter::TreeCursor, source_code: &str, indentation: usize) -> String {
    if !cursor.node().is_named() { return "".to_string(); }
    let indent = "  ".repeat(indentation);//todo for only named nodes
    let kind = cursor.node().kind();


    let text = &source_code[cursor.node().byte_range()];
    // dont  print bodies, blocks and statements
    let mut result = if text.len() > 120 ||kind == "statement" || text.contains("{") {
        format!("\n{indent}{kind}:")
    } else {
        text.replace('\n', &format!("\n {indent} "));
        format!("\n{indent}{kind}: \"{text}\"")//add field name and id if present so wee see what its about
    };

    iterate_children!(cursor, {
        result.push_str(&print_named_tree(cursor, source_code, indentation + 2));
    });
    return result;
}

#[derive(Serialize)]
struct AstJsonNode {
    kind: String,
    text: Vec<String>,
    children: Vec<AstJsonNode>,
}

fn tree_to_json(cursor: &mut tree_sitter::TreeCursor<'_>, source_code: &str) -> AstJsonNode {
    let node = cursor.node();
    let text = source_code[node.byte_range()]
        .split('\n').map(str::to_owned).collect::<Vec<String>>();

    let mut children = Vec::new();
    iterate_children!(cursor, {
        children.push(tree_to_json(cursor, source_code));
    });

    AstJsonNode {
        kind: node.kind().to_string(),
        text,
        children,
    }
}

pub(crate) fn print_tree_json(node: tree_sitter::Node<'_>, source_code: &str) -> anyhow::Result<String> {
    serde_json::to_string_pretty(&tree_to_json(&mut node.walk(), source_code))
        .context("failed to serialize tree as JSON")
}

pub(crate) fn log_info(message: impl Into<String>) {
    tracing::info!("{}", message.into());
}

/**
 * Iterates over the children of the current node using a cursor
 * returns cursor back to parent node afterwards
 */
macro_rules! iterate_children {
    ($cursor:expr, $body:block) => {
        if $cursor.goto_first_child() {
            loop {
                $body
                if !$cursor.goto_next_sibling() {
                    break;
                }
            }
            $cursor.goto_parent();
        }
    };

    ($cursor:expr, $prefix:expr, $body:block) => {
        if $cursor.goto_first_child() {
            $prefix;
            loop {
                $body
                if !$cursor.goto_next_sibling() {
                    break;
                }
            }
            $cursor.goto_parent();
        }
    };
}

pub(crate) use iterate_children;
use tree_sitter::Point;

use crate::workspace::Remapping;


#[cfg(test)]
mod tests;
