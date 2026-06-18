#![allow(unused)]
use std::sync::Arc;

use rustc_hash::FxHashMap;
use smol_str::SmolStr;
use tree_sitter::Node;

use crate::{
    ast::{
        self, Ast, AstIdMap, AstNode, ErasedFileAstId, FileAstId, ImportType, ItemId, NodePtr, ToAstNode, kinds::{
            FieldKind, NodeKind
        }, match_ast
    },
    salsa_db::{FileText, RootAstDatabase},
};

fn field_text(node: Node<'_>, field_id: u16, source: &str) -> Option<SmolStr> {
    node.child_by_field_id(field_id)
        .map(|child| SmolStr::new(&source[child.byte_range()]))
}


/// Item_tree::Item, (not to be mistaken with ast::Item) but similar concept
/// Blanket type for all item_tree::node types
#[derive(Debug, PartialEq, Eq)]
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


/// ItemTree::Lowerer
pub struct Lowerer<'db> {
    db: &'db dyn RootAstDatabase,
    ast: Arc<Ast>,
    ast_id_map: Arc<AstIdMap>,
    top_level: Vec<ItemId>,
    data: FxHashMap<ErasedFileAstId, Item>,
}

impl<'db> Lowerer<'db> {
    pub fn lower(db: &dyn RootAstDatabase, file: FileText) -> ItemTree {
        let mut lowerer = Lowerer {
            db,
            ast: db.parse(file),
            ast_id_map: db.ast_id_map(file),
            top_level: Vec::new(),
            data: FxHashMap::default(),
        };

        lowerer.lower_top();

        lowerer.finish()
    }


    fn lower_top(&mut self) {
        let root = self.ast.root();
        
        for node in root.node().named_children(&mut root.node().walk()) {
            match ast::Item::cast(root.make_ast(node)) {
                Some(ast::Item::Import(i)) => {
                    let Some(id) = self.ast_id_map.id_of(&i) else {continue;};
                    let import = Import {
                        path: i.path().to_string(),
                        import_type: i.import_type(),
                    };
                    self.data.insert(id.erase(), Item::Import(import));
                    self.top_level.push(ItemId::Import(id));
                },
                Some(ast::Item::Contract(c)) => {
                    let Some(id) = self.ast_id_map.id_of(&c) else{continue;};
                    let contract = Contract {
                            name: c.name().unwrap_or_default(),
                            bases: c.bases(),
                            visible_members: self.lower_members(c.members()),
                    };
                    self.data.insert(id.erase(), Item::Contract(contract));
                    self.top_level.push(ItemId::Contract(id));
                },
                Some(ast::Item::Interface(i)) => {
                    let Some(id) = self.ast_id_map.id_of(&i) else {continue;};
                    let interface = Interface {
                        name: i.name().unwrap_or_default(),
                        bases: i.bases(),
                        visible_members: self.lower_members(i.members()),
                    };
                    self.data.insert(id.erase(), Item::Interface(interface));
                    self.top_level.push(ItemId::Interface(id));
                },
                Some(ast::Item::Library(l)) => {
                    let Some(id) = self.ast_id_map.id_of(&l) else {continue;};
                    let library = Library {
                        name: l.name().unwrap_or_default(),
                        visible_members: self.lower_members(l.members()),
                    };
                    self.data.insert(id.erase(), Item::Library(library));
                    self.top_level.push(ItemId::Library(id));
                },
                Some(ast::Item::Function(f)) => {
                    let Some(id) = self.ast_id_map.id_of(&f) else {continue;};
                    let function = Function {
                        name: f.name().unwrap_or_default(),
                    };
                    self.top_level.push(ItemId::Function(id));
                    self.data.insert(id.erase(), Item::Function(function));
                },
                Some(ast::Item::Var(v)) => {
                    let Some(id) = self.ast_id_map.id_of(&v) else {continue;};
                    let variable = Var {
                        name: v.name().unwrap_or_default(),
                    };
                    self.top_level.push(ItemId::Var(id));
                    self.data.insert(id.erase(), Item::Var(variable));
                },
                Some(ast::Item::Struct(s)) => {
                    let Some(id) = self.ast_id_map.id_of(&s) else {continue;};
                    let struct_ = Struct {
                        name: s.name().unwrap_or_default(),
                    };
                    self.top_level.push(ItemId::Struct(id));
                    self.data.insert(id.erase(), Item::Struct(struct_));
                },
                Some(ast::Item::Enum(e)) => {
                    let Some(id) = self.ast_id_map.id_of(&e) else {continue;};
                    let enum_ = Enum {
                        name: e.name().unwrap_or_default(),
                    };
                    self.top_level.push(ItemId::Enum(id));
                    self.data.insert(id.erase(), Item::Enum(enum_));
                },
                Some(ast::Item::Event(e)) => {
                    let Some(id) = self.ast_id_map.id_of(&e) else {continue;};
                    let event = Event {
                        name: e.name().unwrap_or_default(),
                    };
                    self.top_level.push(ItemId::Event(id));
                    self.data.insert(id.erase(), Item::Event(event));
                },
                Some(ast::Item::Error(e)) => {
                    let Some(id) = self.ast_id_map.id_of(&e) else {continue;};
                    let error = Error {
                        name: e.name().unwrap_or_default(),
                    };
                    self.top_level.push(ItemId::Error(id));
                    self.data.insert(id.erase(), Item::Error(error));
                },
                Some(ast::Item::Modifier(m)) => {
                    let Some(id) = self.ast_id_map.id_of(&m) else {continue;};
                    let modifier = Modifier {
                        name: m.name().unwrap_or_default(),
                    };
                    self.top_level.push(ItemId::Modifier(id));
                    self.data.insert(id.erase(), Item::Modifier(modifier));
                },
                _ => continue,//is exhaustive ,but to prevent external changes to item from from breaking here. REMIND: change to None
           }

        }
    }

    fn lower_members(&mut self, members: Box<[ast::Item]>) -> Box<[ItemId]> {
        let mut result = Vec::new();
        for member in members.into_iter() {
            match member {
                ast::Item::Function(f) => {
                    let Some(id) = self.ast_id_map.id_of(&f) else {continue;};
                    let function = Function {
                        name: f.name().unwrap_or_default(),
                    };
                    result.push(ItemId::Function(id));
                    self.data.insert(id.erase(), Item::Function(function));
                },
                ast::Item::Event(e) => {
                    let Some(id) = self.ast_id_map.id_of(&e) else {continue;};
                    let event = Event {
                        name: e.name().unwrap_or_default(),
                    };
                    result.push(ItemId::Event(id));
                    self.data.insert(id.erase(), Item::Event(event));
                },
                ast::Item::Struct(s) => {
                    let Some(id) = self.ast_id_map.id_of(&s) else {continue;};
                    let strukt = Struct {
                        name: s.name().unwrap_or_default(),
                    };
                    result.push(ItemId::Struct(id));
                    self.data.insert(id.erase(), Item::Struct(strukt));
                },
                ast::Item::Enum(e) => {
                    let Some(id) = self.ast_id_map.id_of(&e) else {continue;};
                    let enm = Enum {
                        name: e.name().unwrap_or_default(),
                    };
                    result.push(ItemId::Enum(id));
                    self.data.insert(id.erase(), Item::Enum(enm));
                },
                ast::Item::Error(e) => {
                    let Some(id) = self.ast_id_map.id_of(&e) else {continue;};
                    let error = Error {
                        name: e.name().unwrap_or_default(),
                    };
                    result.push(ItemId::Error(id));
                    self.data.insert(id.erase(), Item::Error(error));
                },
                ast::Item::Modifier(m) => {
                    let Some(id) = self.ast_id_map.id_of(&m) else {continue;};
                    let modifier = Modifier {
                        name: m.name().unwrap_or_default(),
                    };
                    result.push(ItemId::Modifier(id));
                    self.data.insert(id.erase(), Item::Modifier(modifier));
                },
                ast::Item::Var(v) => {
                    let Some(id) = self.ast_id_map.id_of(&v) else {continue;};
                    let variable = Var {
                        name: v.name().unwrap_or_default(),
                    };
                    result.push(ItemId::Var(id));
                    self.data.insert(id.erase(), Item::Var(variable));
                },
                _ => continue,
            }
        }
        result.into_boxed_slice()
    }

    fn finish(mut self) -> ItemTree {
        self.data.shrink_to_fit();
        ItemTree {
            top_level: self.top_level.into_boxed_slice(),
            data: self.data,
        }
    }
}


#[derive(Debug, Default, PartialEq, Eq)]
pub struct ItemTree {
    // file-scope order only
    pub top_level: Box<[ItemId]>,//i dont think i need itemId

    // payload by ast id (single table is fine for v1)
    //we're using erased for now  to prevent lifetime coloring, the itemtree DOES NOT borrow from the ast
    pub data: FxHashMap<ErasedFileAstId, Item>,
}



//TODO implement a lowerer that lowers into the item tree. that will be more flexible(i think) since we can lower indepth or shallow like this
impl ItemTree {
    pub fn get(&self, item_id: ItemId) -> &Item {
        self.data.get(&item_id.erase()).unwrap()
    }

}



///////////////////////////////////// MIR ///////////////////////////////////////////
///                             Item_tree::Nodes                                 ///
/// ////////////////////////////////////////////////////////////////////////////////

#[derive(Debug, PartialEq, Eq)]
pub struct Import {
    pub path: String,
    pub import_type: ImportType,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Contract {
    pub name: SmolStr,
    pub bases: Box<[SmolStr]>,       // unresolved base names in v1
    pub visible_members: Box<[ItemId]>, // we don't filter by visibity yet, this is misleading
}

#[derive(Debug, PartialEq, Eq)]
pub struct Interface {
    pub name: SmolStr,
    pub bases: Box<[SmolStr]>,
    pub visible_members: Box<[ItemId]>,// we don't filter by visibity yet, this is misleading
}

#[derive(Debug, PartialEq, Eq)]
pub struct Library {
    pub name: SmolStr,
    pub visible_members: Box<[ItemId]>,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Function {
    pub name: SmolStr,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Var {
    pub name: SmolStr,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Struct {
    pub name: SmolStr,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Enum {
    pub name: SmolStr,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Event {
    pub name: SmolStr,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Error {
    pub name: SmolStr,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Modifier {
    pub name: SmolStr,
}
