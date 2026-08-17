use std::collections::hash_map::Entry;
use std::mem;
use std::ops::Index;
use std::ops::IndexMut;

use la_arena::{Arena, Idx};
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use smallvec::SmallVec;
use smol_str::SmolStr;

use crate::ast::ImportId;
use crate::ast::UdvtId;
use crate::ast::UsingId;
use crate::ast::{ContractId, EnumId, ErasedAstId, ErrorId, EventId, FunctionId, ImportType, InterfaceId, LibraryId, ModifierId, StructId, VarId};
use crate::ir::item_tree::{ItemId, Import, ItemTree};
use crate::salsa::{FileId, RootDatabase, SourceRootId};

type Name = SmolStr;
type ScopeId = Idx<Scope>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DefId {
    File(FileId),
    Import(ImportId),
    Udvt(UdvtId),
    Using(UsingId),
    Contract(ContractId),
    Interface(InterfaceId),
    Library(LibraryId),
    Function(FunctionId),
    Modifier(ModifierId),
    Struct(StructId),
    Event(EventId),
    Enum(EnumId),
    Error(ErrorId),
    Var(VarId),
}

impl DefId {
    pub fn file_id(&self) -> (FileId, Option<ErasedAstId>) {
        match self {
            DefId::File(f) => (*f, None),
            DefId::Import(i) => (i.file, Some(i.id.erase())),
            DefId::Udvt(u) => (u.file, Some(u.id.erase())),
            DefId::Using(u) => (u.file, Some(u.id.erase())),
            DefId::Contract(c) => (c.file, Some(c.id.erase())),
            DefId::Interface(i) => (i.file, Some(i.id.erase())),
            DefId::Library(l) => (l.file, Some(l.id.erase())),
            DefId::Function(f) => (f.file, Some(f.id.erase())),
            DefId::Modifier(m) => (m.file, Some(m.id.erase())),
            DefId::Struct(s) => (s.file, Some(s.id.erase())),
            DefId::Event(e) => (e.file, Some(e.id.erase())),
            DefId::Enum(e) => (e.file, Some(e.id.erase())),
            DefId::Error(e) => (e.file, Some(e.id.erase())),
            DefId::Var(v) => (v.file, Some(v.id.erase())),
        }
    }
}


#[derive(Clone, PartialEq, Eq)]
pub struct Scope {
    pub owner: DefId,
    pub parent: Option<ScopeId>,
    pub by_name: FxHashMap<Name, ScopeData>,
    pub usings: Option<Vec<DefId>>
}

#[derive(PartialEq, Eq, Clone)]
pub struct ScopeData {
    pub namespace: Namespace,
    pub defs: SmallVec<[DefId;1]>,//most names wont be overloaded
}

/// Overloads can only occur per namespace in a scope and some namespaces dont support overloads within same scope. e.g types and variables
#[derive(PartialEq, Eq, Clone)]
pub enum Namespace {
    Type,//File, Contract, Interface, Library, Struct, Enum
    Function,
    Error,
    Event,
    Variable,
}

impl Namespace {
    pub fn from(def: &DefId) -> Namespace {
        match def {//TODO might remove imports/using from type namespace to its own.
            DefId::File(_) | DefId::Import(_) | DefId::Contract(_) | DefId::Interface(_) | DefId::Library(_) | 
            DefId::Struct(_) | DefId::Enum(_) | DefId::Udvt(_) | DefId::Using(_) => Namespace::Type,
            DefId::Function(_) |DefId::Modifier(_)  => Namespace::Function,
            DefId::Event(_) => Namespace::Event,
            DefId::Error(_) => Namespace::Error,
            DefId::Var(_) => Namespace::Variable,
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct DefData {
    pub name: Option<SmolStr>,
    pub scope: ScopeId, //defined scope
    pub child_scope: Option<ScopeId>,//body scope if any
    //visibility
}

#[derive(Default, Clone, PartialEq, Eq)]
pub struct UnresolvedImports {
    pub imports: Vec<Import>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeEntry {
    pub root: SourceRootId,
    pub scope: ScopeId,
}

#[derive(Clone,PartialEq, Eq)]
pub struct FileData {
    pub id: DefId,
    pub scope: ScopeId,
    pub imported_scopes: Vec<ScopeEntry>,
}


#[derive(Clone, PartialEq, Eq)]//TODO: manually implement partialeq so == can be cheaper while also helping cache invalidation. what does it mean for a defmap to  change?. a change in defs, should be all we need, no? instead of comparing field by field.
pub struct DefMap {
    pub root: SourceRootId,
    pub files: FxHashMap<FileId, FileData>,
    pub scopes: Arena<Scope>,
    pub defs: FxHashMap<DefId, DefData>,
    pub unresolved: FxHashMap<FileId, UnresolvedImports>,
}

pub struct FileCollector<'db, 'collector> {
    file: FileId,
    item_tree: &'db ItemTree,
    // second lifetime required so collector borrows don't inherit db lifetime. 
    // which allows them to be droped per loop iteration @Collector::collect_defmap
    collector: &'collector mut Collector<'db>,
}

impl<'db, 'collector> FileCollector<'db, 'collector> {
    
    fn collect_top(&mut self) {
        let scope_id = self.collect_file();
        let top_level = &self.item_tree.top_level;
        
        for &item_id in top_level.iter() {
            match item_id {
                ItemId::Import(id) => {
                    //@NOTE import defs are not collected in the defmap scopes
                    let import = &self.item_tree[id];
                    let def_data = DefData {
                        name: None,
                        scope: scope_id,
                        child_scope: None,
                    };
                    let id = DefId::Import(ImportId { file: self.file, id });
                    self.collector.defs.insert(id, def_data);
                    self.collector.unresolved_imports.entry(self.file).or_default().imports.push(import.clone());
                }
                ItemId::Using(id) => {
                    let data = DefData {
                        name: None,
                        scope: scope_id,
                        child_scope: None,
                    };
                    let id = DefId::Using(UsingId { file: self.file, id });
                    self.collector.defs.insert(id, data);
                    self.collector.scopes[scope_id].usings.get_or_insert_with(Vec::new).push(id);
                }
                ItemId::Contract(id) => {
                    let c = &self.item_tree[id];
                    let id = DefId::Contract(ContractId { file: self.file, id });
                    self.collect_container(id, scope_id, c.name.clone(), &c.members);
                }
                ItemId::Interface(id) => {
                    let i = &self.item_tree[id];
                    let id = DefId::Interface(InterfaceId { file: self.file, id });
                    self.collect_container(id, scope_id, i.name.clone(), &i.members);
                }
                ItemId::Library(id) => {
                    let l = &self.item_tree[id];
                    let id = DefId::Library(LibraryId { file: self.file, id });
                    self.collect_container(id, scope_id, l.name.clone(), &l.members);
                }
                ItemId::Udvt(id) => {
                    let u = &self.item_tree[id];
                    let id = DefId::Udvt(UdvtId { file: self.file, id });
                    self.collect_def(id, scope_id, None, u.name.clone().unwrap_or_default());
                }
                ItemId::Struct(id) => {//TODO: collect fields. Do we create struct scopes for fields??
                    let s = &self.item_tree[id];
                    let id = DefId::Struct(StructId { file: self.file, id });
                    self.collect_def(id, scope_id, None, s.name.clone());
                }
                ItemId::Enum(id) => {//Smae here
                    let e = &self.item_tree[id];
                    let id = DefId::Enum(EnumId { file: self.file, id });
                    self.collect_def(id, scope_id, None, e.name.clone());
                }
                ItemId::Function(id) => {
                    let f = &self.item_tree[id];
                    let id = DefId::Function(FunctionId { file: self.file, id });
                    self.collect_def(id, scope_id, None, f.name.clone());
                }
                ItemId::Event(id) => {
                    let e = &self.item_tree[id];
                    let id = DefId::Event(EventId { file: self.file, id });
                    self.collect_def(id, scope_id, None, e.name.clone());
                }
                ItemId::Error(id) => {
                    let e = &self.item_tree[id];
                    let id = DefId::Error(ErrorId { file: self.file, id });
                    self.collect_def(id, scope_id, None, e.name.clone());
                }
                ItemId::Modifier(id) => {
                    let m = &self.item_tree[id];
                    let id = DefId::Modifier(ModifierId { file: self.file, id });
                    self.collect_def(id, scope_id, None, m.name.clone());
                }
                ItemId::Var(id) => {
                    let v = &self.item_tree[id];
                    let id = DefId::Var(VarId { file: self.file, id });
                    self.collect_def(id, scope_id, None, v.name.clone());
                }
            }
        }
        
    }

    fn collect_file(&mut self) -> ScopeId {
        let id = DefId::File(self.file);
        let scope_id = self.collector.scopes.alloc(Scope {
            owner: id, 
            parent: None, 
            by_name: FxHashMap::default(), 
            usings: Default::default() 
        });

        self.collector.files.insert(self.file, FileData {
            id,
            scope: scope_id,
            imported_scopes: Vec::new(),
        });
    
        //Insert files as defs as well. so we can import as namespace
        self.collector.defs.insert(id, DefData { 
            name: None, 
            scope: scope_id, //FIXME: this should be none
            child_scope: Some(scope_id)
        });
        scope_id
    }

    fn collect_container(&mut self, id: DefId, scope_id: ScopeId, name: Name, members: &[ItemId]) {
        let sub_scope = Scope {
            owner: id,
            parent: Some(scope_id),
            by_name: FxHashMap::default(),
            usings: Default::default()
        };
        
        let sub_scope_id = self.collector.scopes.alloc(sub_scope);

        self.collect_def(id, scope_id, Some(sub_scope_id), name);
        self.collect_members( members, sub_scope_id);
    }



    fn collect_members(&mut self, members: &[ItemId], parent_scope: ScopeId) {
        for &member in members {
            
            let (id, name) = match member {
                ItemId::Struct(id) => (DefId::Struct(StructId { file: self.file, id }), self.item_tree[id].name.clone()),
                ItemId::Enum(id) => (DefId::Enum(EnumId { file: self.file, id }), self.item_tree[id].name.clone()),
                ItemId::Function(id) => (DefId::Function(FunctionId { file: self.file, id }), self.item_tree[id].name.clone()),
                ItemId::Event(id) => (DefId::Event(EventId { file: self.file, id }), self.item_tree[id].name.clone()),
                ItemId::Error(id) => (DefId::Error(ErrorId { file: self.file, id }), self.item_tree[id].name.clone()),
                ItemId::Modifier(id) => (DefId::Modifier(ModifierId { file: self.file, id }), self.item_tree[id].name.clone()),
                ItemId::Var(id) => (DefId::Var(VarId { file: self.file, id }), self.item_tree[id].name.clone()),
                ItemId::Udvt(id) => (DefId::Udvt(UdvtId { file: self.file, id }), self.item_tree[id].name.clone().unwrap_or_default()),
                ItemId::Using(id) => {
                    let data = DefData {
                        name: None,
                        scope: parent_scope,
                        child_scope: None,
                    };
                    let id = DefId::Using(UsingId { file: self.file, id });
                    self.collector.defs.insert(id, data);
                    self.collector.scopes[parent_scope].usings.get_or_insert_with(Vec::new).push(id);
                    continue
                }
                _ => continue
            };

            self.collect_def(id, parent_scope, None, name);
        }
    }



    fn collect_def(&mut self,id: DefId, scope_id: ScopeId, sub_scope: Option<ScopeId>, name: Name) {
        let scope = self.collector.scopes.index_mut(scope_id);
        let ns = Namespace::from(&id);
        match scope.by_name.entry(name.clone()) {
            Entry::Occupied(mut entry) => {
                let e = entry.get_mut();
                if e.namespace != ns || matches!(e.namespace, Namespace::Type | Namespace::Variable) {
                    // subsequent defs must match the first def's namespace and the namespace must support overloading
                    return;//TODO: collect diagnostic here?
                }
                e.defs.push(id);
            }
            Entry::Vacant(entry) => {
                entry.insert(ScopeData { namespace: ns, defs: SmallVec::from([id]) });
            }
        }
        let def_data = DefData {
            name: Some(name),
            scope: scope_id,
            child_scope: sub_scope,
        };
        self.collector.defs.insert(id, def_data);
    }
}

pub struct Collector<'db> {
    db: &'db dyn RootDatabase,
    root: SourceRootId,
    pub files: FxHashMap<FileId, FileData>,
    pub scopes: Arena<Scope>,
    pub defs: FxHashMap<DefId, DefData>,
    pub unresolved_imports: FxHashMap<FileId, UnresolvedImports>
}


impl<'db> Collector<'db> {
    pub fn collect_defmap(db: &'db dyn RootDatabase, source_root_id: SourceRootId) -> DefMap {
        let files = source_root_id.source_root(db).files.as_ref();

        let mut collector = Collector {
            db,
            root: source_root_id,
            files: FxHashMap::default(),
            scopes: Arena::default(),
            defs: FxHashMap::default(),
            unresolved_imports: FxHashMap::default(),
        };

        for file in files.iter() {
            FileCollector {
                file: *file,
                item_tree: db.item_tree(*file),
                collector: &mut collector,
            }.collect_top();
        }

        collector.resolve_imports();
        
        DefMap {
            root: collector.root,
            files: collector.files,
            scopes: collector.scopes,
            defs: collector.defs,
            unresolved: collector.unresolved_imports,
        }
    }


    /// Resolving imports is tricky because a global import brings its own imported global scope
    /// 
    /// i.e. A imports B but B imports C, so A auto imports C via B (i.e has C's global scope as well)
    /// 
    /// But Solidity also supports cyclic imports, which mean C can also import A (inheriting A's global scope too) typical chicken<->hen problem
    /// 
    /// Now we have an import loop. where all files(A,B,C) essentially have/share the same global scope.
    fn resolve_imports(&mut self) {
        while let Some(&file_id) = self.unresolved_imports.keys().next() {
            let mut seen = FxHashSet::default();
            seen.insert(file_id);
            assert_eq!(self.resolve(file_id, &mut seen),Resolver::Finished, "Unable to resolve imports due to cyclic dependency in file {:?}", file_id);
        }
    }


    
    fn resolve(&mut self, file_id: FileId, chain: &mut FxHashSet<FileId>) -> Resolver {
        let mut unresolved = match self.unresolved_imports.get_mut(&file_id) {
            Some(u) => mem::take(u),
            None => UnresolvedImports::default(),
        };
        let mut resolved_paths = FxHashSet::default();
        let mut resolver = Resolver::Finished;

        for import in unresolved.imports.iter() {

            // If we can't resolve path to file we skip it. TODO: record it instead.
            let Some(dep) = self.db.resolve_to_file( file_id, import.path.as_str()) else {continue;};
            let other_root = self.db.file_source_root(dep);

            // same root
            if self.root == other_root {
                if self.unresolved_imports.contains_key(&dep) {
                    if chain.insert(dep){//@FIXME: fix to support multiple imports/import_directives from same file
                        match self.resolve(dep, chain) {
                            Resolver::Finished => {
                                //it finished/resolved, dep should already remove itself from map
                                // we schedule to remove dep from current unresolved imports
                                resolved_paths.insert(import.path.clone());
                            }
                            c @ Resolver::Cycle(_) => {
                                resolver.merge(c);
                            }
                        }
                        self.extend_scope(file_id, dep, self.root, &import.import_type);
                    } else {
                        resolver.merge(Resolver::Cycle(vec![dep]));
                    }
                } else {//file already resolved just collect scope
                    self.extend_scope(file_id, dep, self.root, &import.import_type);
                    resolved_paths.insert(import.path.clone());
                }
            } else {
                //get scope from other_root's defmap
                self.extend_scope(file_id, dep, other_root, &import.import_type);
                resolved_paths.insert(import.path.clone());
            }
        }
        
        match resolver.resolve(file_id) {
            f @ Resolver::Finished=> {
                //a cycle isn't just "we've seen this file before", but we've seen this file in this chain/dependency path before.
                // if a path resolves it is removed from the chain as we unwind
                self.unresolved_imports.remove(&file_id);//it should be empty atp
                chain.remove(&file_id);
                f
            }
            c @ Resolver::Cycle(_) => {
                //unresolved, so we prune instead and reinsert
                unresolved.imports.retain(|i| !resolved_paths.contains(&i.path));
                self.unresolved_imports.insert(file_id, unresolved);
                c
            }
        }
    }   


    fn extend_scope(&mut self, file: FileId, dep: FileId, dep_root: SourceRootId, import_type: &ImportType) {
        match import_type {
            ImportType::Full => {
                let dep_data = self.file_data(dep_root, dep);
                let file_data = self.files.get_mut(&file).unwrap();
                push_unique_scope(&mut file_data.imported_scopes, ScopeEntry { root: dep_root, scope: dep_data.scope });
                for scope in dep_data.imported_scopes {
                    push_unique_scope(&mut file_data.imported_scopes, scope);
                }
            }
            ImportType::Named { symbols } => {
                let file_scope = self.files.get(&file).unwrap().scope;
                for symbol in symbols {
                    let Some(ids) = self.find_name_in_file(dep_root, dep, &symbol.name) else { continue; };
                    let name = symbol.alias.clone().unwrap_or(symbol.name.clone());
                    match self.scopes.index_mut(file_scope).by_name.entry(name) {
                        Entry::Occupied(mut entry) => {
                            let e = entry.get_mut();
                            if e.namespace != ids.namespace || matches!(e.namespace, Namespace::Type | Namespace::Variable) {
                                continue;
                            }
                            extend_unique_defs(&mut e.defs, ids.defs);
                        }
                        Entry::Vacant(v) => {
                            v.insert(ids);
                        }
                    }
                }
            }
            ImportType::Namespace { alias } => {
                let dep_def_id = self.file_data(dep_root, dep).id;
                let file_scope = self.files.get(&file).unwrap().scope;
                let scope = self.scopes.index_mut(file_scope);
                if let Entry::Vacant(v) =   scope.by_name.entry(alias.clone()) {
                    v.insert(ScopeData {
                        namespace: Namespace::Type,
                        defs: [dep_def_id].into(),
                    });
                }
            }
        }
        self.extend_global_usings(file, dep, dep_root);
    }

    fn extend_global_usings(&mut self, file: FileId, dep: FileId, dep_root: SourceRootId) {
        let dep_data = self.file_data(dep_root, dep);
        let file_scope = self.files.get(&file).unwrap().scope;
        // DFS resolution guarantees all globals (up to this point) are already accumulated in dep scope. no need to check imported scopes
        let to_add: Vec<DefId> = {
            let other_defmap;
            let usings = if self.root == dep_root {
                self.scopes[dep_data.scope].usings.as_deref()
            } else {
                other_defmap = self.db.root_def_map(dep_root);
                other_defmap.scopes[dep_data.scope].usings.as_deref()
            };
            usings.into_iter().flatten().filter(|&using_def| {
                if let DefId::Using(using_id) = using_def {
                    self.db.item_tree(using_id.file)[using_id.id].is_global
                } else {
                    false
                }
            }).copied().collect()
        };
        for using_def in to_add {
            let file_usings = self.scopes[file_scope].usings.get_or_insert_with(Vec::new);
            if !file_usings.contains(&using_def) {
                file_usings.push(using_def);
            }
        }
    }


    fn file_data(&self, root: SourceRootId, file: FileId) -> FileData {
        if self.root == root {
            self.files.get(&file).unwrap().clone()
        } else {
            self.db.root_def_map(root).files.get(&file).unwrap().clone()
        }
    }

    fn find_name_in_file(&self, root: SourceRootId, file: FileId, name: &Name) -> Option<ScopeData> {
        let file_data = self.file_data(root, file);
        let local_scope = ScopeEntry { root, scope: file_data.scope };
        self.find_name_in_scope(local_scope, name).or_else(|| {
            file_data
                .imported_scopes
                .iter()
                .find_map(|scope| self.find_name_in_scope(*scope, name))
        })
    }

    fn find_name_in_scope(&self, scope: ScopeEntry, name: &Name) -> Option<ScopeData> {
        if self.root == scope.root {
            self.scopes.index(scope.scope).by_name.get(name).cloned()
        } else {
            self.db.root_def_map(scope.root).scopes.index(scope.scope).by_name.get(name).cloned()
        }
    }
}

fn push_unique_scope(scopes: &mut Vec<ScopeEntry>, scope: ScopeEntry) {
    if !scopes.contains(&scope) {
        scopes.push(scope);
    }
}

fn push_unique_def(defs: &mut SmallVec<[DefId; 1]>, def: DefId) {
    if !defs.contains(&def) {
        defs.push(def);
    }
}

fn extend_unique_defs(defs: &mut SmallVec<[DefId; 1]>, new_defs: impl IntoIterator<Item = DefId>) {
    for def in new_defs {
        push_unique_def(defs, def);
    }
}


#[derive(Default, Clone, PartialEq, Eq, Debug)]
enum Resolver {
    Cycle(Vec<FileId>),
    #[default]
    Finished,
}

impl Resolver {
    pub fn merge(&mut self, other: Resolver) {
        match (self, other) {
            (_, Resolver::Finished) => (),
            (this @ Resolver::Finished,other @  Resolver::Cycle(_)) => *this = other,
            (Resolver::Cycle(a), Resolver::Cycle(b)) => a.extend(b),
        }
    }

    pub fn resolve(self, file_id: FileId) -> Resolver {
        match self {
            Resolver::Cycle(mut cycle) => {
                cycle.retain(|&f| f != file_id);
                if cycle.is_empty() {
                    Resolver::Finished
                } else {
                    Resolver::Cycle(cycle)
                }  
            }
            Resolver::Finished => self,
        }
    }
}



