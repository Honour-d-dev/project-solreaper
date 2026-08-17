use std::{fmt, format};

use la_arena::{Arena, Idx};
use num_bigint::{BigInt, Sign};
use smallvec::SmallVec;
use smol_str::SmolStr;
use tree_sitter::Node;

use crate::ast::kinds::{FieldKind, NodeKind};
use crate::hir::body_map::{Location, VariableKind};
use crate::hir::exprs::{ExprBuilder, ExprId, Name};
use crate::ir::def_map::DefId;
use crate::salsa::HirDatabase;

pub type TypeId = Idx<TypeName>;

#[derive(PartialEq, Eq, Clone, Hash)]
pub enum LiteralType {
    Boolean(bool),
    Integer(BigInt),
    Rational { numerator: BigInt, denominator: BigInt },
    String(SmolStr),
    HexString(SmolStr),
}

impl LiteralType {
    fn fits_unsigned(value: &BigInt, bits: u16) -> bool {
        value.sign() != Sign::Minus
            && value.to_biguint().is_some_and(|value| value.bits() <= bits as u64)
    }

    fn fits_signed(value: &BigInt, bits: u16) -> bool {
        if bits == 0 {
            return false;
        }
        let one = BigInt::from(1u8);
        let min = -(&one << (bits - 1));
        let max = (&one << (bits - 1)) - &one;
        value >= &min && value <= &max
    }

    fn byte_len(text: &str, hex: bool) -> usize {
        let text = text.trim();
        if hex {
            text.trim_start_matches("hex")
                .trim_matches(|c| c == '"' || c == '\'')
                .chars()
                .filter(|c| c.is_ascii_hexdigit())
                .count() / 2
        } else {
            text.trim_matches(|c| c == '"' || c == '\'').as_bytes().len()
        }
    }

    fn integer_converts_to(value: &BigInt, target: &Type) -> Option<u8> {
        match target {
            Type::Primitive(Primitive::Uint(bits)) if Self::fits_unsigned(value, *bits) => Some(1),
            Type::Primitive(Primitive::Int(bits)) if Self::fits_signed(value, *bits) => Some(1),
            _ => None,
        }
    }

    pub fn converts_to(&self, target: &Type) -> Option<u8> {
        match self {
            Self::Boolean(_) => matches!(target, Type::Primitive(Primitive::Boolean)).then_some(0),
            Self::Integer(value) => Self::integer_converts_to(value, target),
            Self::Rational { numerator, denominator }
                if !denominator.eq(&BigInt::from(0u8)) && numerator % denominator == BigInt::from(0u8) =>
            {
                Self::integer_converts_to(&(numerator / denominator), target)
            }
            Self::Rational { .. } => None,
            Self::String(text) => match target {
                Type::Primitive(Primitive::String) => Some(0),
                Type::Primitive(Primitive::Bytes) => Some(1),
                Type::Primitive(Primitive::FixedBytes(bits))
                    if Self::byte_len(text, false) <= *bits as usize => Some(1),
                _ => None,
            },
            Self::HexString(text) => match target {
                Type::Primitive(Primitive::Bytes) => Some(1),
                Type::Primitive(Primitive::FixedBytes(bits))
                    if Self::byte_len(text, true) <= *bits as usize => Some(1),
                _ => None,
            },
        }
    }

    pub fn inferred_type(&self) -> Option<Type> {
        match self {
            Self::Boolean(_) => Some(Type::Primitive(Primitive::Boolean)),
            Self::Integer(value) => Self::smallest_integer_type(value),
            Self::Rational { numerator, denominator }
                if !denominator.eq(&BigInt::from(0u8)) && numerator % denominator == BigInt::from(0u8) =>
            {
                Self::smallest_integer_type(&(numerator / denominator))
            }
            Self::Rational { .. } => None,
            Self::String(_) => Some(Type::Primitive(Primitive::String)),
            Self::HexString(text) => {
                let len = Self::byte_len(text, true);
                (len > 0 && len <= 32)
                    .then(|| Type::Primitive(Primitive::FixedBytes(len as u8)))
                    .or_else(|| Some(Type::Primitive(Primitive::Bytes)))
            }
        }
    }

    fn smallest_integer_type(value: &BigInt) -> Option<Type> {
        let primitive = if value.sign() == Sign::Minus {
            (8..=256)
                .step_by(8)
                .map(Primitive::Int)
                .find(|ty| Self::fits_signed(value, match ty { Primitive::Int(bits) => *bits, _ => unreachable!() }))
        } else {
            (8..=256)
                .step_by(8)
                .map(Primitive::Uint)
                .find(|ty| Self::fits_unsigned(value, match ty { Primitive::Uint(bits) => *bits, _ => unreachable!() }))
        }?;
        Some(Type::Primitive(primitive))
    }

    fn display(&self) -> String {
        match self {
            Self::Boolean(value) => value.to_string(),
            Self::Integer(value) => value.to_string(),
            Self::Rational { numerator, denominator } => format!("{numerator}/{denominator}"),
            Self::String(value) => value.to_string(),
            Self::HexString(value) => value.to_string(),
        }
    }

    fn default_loc(&self) -> Location {
        match self {
            Self::String(_) | Self::HexString(_) => Location::Memory,
            _ => Location::Stack,
        }
    }
}

#[derive(PartialEq, Eq, Clone, Hash)]
pub enum Type {
    Primitive(Primitive),
    Literal(LiteralType),
    Def(DefId),
    Error,
    Array{ty: Box<Type>, size: Option<usize>},
    Mapping {
        key: Box<Type>,
        value: Box<Type>
    },
    Fn(Fn<Type>),//resolves to return type, when called
    Tuple(Box<[Type]>),// TODO: switch to Cow?
}

impl Type {
    /// Returns the cost of implicitly converting `self` to `target`.
    /// `None` means no implicit conversion is possible.
    /// Cost 0 = identical, higher = more lossy/risky.
    pub fn converts_to(&self, target: &Type, db: &dyn HirDatabase) -> Option<u8> {
        match (self, target) {
            (Type::Literal(literal), target) => literal.converts_to(target),
            (Type::Primitive(a), Type::Primitive(b)) => a.converts_to(b),
            (Type::Def(a), Type::Def(b)) => Self::def_conversion(a, b, db),
            (Type::Error, Type::Error) => Some(0),
            (Type::Array { ty: a, size: sa }, Type::Array { ty: b, size: sb }) => {
                let cost = a.converts_to(b, db)?;
                if sa == sb { Some(cost) } else { None }
            }
            (Type::Mapping { key: ka, value: va }, Type::Mapping { key: kb, value: vb }) => {
                let kc = ka.converts_to(kb, db)?;
                let vc = va.converts_to(vb, db)?;
                Some(kc.saturating_add(vc))
            }
            (Type::Fn(a), Type::Fn(b)) => {
                if a.params.len() != b.params.len() || a.ret.len() != b.ret.len() {
                    return None;
                }
                let mut cost = 0u8;
                for (ap, bp) in a.params.iter().zip(b.params.iter()) {
                    cost = cost.saturating_add(ap.converts_to(bp, db)?);
                }
                for (ar, br) in a.ret.iter().zip(b.ret.iter()) {
                    cost = cost.saturating_add(ar.converts_to(br,db)?);
                }
                Some(cost)
            }
            (Type::Tuple(a), Type::Tuple(b)) => {
                if a.len() != b.len() {
                    return None;
                }
                let mut cost = 0u8;
                for (x, y) in a.iter().zip(b.iter()) {
                    cost = cost.saturating_add(x.converts_to(y,db)?);
                }
                Some(cost)
            }
            _ => None,
        }
    }

    fn def_conversion(a: &DefId, b: &DefId, db: &dyn HirDatabase) -> Option<u8> {
        (a == b).then_some(0).or_else(|| {
            match a {
                DefId::Contract(_) | DefId::Interface(_) => {
                    db.bases(*a).into_iter().find_map(|base| (base == *b).then_some(0))
                }
                _ => None,
            }
        })
    }

    pub fn implicitly_converts(&self, target: &Type, db: &dyn HirDatabase) -> bool {
        self.converts_to(target, db).is_some_and(|c| c == 0)
    }

    pub fn def_id(&self) -> Option<DefId> {
        match self {
            Type::Def(d) => Some(*d),
            _ => None,
        }
    }

    pub fn display(&self, db: &dyn HirDatabase) -> String {
        match self {
            Type::Primitive(p) => p.to_string(),
            Type::Literal(literal) => literal.display(),
            Type::Def(def) => Self::def_name(db, *def).unwrap_or_else(|| "<unknown>".into()),
            Type::Error => "error".into(),
            Type::Array { ty, size: Some(n) } => format!("{}[{n}]", ty.display(db)),
            Type::Array { ty, size: None } => format!("{}[]", ty.display(db)),
            Type::Mapping { key, value } => format!("mapping({} => {})", key.display(db), value.display(db)),
            Type::Fn(_) => "function".into(),
            Type::Tuple(types) => {
                let parts: Vec<String> = types.iter().map(|t| t.display(db)).collect();
                format!("({})", parts.join(", "))
            }
        }
    }

    fn def_name(db: &dyn HirDatabase, def: DefId) -> Option<String> {
        match def {
            DefId::Contract(id) => Some(db.contract_data(id).name.to_string()),
            DefId::Struct(id) => Some(db.struct_data(id).name.to_string()),
            DefId::Enum(id) => Some(db.enum_data(id).name.to_string()),
            DefId::Interface(id) => Some(db.interface_data(id).name.to_string()),
            DefId::Library(id) => Some(db.library_data(id).name.to_string()),
            DefId::Udvt(id) => Some(db.udvt_data(id).name.to_string()),
            DefId::Function(id) => Some(db.function_data(id).name.to_string()),
            DefId::Event(id) => Some(db.event_data(id).name.to_string()),
            DefId::Error(id) => Some(db.error_data(id).name.to_string()),
            DefId::Var(id) => Some(db.var_data(id).name.to_string()),
            DefId::Modifier(id) => Some(db.modifier_data(id).name.to_string()),
            DefId::File(_) | DefId::Import(_) | DefId::Using(_) => None,
        }
    }

    /// Cast types to their default locations.
    /// The locations of dynamic types can be changed upstream based on the location of their parent, if any
    pub fn upcast(self) -> TypeKey {
        let loc = match self {
            Type::Primitive(p) => p.default_loc(),
            Type::Literal(ref literal) => literal.default_loc(),
            Type::Def(d) => {
                match d {
                    DefId::Struct(_) => Location::Memory,//struct construction default memory
                    _ => Location::Stack,// contract/interface/library/enum/udvt
                }
            },
            Type::Fn(_) | Type::Error => Location::Stack,
            Type::Mapping { .. } => Location::Storage,//Mapping can only exist in storage
            Type::Array { .. } => Location::Memory,
            Type::Tuple(_) => Location::Memory,
        };
        TypeKey(self, loc)
    }

    /// Cast types to a specific location.
    /// All dynamic types inherit the parent location.
    pub fn upcast_from(self, loc: Location) -> TypeKey {
        match self.upcast() {
            tk @ TypeKey(_, Location::Stack) => tk,
            TypeKey(ty,_ ) => TypeKey(ty, loc) //all dynanic types inherit parent loc
        }
    }

    pub fn upcast_from_kind(self, kind: VariableKind) -> TypeKey {
        match kind {
            VariableKind::State => {
                self.upcast_from(Location::Storage)
            }
            _ => {
                self.upcast()
            }
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


#[derive(PartialEq, Eq, Clone, Hash)]
pub struct TypeKey(pub Type, pub Location);

impl TypeKey {
    pub fn typ(&self) -> &Type {
        &self.0
    }

    pub fn loc(&self) -> Location {
        self.1
    }

    pub fn as_typ(self) -> Type {
        self.0
    }

    pub fn def_id(&self) -> Option<DefId> {
        self.0.def_id()
    }

    pub fn converts_to(&self, target: &TypeKey, db: &dyn HirDatabase) -> Option<u8> {
        let cost = self.0.converts_to(&target.0, db)?;
        match (self.1, target.1) {
            (a, b) if a == b => Some(cost),
            (Location::Memory, Location::Calldata) => Some(cost),
            (Location::Storage, Location::Memory) => Some(cost),
            (Location::Storage, Location::Calldata) => Some(cost),
            _ => None,
        }
    }
}


#[derive(PartialEq, Eq)]
pub struct Path {
    pub segments: SmallVec<[Name;2]>
}

#[derive(Debug, PartialEq, Eq, Hash, Clone, Copy)]
pub enum Primitive {
    Address,
    AddressPayable,
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
            Primitive::AddressPayable => write!(f, "address payable"),
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
            (Primitive::Uint(n), Primitive::Uint(m)) if *m > *n => Some(1),
            (Primitive::Int(n), Primitive::Int(m)) if *m > *n => Some(1),
            (Primitive::Uint(n), Primitive::Int(m)) if *m > *n => Some(1),
            (Primitive::FixedBytes(n), Primitive::FixedBytes(m)) if *m > *n => Some(1),
            (Primitive::AddressPayable, Primitive::Address) => Some(1),
            _ => None,
        }
    }

    /// Returns the default data location for this primitive when no explicit location is declared.
    /// Value types (uint, int, bool, address, fixed bytes) live on the stack.
    /// Dynamic types (bytes, string) live in memory.
    #[inline]
    pub fn default_loc(&self) -> Location {
        match self {
            Primitive::Bytes | Primitive::String => Location::Memory,
            _ => Location::Stack,
        }
    }

    /// All primitive types commonly used in Solidity, for completion.
    /// `Uint`/`Int` cover 8–256 in steps of 8; `FixedBytes` covers 1–32.
    pub fn all_primitives() -> Vec<Primitive> {
        let mut acc = vec![
            Primitive::Boolean,
            Primitive::Address,
            Primitive::AddressPayable,
            Primitive::String,
            Primitive::Bytes,
        ];
        // uint8 .. uint256
        for size in (8..=256).step_by(8) {
            acc.push(Primitive::Uint(size));
        }
        // int8 .. int256
        for size in (8..=256).step_by(8) {
            acc.push(Primitive::Int(size));
        }
        // bytes1 .. bytes32
        for size in 1..=32u8 {
            acc.push(Primitive::FixedBytes(size));
        }
        acc
    }

    #[inline]
    pub fn parse(ty: &str) -> Primitive {
        match ty {
            "address" => Primitive::Address,
            "address payable" => Primitive::AddressPayable,
            "bool" => Primitive::Boolean,
            "bytes" => Primitive::Bytes,
            "string" => Primitive::String,
            "uint" => Primitive::Uint(256),
            "int" => Primitive::Int(256),
            s if s.strip_prefix("uint")
                .and_then(|size| size.parse::<u16>().ok())
                .is_some_and(|size| (8..=256).contains(&size) && size % 8 == 0) =>
            {
                Primitive::Uint(s[4..].parse().unwrap())
            }
            s if s.strip_prefix("int")
                .and_then(|size| size.parse::<u16>().ok())
                .is_some_and(|size| (8..=256).contains(&size) && size % 8 == 0) =>
            {
                Primitive::Int(s[3..].parse().unwrap())
            }
            s if s.strip_prefix("bytes")
                .and_then(|size| size.parse::<u8>().ok())
                .is_some_and(|size| (1..=32).contains(&size)) =>
            {
                Primitive::FixedBytes(s[5..].parse().unwrap())
            }
            _ => Primitive::Unknown,
        }

    }
}

#[derive(PartialEq, Eq, Hash, Clone)]
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

#[derive(Default, PartialEq, Eq, Clone, Copy, Hash)]
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

#[derive(Default, PartialEq, Eq, Clone, Copy, Hash)]
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
                        let key = node.child_by_field_id(FieldKind::KEY_TYPE.into()).and_then(|k| self.lower_type(k));

                        let value = node.child_by_field_id(FieldKind::VALUE_TYPE.into()).and_then(|v| self.lower_type(v) );

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

#[cfg(test)]
mod tests {
    use super::*;

    fn primitive(value: Primitive) -> Type {
        Type::Primitive(value)
    }

    #[test]
    fn primitive_implicit_conversions_match_solidity_directions() {
        assert_eq!(Primitive::Uint(8).converts_to(&Primitive::Uint(16)), Some(1));
        assert_eq!(Primitive::Uint(16).converts_to(&Primitive::Uint(8)), None);
        assert_eq!(Primitive::Int(8).converts_to(&Primitive::Int(16)), Some(1));
        assert_eq!(Primitive::Uint(8).converts_to(&Primitive::Int(16)), Some(1));
        assert_eq!(Primitive::Int(8).converts_to(&Primitive::Uint(16)), None);
        assert_eq!(Primitive::FixedBytes(4).converts_to(&Primitive::FixedBytes(8)), Some(1));
        assert_eq!(Primitive::AddressPayable.converts_to(&Primitive::Address), Some(1));
        assert_eq!(Primitive::Address.converts_to(&Primitive::AddressPayable), None);
        assert_eq!(Primitive::Address.converts_to(&Primitive::FixedBytes(20)), None);
        assert_eq!(Primitive::FixedBytes(20).converts_to(&Primitive::Address), None);
        assert_eq!(Primitive::FixedBytes(4).converts_to(&Primitive::Bytes), None);
        assert_eq!(Primitive::String.converts_to(&Primitive::Bytes), None);
        assert_eq!(Primitive::Bytes.converts_to(&Primitive::String), None);
        assert_eq!(Primitive::parse("address payable"), Primitive::AddressPayable);
        assert_eq!(Primitive::parse("uint"), Primitive::Uint(256));
        assert_eq!(Primitive::parse("uint7"), Primitive::Unknown);
        assert_eq!(Primitive::parse("bytes33"), Primitive::Unknown);
    }

    #[test]
    fn integer_literals_are_value_sensitive() {
        let small = LiteralType::Integer(BigInt::from(255u16));
        let large = LiteralType::Integer(BigInt::from(256u16));
        let negative = LiteralType::Integer(BigInt::from(-1i8));

        assert!(small.converts_to(&primitive(Primitive::Uint(8))).is_some());
        assert!(large.converts_to(&primitive(Primitive::Uint(8))).is_none());
        assert!(negative.converts_to(&primitive(Primitive::Int(8))).is_some());
        assert!(negative.converts_to(&primitive(Primitive::Uint(8))).is_none());
    }

    #[test]
    fn rational_literals_only_convert_when_integral() {
        let integral = LiteralType::Rational {
            numerator: BigInt::from(6u8),
            denominator: BigInt::from(3u8),
        };
        let fractional = LiteralType::Rational {
            numerator: BigInt::from(3u8),
            denominator: BigInt::from(2u8),
        };

        assert!(integral.converts_to(&primitive(Primitive::Uint(8))).is_some());
        assert!(fractional.converts_to(&primitive(Primitive::Uint(8))).is_none());
    }

    #[test]
    fn string_literals_have_literal_only_byte_conversions() {
        let literal = LiteralType::String("abc".into());
        let hex_literal = LiteralType::HexString(r#"hex"abcd"#.into());

        assert!(literal.converts_to(&primitive(Primitive::String)).is_some());
        assert!(literal.converts_to(&primitive(Primitive::Bytes)).is_some());
        assert!(literal.converts_to(&primitive(Primitive::FixedBytes(3))).is_some());
        assert!(literal.converts_to(&primitive(Primitive::FixedBytes(2))).is_none());
        assert!(hex_literal.converts_to(&primitive(Primitive::FixedBytes(2))).is_some());
        assert!(hex_literal.converts_to(&primitive(Primitive::FixedBytes(1))).is_none());
    }
}