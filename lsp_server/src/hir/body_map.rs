#![allow(unused)]
use la_arena::{Arena, ArenaMap, Idx};
use rustc_hash::FxHashMap;
use smallvec::SmallVec;
use tree_sitter::Node;
use crate::ast::kinds::{FieldKind, NodeKind};
use crate::ast::{AstNode, FunctionId, ModifierId, NodeRange, ToAstNode};
use crate::hir::exprs::{Expr, ExprBuilder, ExprId, Name};
use crate::hir::item_data::{ExprStore, FieldId, VariantId};
use crate::hir::types::{Mutability, Primitive, TypeBuilder, TypeId, TypeName, Visibility};

pub type ByteOffset = u32;

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum SemanticId {
    //Def specific semantics
    Local(LocalId),
    Field(FieldId),
    Variant(VariantId),
    
    // Global semantics
    Expr(ExprId),
    Type(TypeId),
    TypeSegment {
        ty: TypeId,
        segment: u8
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub enum VariableKind {
    #[default]
    Variable,
    Parameter,
    Const,
    Immutable,
    State,
}

impl std::fmt::Display for VariableKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VariableKind::Parameter => write!(f, "parameter"),
            VariableKind::Variable => write!(f, "variable"),
            VariableKind::Const => write!(f, "constant"),
            VariableKind::Immutable => write!(f, "immutable"),
            VariableKind::State => write!(f, "state"),
        }
    }
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Location {
    #[default]
    Stack,//for builtins/primitive
    Memory,
    Calldata,
    Storage,
}

impl std::fmt::Display for Location {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Location::Memory => write!(f, "memory"),
            Location::Calldata => write!(f, "calldata"),
            Location::Storage => write!(f, "storage"),
            Location::Stack => write!(f, "stack"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BodyOwnerId {
    Function(FunctionId),
    Modifier(ModifierId)
}

pub type ScopeId = Idx<Scope>;

#[derive(PartialEq, Eq)]
pub struct Scope {//TODO use a unified scope obj for this and defmap. Only  diff is range  and name-> type, and i dont think we need range
    parent: Option<ScopeId>,
    range: NodeRange,
    by_name: FxHashMap<Name, LocalId>//solidity does not support same-scope shadowing so no vec
}

impl Scope {
    pub fn get(&self, name: &Name) -> Option<&LocalId> {
        self.by_name.get(name)
    }

    pub fn parent(&self) -> Option<ScopeId> {
        self.parent
    }

    pub fn range(&self) -> NodeRange {
        self.range
    }
}

pub type LocalId = Idx<Local>;

#[derive(PartialEq, Eq)]
pub struct Local {
    name: Name,//FIXME: name should be optional
    kind: VariableKind,
    type_name: TypeId,    
    location: Location,
    offset: ByteOffset,//from where this decl becomes visible
    init: Option<ExprId>,
}

#[derive(PartialEq, Eq)]
pub struct BodyMap {
    pub root_scope: ScopeId,
    pub scopes: Arena<Scope>,
    pub locals: Arena<Local>,
    pub expr_store: ExprStore,
}

#[derive(PartialEq, Eq)]
pub struct BodySourceMap {
    pub expr_scopes: ArenaMap<ExprId,ScopeId>,
    pub expr_to_node: ArenaMap<ExprId, NodeRange>,
    pub type_to_node: ArenaMap<TypeId,NodeRange>,
}

impl BodyMap {
    pub fn local(&self, decl: LocalId) -> &Local {
        &self.locals[decl]
    }

    pub fn root_scope(&self) -> ScopeId {
        self.root_scope
    }

    pub fn scope(&self, scope: ScopeId) -> &Scope {
        &self.scopes[scope]
    }
}

impl Local {
    pub fn new(name: Name, kind: VariableKind, type_name: TypeId, location: Location, offset: ByteOffset, init: Option<ExprId>) -> Self {
        Self { name, kind, type_name, location, offset, init }
    }

    pub fn name(&self) -> &Name {
        &self.name
    }
    pub fn kind(&self) -> VariableKind {
        self.kind
    }

    pub fn type_name(&self) -> &TypeId {
        &self.type_name
    }

    pub fn location(&self) -> Location {
        self.location
    }

    pub fn offset(&self) -> ByteOffset {
        self.offset
    }

    pub fn init(&self) -> Option<ExprId> {
        self.init
    }
}

pub struct BodyBuilder {
    current_scope: ScopeId,
    ast_root: AstNode,
    root_scope: ScopeId,
    scopes: Arena<Scope>,
    locals: Arena<Local>,
    expr_store: ExprStore,
    expr_scopes: ArenaMap<ExprId,ScopeId>,
    expr_to_node: ArenaMap<ExprId, NodeRange>,
    type_to_node: ArenaMap<TypeId,NodeRange>,
}


impl BodyBuilder {
    pub fn build(ast_root: AstNode, owner: AstNode) -> Option<(BodyMap,BodySourceMap)> {
        
        let mut scopes = Arena::new();
        let root_scope = scopes.alloc(Scope {
            parent: None,
            //we use "fn range" instead of the intuitive "body range" because parameters are visible from decl. 
            //if root scope starts at body then parameter offset < body offset(ie outside the fn scope)
            //resolution fails @BodyMap::scope_at(offset)
            range: NodeRange::from(&owner.node()),
            by_name: FxHashMap::default(),
        });

        let mut builder = BodyBuilder {
            current_scope: root_scope,
            ast_root,
            root_scope,
            scopes,
            locals: Arena::new(),
            expr_store: ExprStore::default(),
            type_to_node: ArenaMap::new(),
            expr_to_node:ArenaMap::new(),
            expr_scopes: ArenaMap::new(),
        };
        
        builder.collect_parameters(owner.node());

        if let Some(body) = owner.node().child_by_field_id(FieldKind::BODY.into()) {
            //Interface fns dont have bodies
            builder.walk_block(body);
        }

        Some(builder.finish())
    }

    fn push_scope(&mut self, range: NodeRange) {
        self.current_scope = self.alloc_scope(range);
    }

    fn pop_scope(&mut self) {
        self.current_scope = self.scopes[self.current_scope].parent.unwrap();
    }


    fn walk_block(&mut self, block: Node<'_>) {
        for child in block.named_children(&mut block.walk()) {//Switch to cursor walking
            match child.kind_id().into() {
                NodeKind::VAR_DECLARATION_STATEMENT => self.collect_declaration(child),
                // try statements have bare parameters as direct desendants for some reason
                NodeKind::PARAMETER => self.add_local(child, VariableKind::Parameter, child.start_byte() as u32),
                NodeKind::STATEMENT | NodeKind::IF_STATEMENT | NodeKind::WHILE_STATEMENT | 
                NodeKind::DO_WHILE_STATEMENT |NodeKind::RETURN_STATEMENT => self.walk_block(child),
                NodeKind::BLOCK_STATEMENT | NodeKind::FOR_STATEMENT | NodeKind::TRY_STATEMENT => {//maybe seperate later so loop var is declared in inner block scope
                    self.push_scope(NodeRange::from(&child));
                    self.walk_block(child);
                    self.pop_scope();
                }
                NodeKind::EXPRESSION | NodeKind::EXPRESSION_STATEMENT => {
                    self.lower_expr(child);
                }
                NodeKind::EMIT_STATEMENT | NodeKind::REVERT_STATEMENT => {
                    self.lower_expr(child);
                }
                //TODO try/catch
                _  => { //we need top level/statement level error nodes handling. resolve the exprs inside if any
                    self.walk_block(child);
                }
            }
        }
    }

    fn collect_declaration(&mut self, node: Node<'_>) {
        for child in node.named_children(&mut node.walk()) {
            match child.kind_id().into() {
                NodeKind::VAR_DECLARATION => {
                    self.add_local(
                        child,
                        VariableKind::Variable,
                        //FIXME offset should be end of statement
                        child.range().end_byte as ByteOffset,
                    );
                }
                NodeKind::VAR_DECLARATION_TUPLE => {
                    for child in child.named_children(&mut child.walk()) {
                        self.add_local(
                            child,
                            VariableKind::Variable,
                            child.range().end_byte as ByteOffset,
                        );
                    }
                }
                NodeKind::EXPRESSION => {
                    self.lower_expr(child);
                }
                _ => {}
            }
        }
    }

    fn collect_parameters(&mut self, owner_node: Node<'_>) {
        for child in owner_node.named_children(&mut owner_node.walk()) {
            match child.kind_id().into() {
                NodeKind::IDENTIFIER => {
                    self.lower_expr(child);
                }
                NodeKind::PARAMETER => {
                    self.add_local(
                        child,
                        VariableKind::Parameter,
                        child.range().start_byte as ByteOffset,
                    );
                }
                NodeKind::MODIFIER_INVOCATION => {
                    self.lower_expr(child);
                },
                NodeKind::RETURN_DEFINITION => self.collect_parameters(child),
                _ => {}//there's a return_parameter node, find where/how its used
            }
        }
    }

    fn add_local(&mut self, node: Node, kind: VariableKind, offset: ByteOffset) {
        let mut name: Name = "".into();
        let mut type_name = None;
        let mut location = Location::default();
        let mut range = None;
        let mut init = None;
        for child in node.children(&mut node.walk()) {
            match child.kind_id().into() {
                NodeKind::IDENTIFIER => {
                    name = self.ast_root.text_by_range(child.byte_range()).trim().into();
                    // we use the identifier range for the declaration
                    range = Some(NodeRange::from(&child));
                }
                NodeKind::TYPE_NAME => type_name = self.lower_type(child),
                NodeKind::MEMORY => location = Location::Memory,
                NodeKind::STORAGE => location = Location::Storage,
                NodeKind::CALLDATA => location = Location::Calldata,
                NodeKind::EXPRESSION => { init = self.lower_expr(child); },
                _ => {}
            }
        }

        if name.is_empty() || type_name.is_none(){
            return;
        }

        let local_id = self.locals.alloc(Local {
            name: name.clone(),
            kind,
            type_name: type_name.unwrap(),
            location,
            offset,
            init,
        });
        
        self.scopes[self.current_scope].by_name.insert(name, local_id);
        self.expr_store.range_to_semantic.insert(range.unwrap(), SemanticId::Local(local_id));
    }


    fn alloc_scope(&mut self, range: NodeRange) -> ScopeId { 
        self.scopes.alloc(Scope {
            parent: Some(self.current_scope),
            range,
            by_name: FxHashMap::default(),
        })
 
    }

    fn finish(self) -> (BodyMap,BodySourceMap) {
        let BodyBuilder { root_scope , mut scopes,locals: mut decls, mut expr_store, mut expr_scopes, mut expr_to_node, mut type_to_node, ..} = self;
        scopes.shrink_to_fit();
        decls.shrink_to_fit();
        expr_store.exprs.shrink_to_fit();
        expr_store.types.shrink_to_fit();
        expr_store.range_to_semantic.shrink_to_fit();
        type_to_node.shrink_to_fit();
        expr_scopes.shrink_to_fit();
        expr_to_node.shrink_to_fit();
        (BodyMap {root_scope, scopes, locals: decls, expr_store },
        BodySourceMap { expr_scopes, expr_to_node, type_to_node })
    }
}


impl ExprBuilder for BodyBuilder {
    fn root(&self) -> &AstNode {
        &self.ast_root
    }

    fn alloc_expr(&mut self, expr: Expr, node: Node) -> ExprId {
        let expr_id = self.expr_store.exprs.alloc(expr);
        let range = NodeRange::from(&node);
        self.expr_to_node.insert(expr_id, range);
        self.expr_store.range_to_semantic.insert(range, SemanticId::Expr(expr_id));
        self.expr_scopes.insert(expr_id, self.current_scope);
        expr_id
    }

    fn alloc_member_expr(&mut self, member: Expr, node: Node, prop: NodeRange) -> ExprId {
        let mem_id = self.alloc_expr(member, node);
        self.expr_store.range_to_semantic.insert(prop, SemanticId::Expr(mem_id));
        mem_id
    }

    fn alloc_call_expr(&mut self, call: Expr, node: Node, ident: Option<NodeRange>) -> ExprId {
        let call_id = self.alloc_expr(call, node);
        if let Some(ident) = ident {
             self.expr_store.range_to_semantic.insert(ident, SemanticId::Expr(call_id));
        };
        call_id
    }
}

impl TypeBuilder for BodyBuilder {
    fn alloc_type(&mut self, ty: TypeName, node: Node) -> TypeId {
        let seg_count = ty.seg_count();
        let ty_id = self.expr_store.types.alloc(ty);
        let ptr = NodeRange::from(&node);
        self.type_to_node.insert(ty_id, ptr);
        self.expr_store.range_to_semantic.insert(ptr, SemanticId::Type(ty_id));
        if seg_count > 1 {
            self.alloc_segments(node, ty_id);
        }
        ty_id
    }

    fn alloc_segments(&mut self, node: Node, ty: TypeId) {
        for (seg, child) in node.named_children(&mut node.walk()).filter(|n| n.kind_id() == NodeKind::IDENTIFIER).enumerate() {
            self.expr_store.range_to_semantic.insert(NodeRange::from(&child), SemanticId::TypeSegment { ty, segment: seg as u8 });
        }
    }
}



#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use rustc_hash::FxHashMap;
    use tree_sitter::Node;

    use crate::{
        ast, hir::exprs::Literal, loader::{LoadedFile, SourceRootBundle}, salsa::{FileId, RootDatabase, SalsaDatabase}, workspace::{
            Package as WsPackage, PackageConfig, PackageId, PackageKind, SourceRootId, Workspace,
        },
    };
    use ropey::Rope;

    const TYPE_FIXTURE: &str = r#"
        pragma solidity ^0.8.30;

        contract A {
            function testA() external {
                uint a;
                uint[10] b;
                uint[Const] c;
                uint[1][3] d;
                ContractType e;
                Path.ContractType f;
                mapping(address => ContractType) g;
                function(uint) external view returns(int) h;
                (uint8 i, uint32 j) = (1, 2);
            }
        }
    "#;

    fn make_test_db(source: &str) -> (SalsaDatabase, FileId) {
        let root = Utf8PathBuf::from("/tmp/body_map_tests");
        let file_path = root.join("src/Test.sol");

        let mut package_id = FxHashMap::default();
        package_id.insert(root.clone(), PackageId(0));

        let workspace = Workspace {
            root: root.clone(),
            packages: vec![WsPackage {
                kind: PackageKind::Foundry,
                root: root.clone(),
                source_roots: vec![SourceRootId(0)],
                config: PackageConfig::default(),
                is_dependency: false,
            }],
            package_id,
        };

        let bundle = SourceRootBundle {
            source_root_id: SourceRootId(0),
            package_id: PackageId(0),
            is_dependency: false,
            files: vec![LoadedFile {
                path: file_path.clone(),
                text: Rope::from_str(source),
            }],
        };

        let db = SalsaDatabase::new(workspace, vec![bundle]);

        let file = db
            .files
            .get(&file_path)
            .expect("fixture file should be registered in salsa files");

        (db, file)
    }

    fn find_first_kind<'a>(node: Node<'a>, kind: NodeKind) -> Option<Node<'a>> {
        if node.kind_id() == kind {
            return Some(node);
        }

        for child in node.named_children(&mut node.walk()) {
            if let Some(found) = find_first_kind(child, kind) {
                return Some(found);
            }
        }

        None
    }

    fn build_maps_for_first_function(source: &str) -> (BodyMap, BodySourceMap) {
        let (db, file) = make_test_db(source);
        let ast = db.ast(file);
        let root = ast.root();
        let fn_node = find_first_kind(root.node(), NodeKind::FUNCTION_DEFINITION)
            .expect("fixture should contain a function_definition node");

        let fn_id = db
            .ast_id_map(file)
            .id_of_node::<ast::Function>(fn_node)
            .expect("fixture function should map to AstId<Function>");

        let owner = BodyOwnerId::Function(FunctionId { file, id: fn_id });

        let ast_root = db.ast(file).root();
        let ast_id_map = db.ast_id_map(file);
        let owner = ast_id_map.get(&ast_root, fn_id).unwrap().to_node();

        BodyBuilder::build(ast_root, owner).expect("BodyBuilder::build should succeed for fixture")
    }

    fn local_type<'a>(
        body_map: &'a BodyMap,
        source_map: &'a BodySourceMap,
        name: &str,
    ) -> &'a TypeName {
        let scope = &body_map.scopes[body_map.root_scope];
        let local_id = *scope
            .by_name
            .get(name)
            .expect("expected local declaration to exist");

        let type_id = *body_map.local(local_id).type_name();
        &body_map.expr_store.types[type_id]
    }

    #[test]
    fn collects_tuple_declarations_into_scope() {
        let (body_map, _) = build_maps_for_first_function(TYPE_FIXTURE);
        let scope = &body_map.scopes[body_map.root_scope];

        assert!(scope.by_name.contains_key("i"));
        assert!(scope.by_name.contains_key("j"));
    }

    #[test]
    fn lowers_mapping_function_and_nested_array_types() {
        let (body_map, source_map) = build_maps_for_first_function(TYPE_FIXTURE);

        match local_type(&body_map, &source_map, "g") {
            TypeName::Mapping { key, value } => {
                assert!(matches!(
                    body_map.expr_store.types[*key],
                    TypeName::Primitive(Primitive::Address)
                ));

                match &body_map.expr_store.types[*value] {
                    TypeName::UserDefined(path) => {
                        assert_eq!(path.segments.len(), 1);
                        assert_eq!(path.segments[0].as_str(), "ContractType");
                    }
                    _ => panic!("expected mapping value user-defined type"),
                }
            }
            _ => panic!("expected `g` to lower to TypeName::Mapping"),
        }

        match local_type(&body_map, &source_map, "h") {
            TypeName::Fn(fn_ty) => {
                assert!(matches!(fn_ty.vis, Visibility::External));
                assert!(matches!(fn_ty.mutability, Mutability::View));
                assert_eq!(fn_ty.params.len(), 1);
                assert_eq!(fn_ty.ret.len(), 1);

                assert!(matches!(
                    body_map.expr_store.types[fn_ty.params[0]],
                    TypeName::Primitive(Primitive::Uint(256))
                ));
                assert!(matches!(
                    body_map.expr_store.types[fn_ty.ret[0]],
                    TypeName::Primitive(Primitive::Int(256))
                ));
            }
            _ => panic!("expected `h` to lower to TypeName::Fn"),
        }

        match local_type(&body_map, &source_map, "d") {
            TypeName::Array {
                ty: outer_base,
                size: Some(outer_size),
            } => {
                assert!(matches!(
                    body_map.expr_store.exprs[*outer_size],
                    Expr::Literal(Literal::Number(_))
                ));

                match &body_map.expr_store.types[*outer_base] {
                    TypeName::Array {
                        ty: inner_base,
                        size: Some(inner_size),
                    } => {
                        assert!(matches!(
                            body_map.expr_store.exprs[*inner_size],
                            Expr::Literal(Literal::Number(_))
                        ));
                        assert!(matches!(
                            body_map.expr_store.types[*inner_base],
                            TypeName::Primitive(Primitive::Uint(256))
                        ));
                    }
                    _ => panic!("expected nested array for `d` base type"),
                }
            }
            _ => panic!("expected `d` to lower to nested TypeName::Array"),
        }
    }
}