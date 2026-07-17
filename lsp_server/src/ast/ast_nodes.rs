use smallvec::SmallVec;
use smol_str::SmolStr;
use crate::ast::ast::{AstNode, ToAstNode};
use crate::ast::kinds::{FieldKind, NodeKind};
use crate::hir::types::Path;

pub trait HasName: ToAstNode {
    fn name(&self) -> Option<SmolStr> {
        self.raw().node()
            .child_by_field_id(FieldKind::NAME.into())
            .map(|n| self.raw().text_by_range(n.byte_range()).into())
    }
}

pub trait HasBases: ToAstNode {
    fn bases(&self) -> Box<[Path]> {
        self.raw().node()
        .named_children(&mut self.raw().node().walk())
        .filter_map(|n| 
            if n.kind_id() == NodeKind::INHERITANCE_SPECIFIER {
                n.named_children(&mut n.walk())
                .find_map(|base| {
                    if base.kind_id() == NodeKind::USER_DEFINED_TYPE {
                        let segments = base.named_children(&mut base.walk())
                            .map(|ident| self.raw().text_by_range(ident.byte_range()).into())
                            .collect::<SmallVec<_>>();
                        Some(Path {segments})
                    } else {
                        None
                    }
                })
            } else {
                None
            }
        )
        .collect::<Box<_>>()
    }
}

pub trait HasMembers: ToAstNode {
    fn members(&self) -> Box<[Item]> {
        let mut members = Vec::new();
        let Some(body) = self.raw().node().child_by_field_id(FieldKind::BODY.into()) else {
            return members.into();
        };
        for node in body.children(&mut self.raw().node().walk()) {
            match NodeKind::from(node.kind_id()) {
                NodeKind::FUNCTION_DEFINITION => {
                    if let Some(func) = Function::cast(self.raw().upcast(node)) {
                        members.push(Item::Function(func));
                    }
                }
                NodeKind::EVENT_DEFINITION => {
                    if let Some(event) = Event::cast(self.raw().upcast(node)) {
                        members.push(Item::Event(event));
                    }
                }
                NodeKind::STRUCT_DEFINITION => {
                    if let Some(strukt) = Struct::cast(self.raw().upcast(node)) {
                        members.push(Item::Struct(strukt));
                    }
                }
                NodeKind::ERROR_DEFINITION => {
                    if let Some(error) = Error::cast(self.raw().upcast(node)) {
                        members.push(Item::Error(error));
                    }
                }
                NodeKind::MODIFIER_DEFINITION => {
                    if let Some(modifier) = Modifier::cast(self.raw().upcast(node)) {
                        members.push(Item::Modifier(modifier));
                    }
                }
                NodeKind::STATE_VAR_DECLARATION => {
                    if let Some(var) = Var::cast(self.raw().upcast(node)) {
                        members.push(Item::Var(var));
                    }
                }
                _ => {}
            }
        }
        members.into_boxed_slice()
    }
}

macro_rules! impl_to_ast_node {
    ($($type:ty, $($node_kind:ident)|+ $(, $trait:ty)*)+) => {
        $(
            impl ToAstNode for $type {
                
                #[inline]
                fn to_node(self) -> AstNode {
                    self.raw
                }
                
                #[inline]
                fn raw(&self) -> &AstNode {
                    &self.raw
                }
                
                fn cast(n: AstNode) -> Option<Self> {
                    if Self::can_cast(n.node().kind_id().into()) {
                        Some(Self{raw: n})
                    } else {
                        None
                    }
                }

                #[inline]
                fn can_cast(n: NodeKind) -> bool {
                    false $(|| n == NodeKind::$node_kind)+
                }
            }

            $(
                impl $trait for $type {}
            )*
        )*
    };
}

impl_to_ast_node!(
    SourceFile, SOURCE_FILE
    Import, IMPORT_DIRECTIVE
    Contract, CONTRACT_DEFINITION, HasName, HasBases, HasMembers
    Interface, INTERFACE_DEFINITION, HasName, HasBases, HasMembers
    Library, LIBRARY_DEFINITION, HasName, HasMembers
    Function, FUNCTION_DEFINITION, HasName
    Modifier, MODIFIER_DEFINITION, HasName
    Struct, STRUCT_DEFINITION, HasName
    Enum, ENUM_DEFINITION, HasName
    Event, EVENT_DEFINITION, HasName
    Error, ERROR_DEFINITION, HasName
    Var, STATE_VAR_DECLARATION | CONST_VAR_DECLARATION, HasName
);


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFile {
    raw: AstNode
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    raw: AstNode,
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    raw: AstNode,
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    raw: AstNode,
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Struct {
    raw: AstNode,
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enum {
    raw: AstNode,
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    raw: AstNode,
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Var {//Single wrapper for multiple var types
    raw: AstNode,
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    raw: AstNode,
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    raw: AstNode,
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Modifier {
    raw: AstNode,
}



#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    raw: AstNode,
}






#[derive(Debug,Default, Clone, PartialEq, Eq)]
pub(crate) enum ImportType {
    #[default]
    Full,//can i use full for namespace? namespace is just full with an alias
    Named {
        symbols: Vec<ImportItem>,
    },
    Namespace {
        alias: SmolStr
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportItem {
    pub name: SmolStr,
    pub alias: Option<SmolStr>,
}

impl Import {
    pub fn import_type(&self) -> ImportType {
        let mut symbols: Vec<ImportItem> = Vec::new();
        let mut is_alias = false;
        let mut alias = SmolStr::default();
        let mut name = SmolStr::default();
        for node in self.raw.node().children(&mut self.raw.node().walk()) {
            if node.kind_id() == NodeKind::IDENTIFIER {
                if is_alias {
                    alias = self.raw.text_by_range(node.byte_range()).into();
                    if !symbols.is_empty() {
                        symbols.last_mut().unwrap().alias = Some(alias.clone());
                    }
                    //consume alias flag
                    is_alias = false;
                } else {
                    name = self.raw.text_by_range(node.byte_range()).into();
                    symbols.push(ImportItem { name: name.clone(), alias: None });
                }
            }

            if node.kind_id() == NodeKind::AS {
                //prepare for incoming alias identifier
                is_alias = true;
            }
        }

        //FIXME: matching on the last name and symbol can be sketchy
        // for something like: import {* as X, y} from ./path Is this valid syntax??
        match (name, alias) {
            (name, alias) if !name.is_empty()  => ImportType::Named { symbols },
            (name, alias) if name.is_empty() && alias.is_empty() => ImportType::Full,
            (name, alias) if name.is_empty() && !alias.is_empty() => ImportType::Namespace { alias },
            _ => ImportType::Full,//never reached, match is logically exhaustive
        }
    }
    #[inline]
    pub fn path(&self) -> &str {
        self.raw.node().child_by_field_id(FieldKind::SOURCE.into()).map(|p| self.raw.text_by_range(p.byte_range())).unwrap_or("").trim_matches(['"', '\''])
    }
}


/// ast::Item is a unification of all ast kinds acting as a blanket typed representation(i.e generic while still maintaining type info)
/// as opposed to AstNode which is just generic with no underlying type info
/// so we go AstNode -> Specific Type -> Item

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    SourceFile(SourceFile),
    Contract(Contract),
    Interface(Interface),
    Library(Library),
    Struct(Struct),
    Enum(Enum),
    Function(Function),
    Event(Event),
    Error(Error),
    Modifier(Modifier),
    Import(Import),
    Var(Var),
}

impl ToAstNode for Item {
    fn to_node(self) -> AstNode {
        match self {
            Item::SourceFile(s) => s.to_node(),
            Item::Contract(c) => c.to_node(),
            Item::Interface(i) => i.to_node(),
            Item::Library(l) => l.to_node(),
            Item::Struct(s) => s.to_node(),
            Item::Enum(e) => e.to_node(),
            Item::Function(f) => f.to_node(),
            Item::Event(e) => e.to_node(),
            Item::Error(e) => e.to_node(),
            Item::Modifier(m) => m.to_node(),
            Item::Import(i) => i.to_node(),
            Item::Var(v) => v.to_node(),
        }
    }

    fn raw(&self) -> &AstNode {
        match self {
            Item::SourceFile(s) => s.raw(),
            Item::Contract(c) => c.raw(),
            Item::Interface(i) => i.raw(),
            Item::Library(l) => l.raw(),
            Item::Struct(s) => s.raw(),
            Item::Enum(e) => e.raw(),
            Item::Function(f) => f.raw(),
            Item::Event(e) => e.raw(),
            Item::Error(e) => e.raw(),
            Item::Modifier(m) => m.raw(),
            Item::Import(i) => i.raw(),
            Item::Var(v) => v.raw(),
        }
    }

    fn cast(node: AstNode) -> Option<Self> {
        match NodeKind::from(node.node().kind_id()) {
            NodeKind::SOURCE_FILE => Some(Self::SourceFile(SourceFile::cast(node).unwrap())),
            NodeKind::CONTRACT_DEFINITION => Some(Self::Contract(Contract::cast(node).unwrap())),
            NodeKind::INTERFACE_DEFINITION => Some(Self::Interface(Interface::cast(node).unwrap())),
            NodeKind::LIBRARY_DEFINITION => Some(Self::Library(Library::cast(node).unwrap())),
            NodeKind::STRUCT_DEFINITION => Some(Self::Struct(Struct::cast(node).unwrap())),
            NodeKind::ENUM_DEFINITION => Some(Self::Enum(Enum::cast(node).unwrap())),
            NodeKind::FUNCTION_DEFINITION => Some(Self::Function(Function::cast(node).unwrap())),
            NodeKind::EVENT_DEFINITION => Some(Self::Event(Event::cast(node).unwrap())),
            NodeKind::ERROR_DEFINITION => Some(Self::Error(Error::cast(node).unwrap())),
            NodeKind::MODIFIER_DEFINITION => Some(Self::Modifier(Modifier::cast(node).unwrap())),
            NodeKind::IMPORT_DIRECTIVE => Some(Self::Import(Import::cast(node).unwrap())),
            NodeKind::STATE_VAR_DECLARATION | NodeKind::CONST_VAR_DECLARATION => Some(Self::Var(Var::cast(node).unwrap())),
            _ => None,
        }
    }

    fn can_cast(n: NodeKind) -> bool {
        matches!(
            n,
                NodeKind::SOURCE_FILE
                | NodeKind::CONTRACT_DEFINITION
                | NodeKind::INTERFACE_DEFINITION
                | NodeKind::LIBRARY_DEFINITION
                | NodeKind::STRUCT_DEFINITION
                | NodeKind::ENUM_DEFINITION
                | NodeKind::FUNCTION_DEFINITION
                | NodeKind::EVENT_DEFINITION
                | NodeKind::ERROR_DEFINITION
                | NodeKind::MODIFIER_DEFINITION
                | NodeKind::IMPORT_DIRECTIVE
                | NodeKind::STATE_VAR_DECLARATION
                | NodeKind::CONST_VAR_DECLARATION
        )
    }
}