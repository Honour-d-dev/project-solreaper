
use lsp_server::{Request, Response};
use lsp_types::{Position, TextDocumentIdentifier};
use tree_sitter::Node;

use crate::ast::{NodeRange};
use crate::salsa::{RootDatabase, SalsaDb};
use crate::utilities::{log_info, print_named_tree, to_utf8path};

/// Custom LSP request: `solidity/viewAst`
///
/// Params: `{ textDocument, start, end }` — two LSP positions delimiting
/// the user's selection.
///
/// Returns: `{ content: string }` — the raw tree-sitter named AST tree
/// for the largest named node(s) fully enclosed by the selection,
/// produced by `print_named_tree`.
pub fn view_ast(db: &SalsaDb, request: Request) -> anyhow::Result<Response> {
    let params: ViewAstParams = serde_json::from_value(request.params)?;
    let path = to_utf8path(&params.text_document.uri)?;
    log_info(format!("viewAst request for {path}"));

    let (file, start_off) = db.convert(&path, params.start);
    let (_, end_off) = db.convert(&path, params.end);
    let selection = NodeRange { start: start_off, end: end_off };

    let root = db.root(file);
    let ast = db.ast(file);
    let source = ast.source();

    // Find the largest named node(s) fully enclosed by the selection.
    // tree-sitter only provides `descendant_for_byte_range` (smallest
    // node *enclosing* the range), so we do a manual top-down walk:

    let result = find_enclosed_node(root.node(), selection)
        .map(|n| serde_json::json!({ "content": print_named_tree(&mut n.walk(), source, 0) }));

    
    Ok(Response {
        id: request.id,
        result,
        error: None,
    })
}


// ── Node discovery ───────────────────────────────────────────────────

/// Find the largest named nodes whose range is fully contained within
/// `selection`. Walks top-down; once a node is fully contained, its
/// children are not visited (they'd be smaller and redundant).


fn find_enclosed_node(node: Node, selection: NodeRange) -> Option<Node> {
    let node_start = node.start_byte() as u32;
    let node_end = node.end_byte() as u32;

    // No overlap at all → skip.
    if node_end <= selection.start || node_start >= selection.end {
        return None;
    }

    // Fully enclosed → this is a root, don't recurse.
    if node_start >= selection.start && node_end <= selection.end {
        return Some(node);
    }

    // Partially overlapping → recurse into children.
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        if let Some(node) = find_enclosed_node(child, selection) {
            return Some(node);
        }
    }
    None
}


// ── Request type ─────────────────────────────────────────────────────

pub struct ViewAstRequest;

impl lsp_types::request::Request for ViewAstRequest {
    type Params = ViewAstParams;
    type Result = ViewAstResult;
    const METHOD: &'static str = "solidity/viewAst";
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ViewAstResult {
    pub content: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct ViewAstParams {
    #[serde(rename = "textDocument")]
    pub text_document: TextDocumentIdentifier,
    pub start: Position,
    pub end: Position,
}


