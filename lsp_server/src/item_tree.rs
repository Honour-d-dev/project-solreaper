#![allow(unused)]
use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use tree_sitter::Node;

use crate::{
    ast::{self, ErasedFileAstId, ToAstNode},
    kinds::field_kind as Field_Kind,
    salsa_db::{FileText, RootAstDatabase},
};

fn field_text(node: Node<'_>, field_id: u16, source: &str) -> Option<SmolStr> {
    node.child_by_field_id(field_id)
        .map(|child| SmolStr::new(&source[child.byte_range()]))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemId {
    Import(ErasedFileAstId),
    Contract(ErasedFileAstId),
    Interface(ErasedFileAstId),
    Library(ErasedFileAstId),
    Function(ErasedFileAstId),
    StateVar(ErasedFileAstId),
    Struct(ErasedFileAstId),
    Enum(ErasedFileAstId),
    Event(ErasedFileAstId),
    Error(ErasedFileAstId),
    Modifier(ErasedFileAstId),
}


#[derive(Debug)]
pub enum Item {
    Import(Import),
    Contract(Contract),
    Interface(Interface),
    Library(Library),
    Function(Function),
    Var(Var),
    Struct(Struct),
    Enum(Enum),
    Event(Event),
    Error(Error),
    Modifier(Modifier),
}


#[derive(Debug, Default)]
pub struct ItemTree {
    // file-scope order only
    pub top_level: Box<[ItemId]>,

    // payload by ast id (single table is fine for v1)
    pub data: FxHashMap<ErasedFileAstId, Item>,
}




impl ItemTree {
    pub fn new(db: &dyn RootAstDatabase, file: FileText) -> Self {//or i just pass the map in 🤷‍♂️
        let ast = db.parse(file);
        let ast_id_map = db.ast_id_map(file);

        let root = ast.root();
        let root_node = root.node();
        let source = ast.source();
        let mut cursor = root_node.walk();

        let mut top_level = Vec::new();
        let mut data = FxHashMap::default();

        for ast_item in root_node
            .named_children(&mut cursor)
            .map(|node| ast::AstNode::new(node, source))
            .filter_map(ast::Item::cast)
        {
            let Some(ast_id) = ast_id_map.id_of(&ast_item).map(|id| id.erase()) else {
                continue;
            };

            let (item_id, item) = match ast_item {
                ast::Item::Import(i) => {
                    let node = i.ast_node().node();
                    let path = field_text(node, Field_Kind::SOURCE.as_u16(), source)
                        .map(|s| s.to_string())
                        .unwrap_or_default();
                    let name = field_text(node, Field_Kind::IMPORT_NAME.as_u16(), source)
                        .map(|s| s.to_string());
                    let alias = field_text(node, Field_Kind::ALIAS.as_u16(), source)
                        .map(|s| s.to_string());

                    (
                        ItemId::Import(ast_id),
                        Item::Import(Import { path, name, alias }),
                    )
                }
                ast::Item::Contract(c) => {
                    let node = c.ast_node().node();
                    let name = field_text(node, Field_Kind::NAME.as_u16(), source)
                        .unwrap_or_default();

                    (
                        ItemId::Contract(ast_id),
                        Item::Contract(Contract {
                            name,
                            bases: Box::default(),
                            visible_members: Box::default(),
                        }),
                    )
                }
                ast::Item::Interface(i) => {
                    let node = i.ast_node().node();
                    let name = field_text(node, Field_Kind::NAME.as_u16(), source)
                        .unwrap_or_default();

                    (
                        ItemId::Interface(ast_id),
                        Item::Interface(Interface {
                            name,
                            bases: Box::default(),
                        }),
                    )
                }
                ast::Item::Library(l) => {
                    let node = l.ast_node().node();
                    let name = field_text(node, Field_Kind::NAME.as_u16(), source)
                        .unwrap_or_default();

                    (
                        ItemId::Library(ast_id),
                        Item::Library(Library { name }),
                    )
                }
                ast::Item::Function(f) => {
                    let node = f.ast_node().node();
                    let name = field_text(node, Field_Kind::NAME.as_u16(), source)
                        .unwrap_or_default();

                    (
                        ItemId::Function(ast_id),
                        Item::Function(Function { name }),
                    )
                }
                ast::Item::Struct(s) => {
                    let node = s.ast_node().node();
                    let name = field_text(node, Field_Kind::NAME.as_u16(), source)
                        .unwrap_or_default();

                    (
                        ItemId::Struct(ast_id),
                        Item::Struct(Struct { name }),
                    )
                }
                ast::Item::Enum(e) => {
                    let node = e.ast_node().node();
                    let name = field_text(node, Field_Kind::NAME.as_u16(), source)
                        .unwrap_or_default();

                    (
                        ItemId::Enum(ast_id),
                        Item::Enum(Enum { name }),
                    )
                }
                ast::Item::Event(ev) => {
                    let node = ev.ast_node().node();
                    let name = field_text(node, Field_Kind::NAME.as_u16(), source)
                        .unwrap_or_default();

                    (
                        ItemId::Event(ast_id),
                        Item::Event(Event { name }),
                    )
                }
                ast::Item::Error(err) => {
                    let node = err.ast_node().node();
                    let name = field_text(node, Field_Kind::NAME.as_u16(), source)
                        .unwrap_or_default();

                    (
                        ItemId::Error(ast_id),
                        Item::Error(Error { name }),
                    )
                }
                ast::Item::Modifier(m) => {
                    let node = m.ast_node().node();
                    let name = field_text(node, Field_Kind::NAME.as_u16(), source)
                        .unwrap_or_default();

                    (
                        ItemId::Modifier(ast_id),
                        Item::Modifier(Modifier { name }),
                    )
                }
            };

            top_level.push(item_id);
            data.insert(ast_id, item);
        }

        Self {
            top_level: top_level.into_boxed_slice(),
            data,
        }
    }
}





#[derive(Debug)]
pub struct Import {
    pub path: String,
    pub name: Option<String>,
    pub alias: Option<String>,
}

#[derive(Debug)]
pub struct Contract {
    pub name: SmolStr,
    pub bases: Box<[SmolStr]>,       // unresolved base names in v1
    pub visible_members: Box<[ItemId]>,      // member item ids
}

#[derive(Debug)]
pub struct Interface {
    pub name: SmolStr,
    pub bases: Box<[SmolStr]>,       // unresolved base names in
}

#[derive(Debug)]
pub struct Library {
    pub name: SmolStr,
}

#[derive(Debug)]
pub struct Function {
    pub name: SmolStr,
}

#[derive(Debug)]
pub struct Var {
    pub name: SmolStr,
}

#[derive(Debug)]
pub struct Struct {
    pub name: SmolStr,
}

#[derive(Debug)]
pub struct Enum {
    pub name: SmolStr,
}

#[derive(Debug)]
pub struct Event {
    pub name: SmolStr,
}

#[derive(Debug)]
pub struct Error {
    pub name: SmolStr,
}

#[derive(Debug)]
pub struct Modifier {
    pub name: SmolStr,
}
