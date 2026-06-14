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

use std::{collections::VecDeque, marker::PhantomData, sync::Arc};

use camino::Utf8PathBuf;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use tree_sitter::{Node, Range, Tree};
use crate::{kinds::{NodeKind, field_kind as Field_Kind, node_kind as Node_Kind}, salsa_db::FileText};

/// Implement rowan-like ast structure using tree-sitter and string views
/// should be able to generate a fully typed node/tree
/// should be able to generate a minimal(name, type, location) node/tree for symbol resolution
/// given a range or staring node from tree-sitter
/// basically it needs to be able to lower targeted ranges/nodes on demand
#[derive(Debug, Clone)]
pub(crate) struct Ast {
    tree: Tree,
    source: Arc<str>,
}

impl PartialEq for Ast {
    fn eq(&self, other: &Self) -> bool {
        self.source == other.source
    }
}

impl Eq for Ast {}

impl Ast {
    pub fn new(tree: Tree, source: Arc<str>) -> Self {
        Self { tree, source }
    }
    
    pub fn tree(&self) -> &Tree {
        &self.tree
    }
    
    pub fn source(&self) -> &str {
        &self.source
    }

    pub fn root(&self) -> AstNode<'_> {
        AstNode {
            node: self.tree.root_node(),
            source: &self.source,
        }
    }

    pub fn node(&self, range: PtrRange) -> Option<AstNode<'_>> {
        self.tree
            .root_node()
            .descendant_for_byte_range(range.start, range.end)
            .map(|node| AstNode { node, source: &self.source })
    }

}


/// Anything that can move to and from an AstNode
pub(crate) trait ToAstNode<'tree>: Sized {
    fn ast_node(self) -> AstNode<'tree>;
    fn ast_node_ref(&self) -> &AstNode<'tree>;
    fn cast(n: AstNode<'tree>) -> Option<Self>;
}


/// for comparism AstNode is like a RedNode(SyntaxNode) in rowan
/// we can cast from this to typed nodes(contracts, fns, struct etc.) and back
/// One major diff is that unike green nodes, Ast nodes dont own their data/syntax-string, they all use a shared view
pub struct AstNode<'a> {
    node: Node<'a>,
    source: &'a str,
}

impl PartialEq for AstNode<'_> {
    fn eq(&self, other: &Self) -> bool {
        self.text() == other.text()
    }
}

impl Eq for AstNode<'_> {}

impl<'a> AstNode<'a> {
    pub fn new(node: Node<'a>, source: &'a str) -> Self {
        Self { node, source }
    }

    pub fn node(&self) -> Node<'a> {
        self.node
    }

    pub fn text(&self) -> &'a str {
        &self.source[self.node.byte_range()]
    }

    pub fn child_node(&self, range: PtrRange) -> Option<AstNode<'a>> {
        self.node.descendant_for_byte_range(range.start, range.end)
            .map(|node| AstNode { node, source: &self.source })
    }

    pub fn is_root(&self) -> bool {
        self.node.kind_id() == Node_Kind::SOURCE_FILE
    }

    ///The top level of the node
    /// If node is root then, top level symbols in scope
    pub fn children(&self, cursor: &'a mut tree_sitter::TreeCursor<'a>) -> impl Iterator<Item = AstNode<'a>> + 'a {
        assert!(self.node == cursor.node());
        self.node.named_children(cursor)
            .map(|node: Node<'a>| AstNode { node, source: self.source })
    }

}




/// file-local node index wrapper
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ErasedFileAstId(u32); 
 

/// file-local index with type info
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct FileAstId<N> {
    raw: ErasedFileAstId,
    _ty: PhantomData<fn() -> N>,
}

impl<N> FileAstId<N> {
    pub fn erase(self) -> ErasedFileAstId { self.raw }
    pub fn from_erased(raw: ErasedFileAstId) -> Self {
        Self { raw, _ty: PhantomData }
    }
}


/// Unique global index for node
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct AstId<N> {
    pub file_id: FileText,//salsa file id
    pub local_id: FileAstId<N>,
}




pub(crate) fn is_supported_node(node: &Node<'_>) -> bool {
    use Node_Kind::*;
    matches!(// I dont know if i want the identifiers yet, everything basically contains one, i guess lookup will determine
        NodeKind::from(node.kind_id()),
        CONTRACT_DEFINITION
            | INTERFACE_DEFINITION
            | LIBRARY_DEFINITION
            | STRUCT_DEFINITION
            | ENUM_DEFINITION
            | FUNCTION_DEFINITION
            | EVENT_DEFINITION
            | ERROR_DEFINITION
            | MODIFIER_DEFINITION
            | IMPORT_DIRECTIVE
            | CONTRACT_BODY
            | FUNCTION_BODY
            | STATE_VAR_DECLARATION
            | STATEMENT //dont really want a statement node but i want the nested statement type ie variable declaration statement
            | VAR_DECLARATION_STATEMENT
            | BLOCK_STATEMENT
            | FOR_STATEMENT
            | IF_STATEMENT
    )
}


#[derive(Debug, PartialEq, Eq)]
pub(crate) struct AstIdMap {
    ptr: Vec<NodePtr>,
    ptr_to_id: FxHashMap<NodePtr, usize>,
}

impl AstIdMap {
    pub fn new(root: &AstNode) -> Self {
        assert!(root.is_root());

        // Very shallow/simplified id allocation algo for now: does BFS
        let mut ptr = Vec::new();
        let mut ptr_to_id = FxHashMap::default();
        let mut queue = VecDeque::new();

        queue.push_back(root.node());

        while let Some(node) = queue.pop_front() {
            //Dont collect statement but collect nested statement nodes
            if node.kind_id() != Node_Kind::STATEMENT {
                let node_ptr = NodePtr::from_raw_node(node);
                let id = ptr.len();
                ptr.push(node_ptr);
                ptr_to_id.insert(node_ptr, id);
            }

            for child in node.named_children(&mut node.walk()).filter(is_supported_node) {
                queue.push_back(child);
            }
        }

        Self { ptr, ptr_to_id }
    }

    pub fn id_erased(&self, ptr: NodePtr) -> Option<ErasedFileAstId> {
        self.ptr_to_id.get(&ptr).map(|&id| ErasedFileAstId(id as u32))
    }
 
    pub fn id_of<'tree, N: ToAstNode<'tree>>(&self, n: &N) -> Option<FileAstId<N>> {
        let ptr = NodePtr::from_ast_node(n.ast_node_ref());
        self.id_erased(ptr).map(FileAstId::from_erased)
    }
 
    pub fn get<'tree, N: ToAstNode<'tree>>(&self, root: &AstNode<'tree>, id: FileAstId<N>) -> Option<N> {
        let ptr = self.ptr[id.erase().0 as usize];
        //Assert root contains node?
        let node = ptr.to_node(root)?;
        N::cast(node)
    }
}




///                NODEPTR
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PtrRange {
    start: usize,//TODO: use u32
    end: usize
}

impl PtrRange {
    #[inline]
    pub(crate) fn from_range(range: Range) -> Self {
        Self { start: range.start_byte, end: range.end_byte }
    }

    #[inline]
    pub(crate) fn contains(&self, other: &PtrRange) -> bool {
        self.start <= other.start && self.end >= other.end
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct NodePtr {
    kind: NodeKind,
    range: PtrRange,
}

impl NodePtr {
    #[inline]
    pub(crate) fn from_ast_node(node: &AstNode) -> Self {
        Self { kind: node.node().kind_id().into(), range: PtrRange::from_range(node.node().range()) }
    }

    #[inline]
    pub(crate) fn from_raw_node(node: Node) -> Self {
        Self { kind: node.kind_id().into(), range: PtrRange::from_range(node.range()) }
    }

    pub(crate) fn to_node<'tree>(&self, root: &AstNode<'tree>) -> Option<AstNode<'tree>> {
        root.child_node(self.range).filter(|a| a.node().kind_id() == self.kind)
    }
}



///                AST KINDS
pub(crate) struct Contract<'a> {
    raw: AstNode<'a>,
}

impl<'a> ToAstNode<'a> for Contract<'a> {
    fn ast_node(self) -> AstNode<'a> {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode<'a> {
        &self.raw
    }
    
    fn cast(node: AstNode<'a>) -> Option<Self> {
        if node.node().kind_id() == Node_Kind::CONTRACT_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }
}

pub(crate) struct Interface<'a> {
    raw: AstNode<'a>,
}

impl<'a> ToAstNode<'a> for Interface<'a> {
    fn ast_node(self) -> AstNode<'a> {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode<'a> {
        &self.raw
    }

    fn cast(node: AstNode<'a>) -> Option<Self> {
        if node.node().kind_id() == Node_Kind::INTERFACE_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }
}

pub(crate) struct Library<'a> {
    raw: AstNode<'a>,
}

impl<'a> ToAstNode<'a> for Library<'a> {
    fn ast_node(self) -> AstNode<'a> {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode<'a> {
        &self.raw
    }

    fn cast(node: AstNode<'a>) -> Option<Self> {
        if node.node().kind_id() == Node_Kind::LIBRARY_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }
}

pub(crate) struct Struct<'a> {
    raw: AstNode<'a>,
}

impl<'a> ToAstNode<'a> for Struct<'a> {
    fn ast_node(self) -> AstNode<'a> {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode<'a> {
        &self.raw
    }

    fn cast(node: AstNode<'a>) -> Option<Self> {
        if node.node().kind_id() == Node_Kind::STRUCT_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }
}

pub(crate) struct Enum<'a> {
    raw: AstNode<'a>,
}

impl<'a> ToAstNode<'a> for Enum<'a> {
    fn ast_node(self) -> AstNode<'a> {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode<'a> {
        &self.raw
    }

    fn cast(node: AstNode<'a>) -> Option<Self> {
        if node.node().kind_id() == Node_Kind::ENUM_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }
}

pub(crate) struct Function<'a> {
    raw: AstNode<'a>,
}

impl<'a> ToAstNode<'a> for Function<'a> {
    fn ast_node(self) -> AstNode<'a> {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode<'a> {
        &self.raw
    }
    
    fn cast(node: AstNode<'a>) -> Option<Self> {
        if node.node().kind_id() == Node_Kind::FUNCTION_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }
}

pub(crate) struct Event<'a> {
    raw: AstNode<'a>,
}

impl<'a> ToAstNode<'a> for Event<'a> {
    fn ast_node(self) -> AstNode<'a> {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode<'a> {
        &self.raw
    }
    
    fn cast(node: AstNode<'a>) -> Option<Self> {
        if node.node().kind_id() == Node_Kind::EVENT_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }
}

pub(crate) struct Error<'a> {
    raw: AstNode<'a>,
}

impl<'a> ToAstNode<'a> for Error<'a> {
    fn ast_node(self) -> AstNode<'a> {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode<'a> {
        &self.raw
    }

    fn cast(node: AstNode<'a>) -> Option<Self> {
        if node.node().kind_id() == Node_Kind::ERROR_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }
}

pub(crate) struct Modifier<'a> {
    raw: AstNode<'a>,
}


impl<'a> ToAstNode<'a> for Modifier<'a> {
    fn ast_node(self) -> AstNode<'a> {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode<'a> {
        &self.raw
    }

    fn cast(node: AstNode<'a>) -> Option<Self> {
        if node.node().kind_id() == Node_Kind::MODIFIER_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }
}

pub(crate) struct Import<'a> {
    raw: AstNode<'a>,
}

impl<'a> ToAstNode<'a> for Import<'a> {
    fn ast_node(self) -> AstNode<'a> {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode<'a> {
        &self.raw
    }

    fn cast(node: AstNode<'a>) -> Option<Self> {
        if node.node().kind_id() == Node_Kind::IMPORT_DIRECTIVE {
            Some(Self { raw: node })
        } else {
            None
        }
    }
}


pub(crate) enum Item<'a> {
    Contract(Contract<'a>),
    Interface(Interface<'a>),
    Library(Library<'a>),
    Struct(Struct<'a>),
    Enum(Enum<'a>),
    Function(Function<'a>),
    Event(Event<'a>),
    Error(Error<'a>),
    Modifier(Modifier<'a>),
    Import(Import<'a>),
}

impl<'a> ToAstNode<'a> for Item<'a> {
    fn ast_node(self) -> AstNode<'a> {
        match self {
            Item::Contract(c) => c.ast_node(),
            Item::Interface(i) => i.ast_node(),
            Item::Library(l) => l.ast_node(),
            Item::Struct(s) => s.ast_node(),
            Item::Enum(e) => e.ast_node(),
            Item::Function(f) => f.ast_node(),
            Item::Event(e) => e.ast_node(),
            Item::Error(e) => e.ast_node(),
            Item::Modifier(m) => m.ast_node(),
            Item::Import(i) => i.ast_node(),
        }
    }

    fn ast_node_ref(&self) -> &AstNode<'a> {
        match self {
            Item::Contract(c) => c.ast_node_ref(),
            Item::Interface(i) => i.ast_node_ref(),
            Item::Library(l) => l.ast_node_ref(),
            Item::Struct(s) => s.ast_node_ref(),
            Item::Enum(e) => e.ast_node_ref(),
            Item::Function(f) => f.ast_node_ref(),
            Item::Event(e) => e.ast_node_ref(),
            Item::Error(e) => e.ast_node_ref(),
            Item::Modifier(m) => m.ast_node_ref(),
            Item::Import(i) => i.ast_node_ref(),
        }
    }

    fn cast(node: AstNode<'a>) -> Option<Self> {
        match NodeKind::from(node.node().kind_id()) {
            Node_Kind::CONTRACT_DEFINITION => Some(Self::Contract(Contract::cast(node).unwrap())),
            Node_Kind::INTERFACE_DEFINITION => Some(Self::Interface(Interface::cast(node).unwrap())),
            Node_Kind::LIBRARY_DEFINITION => Some(Self::Library(Library::cast(node).unwrap())),
            Node_Kind::STRUCT_DEFINITION => Some(Self::Struct(Struct::cast(node).unwrap())),
            Node_Kind::ENUM_DEFINITION => Some(Self::Enum(Enum::cast(node).unwrap())),
            Node_Kind::FUNCTION_DEFINITION => Some(Self::Function(Function::cast(node).unwrap())),
            Node_Kind::EVENT_DEFINITION => Some(Self::Event(Event::cast(node).unwrap())),
            Node_Kind::ERROR_DEFINITION => Some(Self::Error(Error::cast(node).unwrap())),
            Node_Kind::MODIFIER_DEFINITION => Some(Self::Modifier(Modifier::cast(node).unwrap())),
            Node_Kind::IMPORT_DIRECTIVE => Some(Self::Import(Import::cast(node).unwrap())),
            _ => None,
        }
    }
}









