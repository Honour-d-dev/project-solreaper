
use std::mem;

use la_arena::{Arena, Idx};
use rustc_hash::FxHashMap;
use tree_sitter::Node;

use crate::ast::kinds::{FieldKind, NodeKind};
use crate::ast::{AstNode, Contract, Enum, Error, Event, Function, HasBases, HasName, Import, ImportType, Interface, Library, Modifier, NodeRange, Struct, ToAstNode, Var};
use crate::hir::body_map::{ByteOffset, Local, LocalId, Location, SemanticId, VariableKind};
use crate::hir::exprs::{Expr, ExprBuilder, ExprId, Name};
use crate::hir::types::{Mutability, TypeBuilder, TypeId, TypeName, Visibility};

#[derive(PartialEq, Eq)]
pub struct ExprStore {
    pub types: Arena<TypeName>,
    pub exprs: Arena<Expr>,
    pub range_to_semantic: FxHashMap<NodeRange, SemanticId>
}

impl Default for ExprStore {
    fn default() -> Self {
        Self {
            types: Arena::new(),
            exprs: Arena::new(),
            range_to_semantic: FxHashMap::default(),
        }
    }
}

#[derive(Default, PartialEq, Eq)]
pub struct ImportData {
    pub expr_store: ExprStore
}

#[derive(PartialEq, Eq)]
pub struct VarData {
    pub name: Name,
    pub type_name: TypeId,
    pub vis: Visibility,
    pub kind: VariableKind,
    /// Initializer expression for constant/immutable variables
    pub init: Option<ExprId>,
    /// Var type can be a complex type e.g mapping(x.y => y.z)
    /// we need a store of the component types so we can resove the type on demand
    pub expr_store: ExprStore
}

#[derive(PartialEq, Eq, Hash)]
pub struct Field {
    pub name: Name,
    pub type_name: TypeId,
}

pub type FieldId = Idx<Field>;

#[derive(PartialEq, Eq)]
pub struct StructData {
    pub name: Name,
    pub fields: Arena<Field>,
    pub expr_store: ExprStore,
}

#[derive(PartialEq, Eq, Hash)]
pub struct Variant {
    pub name: Name,
}

pub type VariantId = Idx<Variant>;


#[derive(PartialEq, Eq)]
pub struct EnumData {
    pub name: Name,
    pub variants: Arena<Variant>,
    pub expr_store: ExprStore,
}

#[derive(PartialEq, Eq)]
pub struct FunctionData {
    pub name: Name,
    pub vis: Visibility,
    pub mutability: Mutability,
    //TODO - Modifier invocations
    pub arg_params: Box<[LocalId]>,
    pub ret_params: Box<[LocalId]>,
    pub parameters: Arena<Local>,//parameters and returns
    pub expr_store: ExprStore,
}

#[derive(PartialEq, Eq)]
pub struct EventData {
    pub name: Name,
    pub parameters: Arena<Local>,
    pub expr_store: ExprStore,
}

#[derive(PartialEq, Eq)]
pub struct ErrorData {
    pub name: Name,
    pub parameters: Arena<Local>,
    pub expr_store: ExprStore,
}

#[derive(PartialEq, Eq)]
pub struct ModifierData {
    pub name: Name,
    pub parameters: Arena<Local>,
    pub expr_store: ExprStore,
}

#[derive(PartialEq, Eq)]
pub struct ContractData {
    pub name: Name,
    pub bases: Box<[TypeId]>,
    pub expr_store: ExprStore,
}

#[derive(PartialEq, Eq)]
pub struct InterfaceData {
    pub name: Name,
    pub bases: Box<[TypeId]>,
    pub expr_store: ExprStore,
}

#[derive(PartialEq, Eq)]
pub struct LibraryData {
    pub name: Name,
    pub expr_store: ExprStore,
}


pub struct ItemBuilder {
    root: AstNode,
    expr_store: ExprStore
}

impl ExprBuilder for ItemBuilder {
    fn root(&self) -> &AstNode {
        &self.root
    }

    fn alloc_expr(&mut self, expr: Expr, node: Node) -> ExprId {
        let expr_id = self.expr_store.exprs.alloc(expr);
        self.expr_store.range_to_semantic.insert(NodeRange::from(&node), SemanticId::Expr(expr_id));
        expr_id
    }

    fn alloc_member_expr(&mut self, member: Expr, range: NodeRange, node: Node) -> ExprId {
        let mem_id = self.alloc_expr(member, node);
        self.expr_store.range_to_semantic.insert(range, SemanticId::Expr(mem_id));
        mem_id
    }

    fn alloc_call_expr(&mut self, call: Expr, callee_node: Node, node: Node) -> ExprId {
        let call_id = self.alloc_expr(call, node);
        let callee_range = NodeRange::from(&callee_node);
        self.expr_store.range_to_semantic.insert(callee_range, SemanticId::Expr(call_id));
        if callee_node.kind_id() == NodeKind::MEMBER_EXPRESSION {
            if let Some(prop) = callee_node.child_by_field_id(FieldKind::PROPERTY.into()) {
                self.expr_store.range_to_semantic.insert(NodeRange::from(&prop), SemanticId::Expr(call_id));
            }
        }
        call_id
    }
}

impl TypeBuilder for ItemBuilder {
    fn alloc_type(&mut self, ty: TypeName, node: Node) -> TypeId {
        let seg_count = ty.seg_count();
        let ty_id = self.expr_store.types.alloc(ty);
        let ptr = NodeRange::from(&node);
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

impl ItemBuilder {
    pub fn new(root: AstNode) -> Self {
        Self {
            root,
            expr_store: ExprStore::default(),
        }
    }

    /////////////////////////////////////////////////////////////////////////////////////////////////////////
    ///                                        IMPORT BUILDER                                             ///
    ////////////////////////////////////////////////////////////////////////////////////////////////////////
    
    pub fn build_import(&mut self, import: &Import) -> ImportData {
        let import_type = import.import_type();
        let import_node = import.raw().node();
        match import_type {
            ImportType::Named { symbols } => {
                let mut name_cursor = import_node.walk();
                let mut alias_cursor = import_node.walk();
                let mut names = import.raw().node().children_by_field_id(FieldKind::IMPORT_NAME.into(), &mut name_cursor);
                let mut aliases = import.raw().node().children_by_field_id(FieldKind::ALIAS.into(), &mut alias_cursor);
                for symbol in symbols {
                    let expr = if let Some(alias) = symbol.alias {
                        let alias_node = aliases.next().unwrap();
                        let expr = Expr::Ident(alias);
                        self.alloc_expr(expr.clone(), alias_node);
                        expr
                    } else {
                        Expr::Ident(symbol.name)
                    };
                    let name_node = names.next().unwrap();
                    self.alloc_expr(expr, name_node);
                }
            },
            ImportType::Namespace { alias } => {
                let expr = Expr::Ident(alias);
                let node = import_node.child_by_field_id(FieldKind::ALIAS.into()).unwrap();
                self.alloc_expr(expr, node);

            },
            ImportType::Full => {}//only has path
        };

        let node = import_node.child_by_field_id(FieldKind::SOURCE.into()).unwrap();
        let path = import.raw().text_by_range(node.byte_range()).trim_matches(['"', '\'']);
        let expr = Expr::Path(path.into());
        self.alloc_expr(expr, node);

        ImportData { expr_store: mem::take(&mut self.expr_store) }
    }



    /////////////////////////////////////////////////////////////////////////////////////////////////////////
    ///                                      VARIABLE BUILDER                                             ///
    ////////////////////////////////////////////////////////////////////////////////////////////////////////
    
    pub fn build_var(&mut self, var: &Var) -> VarData {
        let node = var.raw().node();
        let mut name = Name::default();
        let mut vis = Visibility::default();
        let mut kind = VariableKind::State;
        let mut type_name = None;
        let mut init = None;
        for child in node.children(&mut node.walk()) {
            match child.kind_id().into() {
                NodeKind::TYPE_NAME => type_name = self.lower_type(child),
                NodeKind::CONSTANT => kind = VariableKind::Const,
                NodeKind::IMMUTABLE => kind = VariableKind::Immutable,
                NodeKind::VISIBILITY => vis = Visibility::parse(self.root().text_by_range(child.byte_range()).trim()),
                NodeKind::IDENTIFIER => {
                    self.lower_expr(child);
                    name = self.root.text_by_range(child.byte_range()).trim().into();
                }
                NodeKind::EXPRESSION => { init = self.lower_expr(child); },
                _ => {}
            }
        }
        VarData {
            name,
            type_name: type_name.unwrap(),
            vis,
            kind,
            init,
            expr_store: std::mem::take(&mut self.expr_store),
        }
    }

    /////////////////////////////////////////////////////////////////////////////////////////////////////////
    ///                                      STRUCT BUILDER                                               ///
    ////////////////////////////////////////////////////////////////////////////////////////////////////////
    
    pub fn build_struct(&mut self, strukt: &Struct) -> StructData {
        let node = strukt.raw().node();
        let name = strukt.name().unwrap_or_default();
        let mut fields = Arena::new();

        node.child_by_field_id(FieldKind::NAME.into()).map(|n| self.lower_expr(n));

        if let Some(body) = node.child_by_field_id(FieldKind::BODY.into()) {
            for member in body.named_children(&mut body.walk()) {
                match member.kind_id().into() {
                    NodeKind::STRUCT_MEMBER => {
                        let Some((name, range)) = member
                            .child_by_field_id(FieldKind::NAME.into())
                            .map(|n| {
                                (self.root().text_by_range(n.byte_range()).trim().into(), NodeRange::from(&n))
                            }) else {continue;};

                        let Some(type_name) = member
                            .child_by_field_id(FieldKind::TYPE.into())
                            .and_then(|ty| self.lower_type(ty))
                            else {continue;};

                        let field_id = fields.alloc(Field {
                            name,
                            type_name,
                        });

                        self.expr_store.range_to_semantic.insert(NodeRange::from(&member), SemanticId::Field(field_id));
                        self.expr_store.range_to_semantic.insert(range, SemanticId::Field(field_id));
                    }
                    _ => {}
                }
            }
        }

        StructData {
            name,
            fields,
            expr_store: std::mem::take(&mut self.expr_store),
        }
    }

    /////////////////////////////////////////////////////////////////////////////////////////////////////////
    ///                                      ENUM BUILDER                                                 ///
    ////////////////////////////////////////////////////////////////////////////////////////////////////////
    
    pub fn build_enum(&mut self, enum_: &Enum) -> EnumData {
        let node = enum_.raw().node();
        let name = enum_.name().unwrap_or_default();
        let mut variants = Arena::new();

        node.child_by_field_id(FieldKind::NAME.into()).map(|n| self.lower_expr(n));

        if let Some(body) = node.child_by_field_id(FieldKind::BODY.into()) {
            for value in body.named_children(&mut body.walk()) {
                if value.kind_id() == NodeKind::ENUM_VALUE {
                    let name = self.root().text_by_range(value.byte_range()).trim().into();

                    let variant_id = variants.alloc(Variant {
                        name,
                    });
                    self.expr_store.range_to_semantic.insert(NodeRange::from(&value), SemanticId::Variant(variant_id));
                }
            }
        }

        EnumData {
            name,
            variants,
            expr_store: std::mem::take(&mut self.expr_store),
        }
    }

    /////////////////////////////////////////////////////////////////////////////////////////////////////////
    ///                                      EVENT BUILDER                                                ///
    ////////////////////////////////////////////////////////////////////////////////////////////////////////


    pub fn build_event(mut self, event: &Event) -> EventData {
        let node = event.raw().node();
        let name = event.name().unwrap_or_default();
        let mut parameters = Arena::new();

        node.child_by_field_id(FieldKind::NAME.into()).map(|n| self.lower_expr(n));

        for child in node.named_children(&mut node.walk()) {
            match child.kind_id().into() {
                NodeKind::EVENT_PARAMETER => {
                    self.build_parameter(&mut parameters, child);
                }   
                _ => {}
            }
        }

        EventData {
            name,
            parameters,
            expr_store: std::mem::take(&mut self.expr_store),
        }
    }




    /////////////////////////////////////////////////////////////////////////////////////////////////////////
    ///                                      ERROR BUILDER                                                ///
    ////////////////////////////////////////////////////////////////////////////////////////////////////////


    pub fn build_error(mut self, error: &Error) -> ErrorData {
        let node = error.raw().node();
        let name = error.name().unwrap_or_default();
        let mut parameters = Arena::new();

        node.child_by_field_id(FieldKind::NAME.into()).map(|n| self.lower_expr(n));

        for child in node.named_children(&mut node.walk()) {
            match child.kind_id().into() {
                NodeKind::ERROR_PARAMETER => {
                    self.build_parameter(&mut parameters, child);
                }   
                _ => {}
            }
        }

        ErrorData {
            name,
            parameters,
            expr_store: std::mem::take(&mut self.expr_store),
        }
    }



    /////////////////////////////////////////////////////////////////////////////////////////////////////////
    ///                                      MODIF  IER BUILDER                                             ///
    ////////////////////////////////////////////////////////////////////////////////////////////////////////


    pub fn build_modifier(mut self, modifier: &Modifier) -> ModifierData {
        let node = modifier.raw().node();
        let name = modifier.name().unwrap_or_default();
        let mut parameters = Arena::new();

        node.child_by_field_id(FieldKind::NAME.into()).map(|n| self.lower_expr(n));

        for child in node.named_children(&mut node.walk()) {
            match child.kind_id().into() {
                NodeKind::PARAMETER => {
                    self.build_parameter(&mut parameters, child);
                }   
                _ => {}
            }
        }

        ModifierData {
            name,
            parameters,
            expr_store: std::mem::take(&mut self.expr_store),
        }
    }


    /////////////////////////////////////////////////////////////////////////////////////////////////////////
    ///                                      FUNCTION BUILDER                                             ///
    ////////////////////////////////////////////////////////////////////////////////////////////////////////
    
    pub fn build_fn(mut self, func: &Function) -> FunctionData {
        let node = func.raw().node();
        let name = func.name().unwrap_or_default();
        let mut vis = Visibility::Internal;
        let mut mutability = Mutability::NonPayable;
        let mut parameters = Arena::new();
        let mut params = Vec::new();
        let mut rets = Vec::new();

        for child in node.named_children(&mut node.walk()) {
            match child.kind_id().into() {
                NodeKind::PARAMETER => {
                    if let Some(id) = self.build_parameter(&mut parameters, child) {
                        params.push(id);
                    }
                }
                NodeKind::RETURN_DEFINITION => {
                    for ret_child in child.named_children(&mut child.walk()) {
                        if ret_child.kind_id() == NodeKind::PARAMETER {
                            if let Some(id) = self.build_parameter(&mut parameters, ret_child) {
                                rets.push(id);
                            }
                        }
                    }
                }
                NodeKind::VISIBILITY => {
                    vis =  Visibility::parse(func.raw().text_by_range(child.byte_range()))
                }
                NodeKind::STATE_MUTABILITY => {
                    mutability = Mutability::parse(func.raw().text_by_range(child.byte_range()))
                }
                _ => {}
            }
        }

        FunctionData {
            name,
            vis,
            mutability,
            arg_params: params.into_boxed_slice(),
            ret_params: rets.into_boxed_slice(),
            parameters,
            expr_store: std::mem::take(&mut self.expr_store),
        }
    }

    fn build_parameter(&mut self, parameters: &mut Arena<Local>, node: Node) -> Option<LocalId> {
        let mut name: Name = "".into();
        let mut type_name = None;
        let mut location = Location::default();
        let mut name_range = None;

        for child in node.children(&mut node.walk()) {
            match child.kind_id().into() {
                NodeKind::IDENTIFIER => {
                    name = self.root().text_by_range(child.byte_range()).trim().into();
                    name_range = Some(NodeRange::from(&child));
                }
                NodeKind::TYPE_NAME => type_name = self.lower_type(child),
                NodeKind::MEMORY => location = Location::Memory,
                NodeKind::STORAGE => location = Location::Storage,
                NodeKind::CALLDATA => location = Location::Calldata,
                _ => {}
            }
        }

        let Some(type_name) = type_name else {
            return None;
        };
        let local_id = parameters.alloc(Local::new(
            name,
            VariableKind::Parameter,
            type_name,
            location,
            node.start_byte() as ByteOffset,
            None,
        ));

        self.expr_store.range_to_semantic.insert(NodeRange::from(&node), SemanticId::Local(local_id));

        if let Some(range) = name_range {
            self.expr_store.range_to_semantic.insert(range, SemanticId::Local(local_id));
        }
        Some(local_id)
    }

    /////////////////////////////////////////////////////////////////////////////////////////////////////////
    ///                                      CONTRACT BUILDER                                             ///
    ////////////////////////////////////////////////////////////////////////////////////////////////////////

    pub fn build_contract(mut self, contract: &Contract) -> ContractData {
        let node = contract.raw().node();
        let name = contract.name().unwrap_or_default();
        let mut bases = Vec::new();

        node.child_by_field_id(FieldKind::NAME.into()).map(|n| self.lower_expr(n));

        for child in node.named_children(&mut node.walk()) {
            if child.kind_id() == NodeKind::INHERITANCE_SPECIFIER {
                if let Some(base_node) = child.named_children(&mut child.walk())
                    .find(|n| n.kind_id() == NodeKind::USER_DEFINED_TYPE)
                {
                    if let Some(ty_id) = self.lower_type(base_node) {
                        bases.push(ty_id);
                    }
                }
            }
        }

        ContractData {
            name,
            bases: bases.into_boxed_slice(),
            expr_store: std::mem::take(&mut self.expr_store),
        }
    }

    pub fn build_interface(mut self, interface: &Interface) -> InterfaceData {
        let node = interface.raw().node();
        let name = interface.name().unwrap_or_default();
        let mut bases = Vec::new();

        node.child_by_field_id(FieldKind::NAME.into()).map(|n| self.lower_expr(n));

        for child in node.named_children(&mut node.walk()) {
            if child.kind_id() == NodeKind::INHERITANCE_SPECIFIER {
                if let Some(base_node) = child.named_children(&mut child.walk())
                    .find(|n| n.kind_id() == NodeKind::USER_DEFINED_TYPE)
                {
                    if let Some(ty_id) = self.lower_type(base_node) {
                        bases.push(ty_id);
                    }
                }
            }
        }

        InterfaceData {
            name,
            bases: bases.into_boxed_slice(),
            expr_store: std::mem::take(&mut self.expr_store),
        }
    }

    pub fn build_library(mut self, library: &Library) -> LibraryData {
        let name = library.name().unwrap_or_default();

        library.raw().node().child_by_field_id(FieldKind::NAME.into()).map(|n| self.lower_expr(n));

        LibraryData {
            name,
            expr_store: std::mem::take(&mut self.expr_store),
        }
    }
}