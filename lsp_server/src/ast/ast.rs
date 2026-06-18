#![allow(unused)]
/// My current approach to tackle the inheritance problem:
/// Implement a per-project inheritance graph
/// graph implements forwards and backwards dependency, for easy mods when contracts change
/// The graph is incrementally updated as we discover(fully lower on did-open) files
/// ideally this graph should be in salsa so we dont have to eagerly apply changes
/// from the graph we can linearize  and build full symbol/scope per contact containing parent symbols/scopes
/// but how  do i marry the scope system  with the inheritance graph?
/// the scope system helps for local symbol resolution
/// the inheritance graph helps attach inherited symbols to the scope
/// 
/// with selective parsing we can quicky get all symbols in a within the scope range by identifying the scope node
/// for efficiency the symbols wont be fully typed out nodes, just the minimal information needed for symbol resolution i.e name, type, location
/// 
/// Resolution order:
/// - lexical locals
/// - current contract members
/// - linearized base contracts
/// - file/import globals

use std::{collections::VecDeque, marker::PhantomData, rc::Rc, sync::Arc};

use camino::Utf8PathBuf;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use tree_sitter::{Node, Range, Tree, ffi::TSNode};
use crate::
    ast::{
        ast_id::PtrRange, 
        kinds::NodeKind
    };
    

/// Implement rowan-like ast structure using tree-sitter and string views
/// should be able to generate a fully typed node/tree
/// should be able to generate a minimal(name, type, location) node/tree for symbol resolution
/// given a range or staring node from tree-sitter
/// basically it needs to be able to lower targeted ranges/nodes on demand
/// 

type ByteRange = std::ops::Range<usize>;

#[derive(Debug, Clone)]
pub struct AstInner {
    tree: Tree,
    source: Arc<str>,
}

impl PartialEq for AstInner {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for AstInner {}


#[derive(Debug, Clone)]
pub struct Ast {
    inner: AstInner
}

impl PartialEq for Ast {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for Ast {}

impl Ast {
    pub fn new(tree: Tree, source: Arc<str>) -> Self {
        Self { inner: AstInner { tree, source } }
    }
    
    pub fn tree(&self) -> &Tree {
        &self.inner.tree
    }
    
    pub fn source(&self) -> &str {
        &self.inner.source
    }

    pub fn root(&self) -> AstNode {
        AstNode {
            node: self.tree().root_node().into_raw(),
            inner: self.inner.clone(),
        }
    }

     pub fn make_ast(&self, node: Node<'_>) -> AstNode {
        AstNode::new(node.into_raw(), self.inner.clone())
    }

    pub fn node(&self, range: PtrRange) -> Option<AstNode> {
        self.tree()
            .root_node()
            .descendant_for_byte_range(range.start, range.end)
            .map(|node| AstNode::new(node.into_raw(), self.inner.clone()))
    }

}


/// for comparism AstNode is like a RedNode(SyntaxNode) in rowan
/// we can cast from this to typed nodes(contracts, fns, struct etc.) and back
/// One major diff is that unike green nodes, tree-sitter nodes dont own their data/syntax-string, so we have to use a shared view



/// lifetime colouring work around for AstNode
/// Using the raw TsNode instead of the typed Node<'a> to avoid lifetime coloring
/// but we incure an extra 8 byte overhead
/// We keep a copy of the tree and source pointer so tree is never dropped as long as a node exists
#[derive(Debug, Clone)]
pub struct AstNode{
    node: TSNode,
    inner: AstInner,
}


impl PartialEq for AstNode {
    fn eq(&self, other: &Self) -> bool {
        self.inner == other.inner
    }
}

impl Eq for AstNode {}

impl AstNode {
    pub fn new(node: TSNode, inner: AstInner) -> Self {
        Self { node, inner }
    }

    pub fn node(&self) -> Node<'_> {
        unsafe { Node::from_raw(self.node) }
    }

    pub fn raw_node(&self) -> TSNode {
        self.node
    }

    pub fn text(&self) -> &str {
        &self.inner.source[self.node().byte_range()]
    }

    pub fn text_by_range(&self, range: ByteRange) -> &str {
        &self.inner.source[range]
    }

    pub fn make_ast(&self, node: Node<'_>) -> AstNode {
        AstNode::new(node.into_raw(), self.inner.clone())
    }

    pub fn child_node(&self, range: PtrRange) -> Option<AstNode> {
        self.node().descendant_for_byte_range(range.start, range.end)
            .map(|node| AstNode::new(node.into_raw(), self.inner.clone()))
    }

    pub fn is_root(&self) -> bool {
        self.node().kind_id() == NodeKind::SOURCE_FILE
    }

    ///The top level of the node
    /// If node is root then, top level symbols in scope
    pub fn children<'a>(&'a self, cursor: &'a mut tree_sitter::TreeCursor<'a>) -> impl Iterator<Item = AstNode> + 'a {
        assert!(self.node() == cursor.node());
        self.node().named_children(cursor)
            .map(|node| AstNode::new(node.into_raw(), self.inner.clone()))
    }

}


/// Anything that can move to and from an AstNode
pub trait ToAstNode: Sized {
    fn ast_node(self) -> AstNode;
    fn ast_node_ref(&self) -> &AstNode;
    fn cast(n: AstNode) -> Option<Self>;
    fn can_cast(n: &Node) -> bool;
}



