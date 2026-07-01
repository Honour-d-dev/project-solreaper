use std::ops::Index;

use triomphe::Arc;

use rustc_hash::FxHashMap;
use smol_str::SmolStr;

use crate::{
    ast::{
        self, Ast, AstId, AstIdMap, ErasedAstId, ImportType, ToAstNode,
    }, salsa_db::{File, RootDatabase},
};



/// useful for pattern matching out of items to retain type info.
/// similar to ast::Item, but on the id level.
/// ErasedFileAstId -> FileAstId -> ItemId
/// either this or we can_cast 
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemId {
    Import(AstId<ast::Import>),
    Contract(AstId<ast::Contract>),
    Interface(AstId<ast::Interface>),
    Library(AstId<ast::Library>),
    Function(AstId<ast::Function>),
    Var(AstId<ast::Var>),
    Struct(AstId<ast::Struct>),
    Enum(AstId<ast::Enum>),
    Event(AstId<ast::Event>),
    Error(AstId<ast::Error>),
    Modifier(AstId<ast::Modifier>),
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
pub struct Lowerer {
    ast: Arc<Ast>,
    ast_id_map: Arc<AstIdMap>,
    top_level: Vec<ItemId>,
    data: FxHashMap<ErasedAstId, Item>,
}

impl Lowerer {
    pub fn lower(db: &dyn RootDatabase, file: File) -> ItemTree {
        let mut lowerer = Lowerer {
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
                        path: i.path().into(),
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
                            members: self.lower_members(c.members()),
                    };
                    self.data.insert(id.erase(), Item::Contract(contract));
                    self.top_level.push(ItemId::Contract(id));
                },
                Some(ast::Item::Interface(i)) => {
                    let Some(id) = self.ast_id_map.id_of(&i) else {continue;};
                    let interface = Interface {
                        name: i.name().unwrap_or_default(),
                        bases: i.bases(),
                        members: self.lower_members(i.members()),
                    };
                    self.data.insert(id.erase(), Item::Interface(interface));
                    self.top_level.push(ItemId::Interface(id));
                },
                Some(ast::Item::Library(l)) => {
                    let Some(id) = self.ast_id_map.id_of(&l) else {continue;};
                    let library = Library {
                        name: l.name().unwrap_or_default(),
                        members: self.lower_members(l.members()),
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
    // file-scope only
    pub top_level: Box<[ItemId]>,

    //we're using erased to prevent lifetime coloring, the itemtree DOES NOT borrow from the ast
    pub data: FxHashMap<ErasedAstId, Item>,
}


macro_rules! impl_item_tree_index {
    ($($ast_ty:ty => $variant:ident),* $(,)?) => {
        $(
            impl Index<AstId<$ast_ty>> for ItemTree {
                type Output = $variant;
                fn index(&self, index: AstId<$ast_ty>) -> &Self::Output {
                    match self.data.get(&index.erase()).unwrap() {
                        Item::$variant(it) => it,
                        _ => panic!(concat!("expected ", stringify!($variant))),
                    }
                }
            }
        )*
    };
}

impl_item_tree_index! {
    ast::Import => Import,
    ast::Contract => Contract,
    ast::Interface => Interface,
    ast::Library => Library,
    ast::Function => Function,
    ast::Var => Var,
    ast::Struct => Struct,
    ast::Enum => Enum,
    ast::Event => Event,
    ast::Error => Error,
    ast::Modifier => Modifier,
}
///////////////////////////////////// MIR ///////////////////////////////////////////
///                             Item_tree::Nodes                                 ///
/// ////////////////////////////////////////////////////////////////////////////////

#[derive(Debug,Clone, PartialEq, Eq)]
pub struct Import {
    pub path: SmolStr,//tecnically not a small str but just to capitalize on the cheap clones
    pub import_type: ImportType,
}

#[derive(Debug,Clone, PartialEq, Eq)]
pub struct Contract {
    pub name: SmolStr,
    pub bases: Box<[SmolStr]>,       // unresolved base names in v1
    pub members: Box<[ItemId]>,
}

#[derive(Debug,Clone, PartialEq, Eq)]
pub struct Interface {
    pub name: SmolStr,
    pub bases: Box<[SmolStr]>,
    pub members: Box<[ItemId]>,
}

#[derive(Debug,Clone, PartialEq, Eq)]
pub struct Library {
    pub name: SmolStr,
    pub members: Box<[ItemId]>,
}

#[derive(Debug,Clone, PartialEq, Eq)]
pub struct Function {
    pub name: SmolStr,
}

#[derive(Debug,Clone, PartialEq, Eq)]
pub struct Var {
    pub name: SmolStr,
}

#[derive(Debug,Clone, PartialEq, Eq)]
pub struct Struct {
    pub name: SmolStr,
}

#[derive(Debug,Clone, PartialEq, Eq)]
pub struct Enum {
    pub name: SmolStr,
}

#[derive(Debug,Clone, PartialEq, Eq)]
pub struct Event {
    pub name: SmolStr,
}

#[derive(Debug,Clone, PartialEq, Eq)]
pub struct Error {
    pub name: SmolStr,
}

#[derive(Debug,Clone, PartialEq, Eq)]
pub struct Modifier {
    pub name: SmolStr,
}
