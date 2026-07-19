
use la_arena::{Arena, Idx};
use rustc_hash::FxHashMap;
use tree_sitter::Node;

use crate::ast::kinds::{FieldKind, NodeKind};
use crate::ast::{AstNode, Enum, Error, Event, Function, Modifier, NodeRange, Struct, ToAstNode, HasName, Var};
use crate::hir::body_map::{ByteOffset, Local, LocalId, Location, SemanticId, VariableKind};
use crate::hir::exprs::{Expr, ExprBuilder, ExprId, Name};
use crate::hir::types::{TypeBuilder, TypeId, TypeName, Visibility};

#[derive(PartialEq, Eq)]
pub struct ExprStore {
    pub types: Arena<TypeName>,
    pub exprs: Arena<Expr>,
    pub range_to_id: FxHashMap<NodeRange, SemanticId>
}

impl Default for ExprStore {
    fn default() -> Self {
        Self {
            types: Arena::new(),
            exprs: Arena::new(),
            range_to_id: FxHashMap::default(),
        }
    }
}

#[derive(PartialEq, Eq)]
pub struct VarData {
    pub name: Name,
    pub type_name: TypeId,
    pub vis: Visibility,
    pub kind: VariableKind,
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
        self.expr_store.range_to_id.insert(NodeRange::from(&node), SemanticId::Expr(expr_id));
        expr_id
    }

    fn alloc_member_expr(&mut self, member: Expr, range: NodeRange, node: Node) -> ExprId {
        let mem_id = self.alloc_expr(member, node);
        self.expr_store.range_to_id.insert(range, SemanticId::Member(mem_id));
        mem_id
    }
}

impl TypeBuilder for ItemBuilder {
    fn alloc_type(&mut self, ty: TypeName, node: Node) -> TypeId {
        let seg_count = ty.seg_count();
        let ty_id = self.expr_store.types.alloc(ty);
        let ptr = NodeRange::from(&node);
        self.expr_store.range_to_id.insert(ptr, SemanticId::Type(ty_id));
        if seg_count > 1 {
            self.alloc_segments(node, ty_id);
        }
        ty_id
    }

    fn alloc_segments(&mut self, node: Node, ty: TypeId) {
        for (seg, child) in node.named_children(&mut node.walk()).filter(|n| n.kind_id() == NodeKind::IDENTIFIER).enumerate() {
            self.expr_store.range_to_id.insert(NodeRange::from(&child), SemanticId::TypeSegment { ty, segment: seg as u8 });
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
    ///                                      VARIABLE BUILDER                                             ///
    ////////////////////////////////////////////////////////////////////////////////////////////////////////
    
    pub fn build_var(&mut self, var: &Var) -> VarData {
        let node = var.raw().node();
        let mut name = Name::default();
        let mut vis = Visibility::default();
        let mut kind = VariableKind::State;
        let mut type_name = None;
        let mut range = None;
        for child in node.children(&mut node.walk()) {
            match child.kind_id().into() {
                NodeKind::TYPE_NAME => type_name = self.lower_type(child),
                NodeKind::CONSTANT => kind = VariableKind::Const,
                NodeKind::IMMUTABLE => kind = VariableKind::Immutable,
                NodeKind::VISIBILITY => vis = Visibility::parse(self.root().text_by_range(child.byte_range()).trim()),
                NodeKind::IDENTIFIER => {
                    name = self.root.text_by_range(child.byte_range()).trim().into();
                    // we use the identifier range for the declaration
                    range = Some(NodeRange::from(&child));
                }
                NodeKind::EXPRESSION => { self.lower_expr(child); },
                _ => {}
            }
        }
        self.expr_store.range_to_id.insert(range.unwrap(), SemanticId::Name);
        VarData {
            name,
            type_name: type_name.unwrap(),
            vis,
            kind,
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

                        self.expr_store.range_to_id.insert(NodeRange::from(&member), SemanticId::Field(field_id));
                        self.expr_store.range_to_id.insert(range, SemanticId::Field(field_id));
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

        if let Some(body) = node.child_by_field_id(FieldKind::BODY.into()) {
            for value in body.named_children(&mut body.walk()) {
                if value.kind_id() == NodeKind::ENUM_VALUE {
                    let name = self.root().text_by_range(value.byte_range()).trim().into();

                    let variant_id = variants.alloc(Variant {
                        name,
                    });
                    self.expr_store.range_to_id.insert(NodeRange::from(&value), SemanticId::Variant(variant_id));
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
                _ => {}
            }
        }

        FunctionData {
            name,
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
        ));

        self.expr_store.range_to_id.insert(NodeRange::from(&node), SemanticId::Local(local_id));

        if let Some(range) = name_range {
            self.expr_store.range_to_id.insert(range, SemanticId::Local(local_id));
        }
        Some(local_id)
    }
}