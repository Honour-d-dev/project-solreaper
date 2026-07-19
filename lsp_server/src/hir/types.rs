use la_arena::Idx;
use smallvec::SmallVec;
use tree_sitter::Node;
use crate::ast::kinds::{FieldKind, NodeKind};
use crate::ir::def_map::DefId;
use crate::hir::exprs::{ExprBuilder, ExprId, Name};

pub type TypeId = Idx<TypeName>;

#[derive(PartialEq, Eq)]
pub enum Type {
    Primitive(Primitive),
    UserDefined(DefId),
    Array(Box<Type>),
    Mapping {
        key: Box<Type>,
        value: Box<Type>
    },
    Fn{
        params: Box<[Type]>,
        ret: Box<[Type]>
    },//resolves to return type
}

#[derive(PartialEq, Eq)]
pub enum TypeName {
    Primitive(Primitive),
    UserDefined(Path),
    Array{
        ty: TypeId,
        size: Option<ExprId>
    },
    Mapping {
        key: TypeId,
        value: TypeId
    },
    Fn(FnType),//function pointers
}

impl TypeName {
    #[inline]
    pub fn seg_count(&self) -> usize {
        match self {
            TypeName::UserDefined(path) => path.segments.len(),
            _ => 0,
        }
    }
}

#[derive(PartialEq, Eq)]
pub struct Path {
    pub segments: SmallVec<[Name;2]>
}

#[derive(PartialEq, Eq, Clone, Copy)]
pub enum Primitive {
    Address,
    Boolean,
    Bytes,
    String,
    Uint(u16),
    Int(u16),
    FixedBytes(u8),
    Unknown
}

impl Primitive {
    #[inline]
    pub fn parse(ty: &str) -> Primitive {
        match ty {
            "address" | "address payable" => Primitive::Address,
            "bool" => Primitive::Boolean,
            "bytes" => Primitive::Bytes,
            "string" => Primitive::String,
            s if s.starts_with("uint") => {
                let size = s[4..].parse::<u16>().unwrap_or(256);
                Primitive::Uint(size.min(256))
            }
            s if s.starts_with("int") => {
                let size = s[3..].parse::<u16>().unwrap_or(256);
                Primitive::Int(size.min(256))
            }
            s if s.starts_with("bytes") => {
                let size = s[5..].parse::<u8>().unwrap_or(32);
                Primitive::FixedBytes(size.min(32))
            }
            _ => Primitive::Unknown,
        }

    }
}

#[derive(Default, PartialEq, Eq)]
pub struct FnType {
    pub vis: Visibility,
    pub mutability: Mutability,
    pub params: Box<[TypeId]>,
    pub ret: Box<[TypeId]>,
}

#[derive(Default, PartialEq, Eq)]
pub enum Visibility {
    #[default]
    Internal,
    Public,
    Private,
    External,
}

impl Visibility {
    #[inline]
    pub fn parse(s: &str) -> Visibility {
        match s {
            "public" => Visibility::Public,
            "private" => Visibility::Private,
            "external" => Visibility::External,
            _ => Visibility::Internal,
        }
    }
}

#[derive(Default, PartialEq, Eq)]
pub enum Mutability {
    #[default]
    NonPayable,
    Payable,
    View,
    Pure,
}

impl Mutability {
    #[inline]
    pub fn parse(s: &str) -> Mutability {
        match s {
            "payable" => Mutability::Payable,
            "view" => Mutability::View,
            "pure" => Mutability::Pure,
            _ => Mutability::NonPayable,
        }
    }
}

pub enum TypeShape {
    Function,
    Mapping,
    Array,
    Basic
}


pub trait TypeBuilder: ExprBuilder {
    fn alloc_segments(&mut self, node: Node, ty: TypeId);
    fn alloc_type(&mut self, ty: TypeName, node: Node) -> TypeId;

    fn lower_type(&mut self, node: Node) -> Option<TypeId> {
        match node.kind_id().into() {
            NodeKind::PRIMITIVE_TYPE => {
                let type_str = self.root().text_by_range(node.byte_range()).trim();
                let ty = Primitive::parse(type_str);
                return Some(self.alloc_type(TypeName::Primitive(ty), node));
            }
            NodeKind::USER_DEFINED_TYPE => {
                let segments: SmallVec<[Name; 2]> = node
                    .named_children(&mut node.walk())
                    .map(|ident| self.root().text_by_range(ident.byte_range()).into())
                    .collect::<SmallVec<_>>();

                return Some(self.alloc_type(TypeName::UserDefined(Path { segments }), node));
            }
            NodeKind::TYPE_NAME => {
                match self.type_shape(node) {
                    TypeShape::Function => {
                        return Some(self.lower_fn_type(node));
                    }
                    TypeShape::Mapping => {
                        // TODO: add identifier lowering: mainly for state lvl mapping decl though
                        let mut key = None;
                        let mut value = None;
                        if let Some(key_ty) = node.child_by_field_id(FieldKind::KEY_TYPE.into()) {
                            key = self.lower_type(key_ty);
                        }

                        if let Some(value_ty) = node.child_by_field_id(FieldKind::VALUE_TYPE.into()) {
                            value = self.lower_type(value_ty);
                        }

                        if let (Some(key), Some(value)) = (key,value) {
                            return Some(self.alloc_type(TypeName::Mapping { key, value }, node));
                        }
                        None
                    }
                    TypeShape::Array => {
                        let mut base = None;
                        let mut size = None;

                        for child in node.named_children(&mut node.walk()) {
                            match child.kind_id().into() {
                                NodeKind::TYPE_NAME => {
                                    base = self.lower_type(child);
                                }
                                NodeKind::EXPRESSION => {
                                    size = self.walk_expr(child);
                                }
                                _ => {}
                            }
                        }

                        if let Some(ty) = base {
                            return Some(self.alloc_type(TypeName::Array { ty, size }, node));
                        }
                        None
                    }
                    TypeShape::Basic => {
                        for child in node.named_children(&mut node.walk()) {//Fix: we return on the first match. can there be more??
                            if matches!( child.kind_id().into(), NodeKind::PRIMITIVE_TYPE | NodeKind::USER_DEFINED_TYPE | NodeKind::TYPE_NAME) {
                                return self.lower_type(child);
                            }
                        }
                        None
                    }
                }
            }
            _ => {None}
        }
    }

    fn lower_fn_type(&mut self, node: Node) -> TypeId {
        let mut params = Vec::new();
        let mut ret = Vec::new();
        let mut fn_ty = FnType::default();

        for child in node.named_children(&mut node.walk()) {
            match child.kind_id().into() {
                NodeKind::PARAMETER => {
                    if let Some(param_ty) = child.child_by_field_id(FieldKind::TYPE.into()) {
                        if let Some(ty) = self.lower_type(param_ty) {
                            params.push(ty);
                        }
                    }
                }
                NodeKind::RETURN_PARAMETER => {
                    if let Some(ret_ty) = child.child_by_field_id(FieldKind::TYPE.into()) {
                        if let Some(ty) = self.lower_type(ret_ty) {
                            ret.push(ty);
                        }
                    }
                }
                NodeKind::VISIBILITY => {//I Think public/private are not allowed for fn poiinters
                    let vis_text = self.root().text_by_range(child.byte_range()).trim();
                    fn_ty.vis = Visibility::parse(vis_text);
                }
                NodeKind::STATE_MUTABILITY => {
                    let mut_text = self.root().text_by_range(child.byte_range()).trim();
                    fn_ty.mutability = Mutability::parse(mut_text);
                }
                _ => {}
            }
        }
        
        fn_ty.params = params.into_boxed_slice();
        fn_ty.ret = ret.into_boxed_slice();

        self.alloc_type(TypeName::Fn(fn_ty), node)
    }


    fn type_shape(&self, node: Node) -> TypeShape {
        let s = self.root().text_by_range(node.byte_range()).trim();
        if s.ends_with(']') { return TypeShape::Array; }
        let prefix = s
            .split(['(', ' '])
            .next()
            .unwrap_or_default().trim();
        if prefix == "function" { return TypeShape::Function; }
        if prefix == "mapping" { return TypeShape::Mapping; }
        TypeShape::Basic
    }
}