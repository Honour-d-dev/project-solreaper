use smol_str::SmolStr;
use tree_sitter::Node;
use crate::{
    ast::{
        ast::{AstNode, ToAstNode},
        kinds::{
            NodeKind,
            FieldKind,
        },
    },
};



////////////////////////////////////////////////////
///              CONTRACT                       ///
//////////////////////////////////////////////////
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Contract {
    raw: AstNode,
}

impl Contract {
    #[inline]
    pub fn name(&self) -> Option<SmolStr> {
        self.raw.node()
            .child_by_field_id(FieldKind::NAME.into())
            .map(|n| self.raw.text_by_range(n.byte_range()).into())
    }

    pub fn bases(&self) -> Box<[SmolStr]> {
        self.raw.node()
        .named_children(&mut self.raw.node().walk())
        .filter(|n| n.kind_id() == NodeKind::INHERITANCE_SPECIFIER)
        .filter_map(|inheritance| {
            inheritance.named_children(&mut inheritance.walk())
                .find(|base| base.kind_id() == NodeKind::USER_DEFINED_TYPE)
                .map(|base| SmolStr::new(self.raw.text_by_range(base.byte_range())))
        })
        .collect::<Box<_>>()
    }

    pub fn members(&self) -> Box<[Item]> {
        let mut members = Vec::new();
        let Some(body) = self.raw.node().child_by_field_id(FieldKind::BODY.into()) else {
            return members.into();
        };
        for node in body.children(&mut self.raw.node().walk()) {
            match NodeKind::from(node.kind_id()) {
                NodeKind::FUNCTION_DEFINITION => {
                    if let Some(func) = Function::cast(self.raw.make_ast(node)) {
                        members.push(Item::Function(func));
                    }
                }
                NodeKind::EVENT_DEFINITION => {
                    if let Some(event) = Event::cast(self.raw.make_ast(node)) {
                        members.push(Item::Event(event));
                    }
                }
                NodeKind::STRUCT_DEFINITION => {
                    if let Some(strukt) = Struct::cast(self.raw.make_ast(node)) {
                        members.push(Item::Struct(strukt));
                    }
                }
                NodeKind::ERROR_DEFINITION => {
                    if let Some(error) = Error::cast(self.raw.make_ast(node)) {
                        members.push(Item::Error(error));
                    }
                }
                NodeKind::MODIFIER_DEFINITION => {
                    if let Some(modifier) = Modifier::cast(self.raw.make_ast(node)) {
                        members.push(Item::Modifier(modifier));
                    }
                }
                NodeKind::STATE_VAR_DECLARATION => {
                    if let Some(var) = Var::cast(self.raw.make_ast(node)) {
                        members.push(Item::Var(var));
                    }
                }
                _ => {}
            }
        }
        members.into_boxed_slice()
    }


}

impl ToAstNode for Contract {
    fn ast_node(self) -> AstNode {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode {
        &self.raw
    }
    
    fn cast(node: AstNode) -> Option<Self> {
        if node.node().kind_id() == NodeKind::CONTRACT_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }

    fn can_cast(node: &Node) -> bool {
        node.kind_id() == NodeKind::CONTRACT_DEFINITION
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Interface {
    raw: AstNode,
}

impl Interface {
    #[inline]
    pub fn name(&self) -> Option<SmolStr> {
        self.raw.node()
            .child_by_field_id(FieldKind::NAME.into())
            .map(|n| self.raw.text_by_range(n.byte_range()).into())
    }

    pub fn bases(&self) -> Box<[SmolStr]> {
        self.raw.node()
        .named_children(&mut self.raw.node().walk())
        .filter(|n| n.kind_id() == NodeKind::INHERITANCE_SPECIFIER)
        .filter_map(|inheritance| {
            inheritance.named_children(&mut inheritance.walk())
                .find(|base| base.kind_id() == NodeKind::USER_DEFINED_TYPE)
                .map(|base| SmolStr::new(self.raw.text_by_range(base.byte_range())))
        })
        .collect::<Box<_>>()
    }

    pub fn members(&self) -> Box<[Item]> {
        let mut members = Vec::new();
        let Some(body) = self.raw.node().child_by_field_id(FieldKind::BODY.into()) else {
            return members.into();
        };
        for node in body.children(&mut self.raw.node().walk()) {
            match NodeKind::from(node.kind_id()) {
                NodeKind::FUNCTION_DEFINITION => {
                    if let Some(func) = Function::cast(self.raw.make_ast(node)) {
                        members.push(Item::Function(func));
                    }
                }
                NodeKind::EVENT_DEFINITION => {
                    if let Some(event) = Event::cast(self.raw.make_ast(node)) {
                        members.push(Item::Event(event));
                    }
                }
                NodeKind::STRUCT_DEFINITION => {
                    if let Some(strukt) = Struct::cast(self.raw.make_ast(node)) {
                        members.push(Item::Struct(strukt));
                    }
                }
                NodeKind::ERROR_DEFINITION => {
                    if let Some(error) = Error::cast(self.raw.make_ast(node)) {
                        members.push(Item::Error(error));
                    }
                }
                NodeKind::MODIFIER_DEFINITION => {
                    if let Some(modifier) = Modifier::cast(self.raw.make_ast(node)) {
                        members.push(Item::Modifier(modifier));
                    }
                }
                NodeKind::STATE_VAR_DECLARATION => {
                    if let Some(var) = Var::cast(self.raw.make_ast(node)) {
                        members.push(Item::Var(var));
                    }
                }
                _ => {}
            }
        }
        members.into_boxed_slice()
    }
}

impl ToAstNode for Interface {
    fn ast_node(self) -> AstNode {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode {
        &self.raw
    }

    fn cast(node: AstNode) -> Option<Self> {
        if node.node().kind_id() == NodeKind::INTERFACE_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }
    
    fn can_cast(node: &Node) -> bool {
        node.kind_id() == NodeKind::INTERFACE_DEFINITION
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Library {
    raw: AstNode,
}

impl Library {
    #[inline]
    pub fn name(&self) -> Option<SmolStr> {
        self.raw.node()
            .child_by_field_id(FieldKind::NAME.into())
            .map(|n| self.raw.text_by_range(n.byte_range()).into())
    }


    pub fn members(&self) -> Box<[Item]> {
        let mut members = Vec::new();
        let Some(body) = self.raw.node().child_by_field_id(FieldKind::BODY.into()) else {
            return members.into();
        };
        for node in body.children(&mut self.raw.node().walk()) {
            match NodeKind::from(node.kind_id()) {
                NodeKind::FUNCTION_DEFINITION => {
                    if let Some(func) = Function::cast(self.raw.make_ast(node)) {
                        members.push(Item::Function(func));
                    }
                }
                NodeKind::EVENT_DEFINITION => {
                    if let Some(event) = Event::cast(self.raw.make_ast(node)) {
                        members.push(Item::Event(event));
                    }
                }
                NodeKind::STRUCT_DEFINITION => {
                    if let Some(strukt) = Struct::cast(self.raw.make_ast(node)) {
                        members.push(Item::Struct(strukt));
                    }
                }
                NodeKind::ERROR_DEFINITION => {
                    if let Some(error) = Error::cast(self.raw.make_ast(node)) {
                        members.push(Item::Error(error));
                    }
                }
                NodeKind::MODIFIER_DEFINITION => {
                    if let Some(modifier) = Modifier::cast(self.raw.make_ast(node)) {
                        members.push(Item::Modifier(modifier));
                    }
                }
                NodeKind::STATE_VAR_DECLARATION => {
                    if let Some(var) = Var::cast(self.raw.make_ast(node)) {
                        members.push(Item::Var(var));
                    }
                }
                _ => {}
            }
        }
        members.into_boxed_slice()
    }
}

impl ToAstNode for Library {
    fn ast_node(self) -> AstNode {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode {
        &self.raw
    }

    fn cast(node: AstNode) -> Option<Self> {
        if node.node().kind_id() == NodeKind::LIBRARY_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }

    fn can_cast(node: &Node) -> bool {
        node.kind_id() == NodeKind::LIBRARY_DEFINITION
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Struct {
    raw: AstNode,
}

impl Struct {
    #[inline]
    pub fn name(&self) -> Option<SmolStr> {
        self.raw.node()
            .child_by_field_id(FieldKind::NAME.into())
            .map(|n| self.raw.text_by_range(n.byte_range()).into())
    }
}

impl ToAstNode for Struct {
    fn ast_node(self) -> AstNode {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode {
        &self.raw
    }

    fn cast(node: AstNode) -> Option<Self> {
        if node.node().kind_id() == NodeKind::STRUCT_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }

    fn can_cast(node: &Node) -> bool {
        node.kind_id() == NodeKind::STRUCT_DEFINITION
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Enum {
    raw: AstNode,
}

impl Enum {
    #[inline]
    pub fn name(&self) -> Option<SmolStr> {
        self.raw.node()
            .child_by_field_id(FieldKind::NAME.into())
            .map(|n| self.raw.text_by_range(n.byte_range()).into())
    }
}

impl ToAstNode for Enum {
    fn ast_node(self) -> AstNode {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode {
        &self.raw
    }

    fn cast(node: AstNode) -> Option<Self> {
        if node.node().kind_id() == NodeKind::ENUM_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }
    
    fn can_cast(node: &Node) -> bool {
        node.kind_id() == NodeKind::ENUM_DEFINITION
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    raw: AstNode,
}

impl Function {
    #[inline]
    pub fn name(&self) -> Option<SmolStr> {
        self.raw.node()
            .child_by_field_id(FieldKind::NAME.into())
            .map(|n| self.raw.text_by_range(n.byte_range()).into())
    }
}

impl ToAstNode for Function {
    fn ast_node(self) -> AstNode {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode {
        &self.raw
    }
    
    fn cast(node: AstNode) -> Option<Self> {
        if node.node().kind_id() == NodeKind::FUNCTION_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }

    fn can_cast(node: &Node) -> bool {
        node.kind_id() == NodeKind::FUNCTION_DEFINITION
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Var {//Single wrapper for multiple var types
    raw: AstNode,
}

impl Var {
    #[inline]
    pub fn name(&self) -> Option<SmolStr> {
        self.raw.node()
            .child_by_field_id(FieldKind::NAME.into())
            .map(|n| self.raw.text_by_range(n.byte_range()).into())
    }
}

impl ToAstNode for Var {
    fn ast_node(self) -> AstNode {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode {
        &self.raw
    }
    
    fn cast(node: AstNode) -> Option<Self> {
        if matches! (NodeKind::from(node.node().kind_id()), NodeKind::STATE_VAR_DECLARATION | NodeKind::VAR_DECLARATION_STATEMENT | NodeKind::CONST_VAR_DECLARATION) {
            Some(Self { raw: node })
        } else {
            None
        }
    }

    fn can_cast(node: &Node) -> bool {
        matches! (NodeKind::from(node.kind_id()), NodeKind::STATE_VAR_DECLARATION | NodeKind::VAR_DECLARATION_STATEMENT | NodeKind::CONST_VAR_DECLARATION)
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Event {
    raw: AstNode,
}

impl Event {
    #[inline]
    pub fn name(&self) -> Option<SmolStr> {
        self.raw.node()
            .child_by_field_id(FieldKind::NAME.into())
            .map(|n| self.raw.text_by_range(n.byte_range()).into())
    }
}

impl ToAstNode for Event {
    fn ast_node(self) -> AstNode {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode {
        &self.raw
    }
    
    fn cast(node: AstNode) -> Option<Self> {
        if node.node().kind_id() == NodeKind::EVENT_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }

    fn can_cast(node: &Node) -> bool {
        node.kind_id() == NodeKind::EVENT_DEFINITION
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    raw: AstNode,
}

impl Error {
    #[inline]
    pub fn name(&self) -> Option<SmolStr> {
        self.raw.node()
            .child_by_field_id(FieldKind::NAME.into())
            .map(|n| self.raw.text_by_range(n.byte_range()).into())
    }
}

impl ToAstNode for Error {
    fn ast_node(self) -> AstNode {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode {
        &self.raw
    }

    fn cast(node: AstNode) -> Option<Self> {
        if node.node().kind_id() == NodeKind::ERROR_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }
    
    fn can_cast(node: &Node) -> bool {
        node.kind_id() == NodeKind::ERROR_DEFINITION
    }
}


#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Modifier {
    raw: AstNode,
}


impl Modifier {
    #[inline]
    pub fn name(&self) -> Option<SmolStr> {
        self.raw.node()
            .child_by_field_id(FieldKind::NAME.into())
            .map(|n| self.raw.text_by_range(n.byte_range()).into())
    }
}

impl ToAstNode for Modifier {
    fn ast_node(self) -> AstNode {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode {
        &self.raw
    }

    fn cast(node: AstNode) -> Option<Self> {
        if node.node().kind_id() == NodeKind::MODIFIER_DEFINITION {
            Some(Self { raw: node })
        } else {
            None
        }
    }
    
    fn can_cast(node: &Node) -> bool {
        node.kind_id() == NodeKind::MODIFIER_DEFINITION
    }
}


////////////////////////////////////////////////////
///            IMPORT                           ///
//////////////////////////////////////////////////

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ImportType {
    Full,//can i use full for namespace? namespace is just full with an alias
    Named {
        symbols: Vec<ImportItem>,
    },
    Namespace {
        alias: String
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ImportItem {
    pub name: String,
    pub alias: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Import {
    raw: AstNode,
}

impl Import {
    pub fn import_type(&self) -> ImportType {
        let mut symbols: Vec<ImportItem> = Vec::new();
        let mut is_alias = false;
        let mut alias = String::new();
        let mut name = String::new();
        for node in self.raw.node().children(&mut self.raw.node().walk()) {
            if node.kind_id() == NodeKind::IDENTIFIER {
                if is_alias {
                    alias = self.raw.text_by_range(node.byte_range()).to_string();
                    if !symbols.is_empty() {
                        symbols.last_mut().unwrap().alias = Some(alias.clone());
                    }
                    //consume alias flag
                    is_alias = false;
                } else {
                    name = self.raw.text_by_range(node.byte_range()).to_string();
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
        self.raw.node().child_by_field_id(FieldKind::SOURCE.into()).map(|p| self.raw.text_by_range(p.byte_range())).unwrap_or("")
    }
}

impl ToAstNode for Import {
    fn ast_node(self) -> AstNode {
        self.raw
    }

    fn ast_node_ref(&self) -> &AstNode {
        &self.raw
    }

    fn cast(node: AstNode) -> Option<Self> {
        if node.node().kind_id() == NodeKind::IMPORT_DIRECTIVE {
            Some(Self { raw: node })
        } else {
            None
        }
    }

    fn can_cast(node: &Node) -> bool {
        node.kind_id() == NodeKind::IMPORT_DIRECTIVE
    }
}

/// ast::Item is a unification of all ast kinds acting as a blanket typed representation(i.e generic while still maintaining type info)
/// as opposed to AstNode which is just generic with no underlying type info
/// so we go AstNode -> Specific Type -> Item

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
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
    fn ast_node(self) -> AstNode {
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
            Item::Var(v) => v.ast_node(),
        }
    }

    fn ast_node_ref(&self) -> &AstNode {
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
            Item::Var(v) => v.ast_node_ref(),
        }
    }

    fn cast(node: AstNode) -> Option<Self> {
        match NodeKind::from(node.node().kind_id()) {
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

    fn can_cast(node: &Node) -> bool {
        matches!(
            NodeKind::from(node.kind_id()),
            NodeKind::CONTRACT_DEFINITION
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


#[macro_export]
macro_rules! match_ast {
    //root+node
    (
        match ($owner:expr, $node:expr) {
            $( $( $path:ident )::+ ($it:pat) => $res:expr, )*
            _ => $catch_all:expr $(,)?
        }
    ) => {{
        let __node = $node;
        $(
            if <$($path)::+ as $crate::ast::ToAstNode>::can_cast(&__node) {
                let $it = <$($path)::+ as $crate::ast::ToAstNode>::cast(
                    ($owner).make_ast(__node)
                ).expect("match_ast!: cast failed after can_cast");
                $res
            } else
        )*
        { $catch_all }
    }};

    //AstNode.
    (
        match ($ast:expr) {
            $( $( $path:ident )::+ ($it:pat) => $res:expr, )*
            _ => $catch_all:expr $(,)?
        }
    ) => {{
        let __ast = $ast;
        let __node = __ast.node();
        $(
            if <$($path)::+ as $crate::ast::ToAstNode>::can_cast(&__node) {
                let $it = <$($path)::+ as $crate::ast::ToAstNode>::cast(__ast)
                    .expect("match_ast!: cast failed after can_cast");
                $res
            } else
        )*
        { $catch_all }
    }};
}