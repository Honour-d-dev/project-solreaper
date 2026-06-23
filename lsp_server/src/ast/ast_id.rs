#![allow(unused)]
use std::{collections::VecDeque, hash::{Hash, Hasher}};
use std::marker::PhantomData;
use rustc_hash::FxHashMap;
use tree_sitter::{Node, Range};
use crate::{
    ast::{
        self,
        ast::{AstNode, ToAstNode},
        kinds::{NodeKind},
    },
    salsa_db::FileId,
};


/// file-local node index wrapper
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ErasedFileAstId { 
    id: u32,
    parent: Option<u32>,
}
 

/// Zero-cost file-local index with type info
#[derive(Debug, PartialEq, Eq)]
pub struct FileAstId<N> {
    raw: ErasedFileAstId,
    _ty: PhantomData<fn() -> N>,
}

//manual impls to avoid N: clone/copy bound on derived version
impl<N> Clone for FileAstId<N> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<N> Copy for FileAstId<N> {}

impl<N> Hash for FileAstId<N> {
    fn hash<H: Hasher>(&self, hasher: &mut H) {
        self.raw.hash(hasher);
    }
}

impl<N> FileAstId<N> {
    pub fn erase(self) -> ErasedFileAstId { self.raw }
    pub fn from_erased(raw: ErasedFileAstId) -> Self {
        Self { raw, _ty: PhantomData }
    }
    pub fn upcast<M: ToAstNode>(self) -> FileAstId<M> {
        FileAstId::from_erased(self.raw)
    }
}


/// useful for pattern matching out of items to retain type info.
/// similar to ast::Item, but on the id level.
/// ErasedFileAstId -> FileAstId -> ItemId
/// either this or we can_cast 
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemId {
    SourceFile(FileAstId<ast::SourceFile>),
    Import(FileAstId<ast::Import>),
    Contract(FileAstId<ast::Contract>),
    Interface(FileAstId<ast::Interface>),
    Library(FileAstId<ast::Library>),
    Function(FileAstId<ast::Function>),
    Var(FileAstId<ast::Var>),
    Struct(FileAstId<ast::Struct>),
    Enum(FileAstId<ast::Enum>),
    Event(FileAstId<ast::Event>),
    Error(FileAstId<ast::Error>),
    Modifier(FileAstId<ast::Modifier>),
}

impl ItemId {
    pub fn upcast(self) -> FileAstId<ast::Item> {
        match self {
            ItemId::SourceFile(id) => id.upcast(),
            ItemId::Import(id) => id.upcast(),
            ItemId::Contract(id) => id.upcast(),
            ItemId::Interface(id) => id.upcast(),
            ItemId::Library(id) => id.upcast(),
            ItemId::Function(id) => id.upcast(),
            ItemId::Var(id) => id.upcast(),
            ItemId::Struct(id) => id.upcast(),
            ItemId::Enum(id) => id.upcast(),
            ItemId::Event(id) => id.upcast(),
            ItemId::Error(id) => id.upcast(),
            ItemId::Modifier(id) => id.upcast(),
        }
    }

    pub fn erase(self) -> ErasedFileAstId {
        match self {
            ItemId::SourceFile(id) => id.erase(),
            ItemId::Import(id) => id.erase(),
            ItemId::Contract(id) => id.erase(),
            ItemId::Interface(id) => id.erase(),
            ItemId::Library(id) => id.erase(),
            ItemId::Function(id) => id.erase(),
            ItemId::Var(id) => id.erase(),
            ItemId::Struct(id) => id.erase(),
            ItemId::Enum(id) => id.erase(),
            ItemId::Event(id) => id.erase(),
            ItemId::Error(id) => id.erase(),
            ItemId::Modifier(id) => id.erase(),
        }
    }
}

/// Unique global index for node
#[derive(PartialEq, Eq)]
pub struct AstId<N> {
    pub file_id: FileId,//salsa file id
    pub local_id: FileAstId<N>,
}

impl<N> Hash for AstId<N> {
    fn hash<H: std::hash::Hasher>(&self, hasher: &mut H) {
        self.file_id.hash(hasher);
        self.local_id.hash(hasher);
    }
}

impl<N> Clone for AstId<N> {
    fn clone(&self) -> Self { *self }
}

impl<N> Copy for AstId<N> {}




pub fn is_supported_node(node: &Node<'_>) -> bool {
    matches!(// I dont know if i want the identifiers yet, everything basically contains one, i guess lookup will determine
        NodeKind::from(node.kind_id()),
        NodeKind::IMPORT_DIRECTIVE
        | NodeKind::CONTRACT_DEFINITION
        | NodeKind::INTERFACE_DEFINITION
        | NodeKind::LIBRARY_DEFINITION
        | NodeKind::FUNCTION_DEFINITION
        | NodeKind::STRUCT_DEFINITION
        | NodeKind::ENUM_DEFINITION
        | NodeKind::EVENT_DEFINITION
        | NodeKind::ERROR_DEFINITION
        | NodeKind::MODIFIER_DEFINITION
        | NodeKind::CONTRACT_BODY //covers interface & library bodies: should this be a parent? the parent should be the contract definition, no?
        | NodeKind::STATE_VAR_DECLARATION
        | NodeKind::CONST_VAR_DECLARATION
    )
}


#[derive(Debug, PartialEq, Eq)]
pub struct AstIdMap {
    ptr_to_id: FxHashMap<NodePtr, ErasedFileAstId>,
    id_to_ptr: FxHashMap<ErasedFileAstId, NodePtr>,
}

impl AstIdMap {
    pub fn new(root: &AstNode) -> Self {
        assert!(root.is_root());

        // Very shallow/simplified id allocation algo for now: does BFS
        // id system only works for 2 layers, thankfully we don't need more than 2 layers
        // Global layer: consts, free fns, contracts/interface/library etc.
        // 2nd layer: contract/interface/library members
        // everything below function level is ignored
        let mut ptr_id = 0;
        let mut ptr_to_id = FxHashMap::default();
        let mut id_to_ptr = FxHashMap::default();
        let mut queue = VecDeque::new();
        let mut parent_map = FxHashMap::default();

        queue.push_back((root.node(), None));

        while let Some((node, parent)) = queue.pop_front() {
            let node_ptr = NodePtr::from_raw_node(node);
            let id = Self::generate_id(parent, ptr_id, &mut parent_map);
            ptr_id += 1;
            ptr_to_id.insert(node_ptr, id);
            id_to_ptr.insert(id, node_ptr);

            for child in node.named_children(&mut node.walk()).filter(is_supported_node) {
                //source file should not be a parent to top level items
                let parent = if node.kind_id() == NodeKind::SOURCE_FILE {None} else {Some(id)};
                queue.push_back((child, parent));
            }
        }

        Self { ptr_to_id, id_to_ptr }.shrink_to_fit()
    }

    fn shrink_to_fit(mut self) -> Self {
        self.ptr_to_id.shrink_to_fit();
        self.id_to_ptr.shrink_to_fit();
        self
    }

    fn generate_id(parent: Option<ErasedFileAstId>, index: usize, parent_map: &mut FxHashMap<usize, usize>) -> ErasedFileAstId {
        if let Some(parent) = parent {
            let idx = parent_map.entry(parent.id as usize).or_default();
            let id = *idx;
            *idx += 1;
            ErasedFileAstId { id: id as u32, parent: Some(parent.id) }
        } else {
            ErasedFileAstId { id: index as u32, parent: None }
        }
    }

    pub fn id_erased(&self, ptr: NodePtr) -> Option<ErasedFileAstId> {
        self.ptr_to_id.get(&ptr).copied()
    }

    /// No type Inference, Lets the call site specify the type of node.
    pub fn id_of_ptr<N: ToAstNode>(&self, ptr: NodePtr) -> Option<FileAstId<N>> {
        self.id_erased(ptr).map(FileAstId::from_erased)
    }

    /// No type Inference, Lets the call site specify the type of node.
    pub fn id_of_node<N: ToAstNode>(&self, n: Node<'_>) -> Option<FileAstId<N>> {
        self.id_of_ptr(NodePtr::from_raw_node(n))
    }
 
    pub fn id_of<N: ToAstNode>(&self, n: &N) -> Option<FileAstId<N>> {
        let ptr = NodePtr::from_ast_node(n.ast_node_ref());
        self.id_of_ptr(ptr)
    }
 
    pub fn get<N: ToAstNode>(&self, root: &AstNode, id: FileAstId<N>) -> Option<N> {
        let ptr = self.id_to_ptr.get(&id.erase()).copied();
        //Assert root contains node?
        let node = ptr.and_then(|ptr| ptr.to_node(root)).unwrap();
        N::cast(node)
    }
}




///                NODEPTR
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PtrRange {
    pub start: u32,
    pub end: u32,
}

impl PtrRange {
    #[inline]
    pub fn from_range(range: Range) -> Self {
        Self { start: range.start_byte as u32, end: range.end_byte as u32 }
    }

    #[inline]
    pub fn contains(&self, other: &PtrRange) -> bool {
        self.start <= other.start && self.end >= other.end
    }
}


/// A light-weight repr of a node with kind info
/// Used to index nodes in the ast_id_map
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodePtr {
    kind: NodeKind,
    range: PtrRange,
}

impl NodePtr {
    #[inline]
    pub fn from_ast_node(node: &AstNode) -> Self {
        Self { kind: node.node().kind_id().into(), range: PtrRange::from_range(node.node().range()) }
    }

    #[inline]
    pub fn from_raw_node(node: Node) -> Self {
        Self { kind: node.kind_id().into(), range: PtrRange::from_range(node.range()) }
    }

    pub fn to_node(&self, root: &AstNode) -> Option<AstNode> {
        root.child_node(self.range).filter(|a| a.node().kind_id() == self.kind)
    }

    //Note: we can also go from ptr to typed ast::item
}