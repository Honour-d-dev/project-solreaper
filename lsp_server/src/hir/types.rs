use std::{fmt, format};

use la_arena::{Arena, Idx};
use smallvec::SmallVec;
use tree_sitter::Node;
use crate::ast::kinds::{FieldKind, NodeKind};
use crate::ir::def_map::DefId;
use crate::hir::exprs::{ExprBuilder, ExprId, Name};

pub type TypeId = Idx<TypeName>;

// type FnTypeName = Fn<TypeId>;

#[derive(PartialEq, Eq)]
pub enum Type {
    Primitive(Primitive),
    UserDefined(DefId),
    Array{ty: Box<Type>, size: Option<usize>},
    Mapping {
        key: Box<Type>,
        value: Box<Type>
    },
    Fn(Fn<Type>),//resolves to return type
    Tuple(Box<[Type]>),
}

impl Type {
    /// Returns the cost of implicitly converting `self` to `target`.
    /// `None` means no implicit conversion is possible.
    /// Cost 0 = identical, higher = more lossy/risky.
    pub fn converts_to(&self, target: &Type) -> Option<u8> {
        match (self, target) {
            (Type::Primitive(a), Type::Primitive(b)) => a.converts_to(b),
            (Type::UserDefined(a), Type::UserDefined(b)) if a == b => Some(0),
            (Type::Array { ty: a, size: sa }, Type::Array { ty: b, size: sb }) => {
                let cost = a.converts_to(b)?;
                if sa == sb { Some(cost) } else { None }
            }
            (Type::Mapping { key: ka, value: va }, Type::Mapping { key: kb, value: vb }) => {
                let kc = ka.converts_to(kb)?;
                let vc = va.converts_to(vb)?;
                Some(kc.saturating_add(vc))
            }
            (Type::Fn(a), Type::Fn(b)) => {
                if a.params.len() != b.params.len() || a.ret.len() != b.ret.len() {
                    return None;
                }
                let mut cost = 0u8;
                for (ap, bp) in a.params.iter().zip(b.params.iter()) {
                    cost = cost.saturating_add(ap.converts_to(bp)?);
                }
                for (ar, br) in a.ret.iter().zip(b.ret.iter()) {
                    cost = cost.saturating_add(ar.converts_to(br)?);
                }
                Some(cost)
            }
            (Type::Tuple(a), Type::Tuple(b)) => {
                if a.len() != b.len() {
                    return None;
                }
                let mut cost = 0u8;
                for (x, y) in a.iter().zip(b.iter()) {
                    cost = cost.saturating_add(x.converts_to(y)?);
                }
                Some(cost)
            }
            _ => None,
        }
    }
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
    Fn(Fn<TypeId>),//function pointers
}

impl TypeName {
    #[inline]
    pub fn seg_count(&self) -> usize {
        match self {
            TypeName::UserDefined(path) => path.segments.len(),
            _ => 0,
        }
    }

    pub fn to_string(&self, arena: &Arena<TypeName>) -> String {
        match self {
            TypeName::Primitive(p) => p.to_string(),
            TypeName::UserDefined(path) => path
                .segments
                .iter()
                .map(|s| s.as_str())
                .collect::<Vec<_>>()
                .join("."),
            TypeName::Array { ty, size } => {
                let base = arena[*ty].to_string(arena);
                match size {
                    Some(_) => format!("{}[_]", base),
                    None => format!("{}[]", base),
                }
            }
            TypeName::Mapping { key, value } => {
                format!(
                    "mapping({} => {})",
                    arena[*key].to_string(arena),
                    arena[*value].to_string(arena)
                )
            }
            TypeName::Fn(f) => {
                let params = f
                    .params
                    .iter()
                    .map(|&p| arena[p].to_string(arena))
                    .collect::<Vec<_>>()
                    .join(", ");
                let ret = f
                    .ret
                    .iter()
                    .map(|&r| arena[r].to_string(arena))
                    .collect::<Vec<_>>()
                    .join(", ");
                let mut s = format!("function({})", params);
                if f.vis != Visibility::Internal {
                    s.push(' ');
                    s.push_str(f.vis.as_str());
                }
                if f.mutability != Mutability::NonPayable {
                    s.push(' ');
                    s.push_str(f.mutability.as_str());
                }
                if !f.ret.is_empty() {
                    s.push_str(&format!(" returns ({})", ret));
                }
                s
            }
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

impl fmt::Display for Primitive {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Primitive::Address => write!(f, "address"),
            Primitive::Boolean => write!(f, "bool"),
            Primitive::Bytes => write!(f, "bytes"),
            Primitive::String => write!(f, "string"),
            Primitive::Uint(n) => write!(f, "uint{n}"),
            Primitive::Int(n) => write!(f, "int{n}"),
            Primitive::FixedBytes(n) => write!(f, "bytes{n}"),
            Primitive::Unknown => write!(f, "unknown"),
        }
    }
}

impl Primitive {
    /// Returns the cost of implicitly converting `self` to `target`.
    /// `None` means no implicit conversion is possible.
    pub fn converts_to(&self, target: &Primitive) -> Option<u8> {
        match (self, target) {
            (a, b) if a == b => Some(0),
            // uint widening: uintN -> uintM where M >= N
            (Primitive::Uint(n), Primitive::Uint(m)) if *m >= *n => Some(1),
            // int widening: intN -> intM where M >= N
            (Primitive::Int(n), Primitive::Int(m)) if *m >= *n => Some(1),
            // int -> uint: only intN -> uintM where M > N (sign bit needs room)
            (Primitive::Int(n), Primitive::Uint(m)) if *m > *n => Some(2),
            // fixed bytes widening: bytesN -> bytesM where M >= N
            (Primitive::FixedBytes(n), Primitive::FixedBytes(m)) if *m >= *n => Some(1),
            // address <-> bytes20
            (Primitive::Address, Primitive::FixedBytes(20)) => Some(1),
            (Primitive::FixedBytes(20), Primitive::Address) => Some(1),
            // bytes1 -> bytes (dynamic)
            (Primitive::FixedBytes(n), Primitive::Bytes) if *n <= 32 => Some(1),
            // string literals -> bytes/string
            (Primitive::String, Primitive::Bytes) => Some(1),
            (Primitive::Bytes, Primitive::String) => Some(1),
            _ => None,
        }
    }

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

#[derive(PartialEq, Eq)]
pub struct Fn<T> {
    pub vis: Visibility,
    pub mutability: Mutability,
    pub params: Box<[T]>,
    pub ret: Box<[T]>,
}

impl<T> Default for Fn<T> {
    fn default() -> Self {
        Self {
            vis: Visibility::default(),
            mutability: Mutability::default(),
            params: Box::default(),
            ret: Box::default(),
        }
    }
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
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

    pub fn as_str(&self) -> &'static str {
        match self {
            Visibility::Internal => "internal",
            Visibility::Public => "public",
            Visibility::Private => "private",
            Visibility::External => "external",
        }
    }

}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
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

    pub fn as_str(&self) -> &'static str {
        match self {
            Mutability::NonPayable => "",
            Mutability::Payable => "payable",
            Mutability::View => "view",
            Mutability::Pure => "pure",
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
                                    size = self.lower_expr(child);
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
        let mut fn_ty = Fn::<TypeId>::default();

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