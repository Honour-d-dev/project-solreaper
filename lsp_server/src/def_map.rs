use std::mem;
use std::ops::Index;
use std::ops::IndexMut;

use la_arena::{Arena, Idx};
use rustc_hash::FxHashMap;
use rustc_hash::FxHashSet;
use smol_str::SmolStr;

use crate::ast::{ContractId, EnumId, ErrorId, EventId, FunctionId, ImportType, InterfaceId, LibraryId, ModifierId, StructId, VariableId};
use crate::item_tree::{ItemId};
use crate::{
    item_tree::{Import, ItemTree}, salsa_db::{FileId, RootDatabase, SourceRootId},
};

type Name = SmolStr;
type ScopeId = Idx<Scope>;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub enum DefId {
    SourceFile(FileId),
    Contract(ContractId),
    Interface(InterfaceId),
    Library(LibraryId),
    Function(FunctionId),
    Modifier(ModifierId),
    Struct(StructId),
    Event(EventId),
    Enum(EnumId),
    Error(ErrorId),
    Variable(VariableId),
}


#[derive(Default, Clone, PartialEq, Eq)]
pub struct Scope {
    pub parent: Option<ScopeId>,
    pub by_name: FxHashMap<Name, Vec<DefId>>,
}

#[derive(Clone, PartialEq, Eq)]
pub struct DefData {
    pub name: Option<SmolStr>,
    pub scope: ScopeId, //defined scope
    pub child_scope: Option<ScopeId>,//body scope if any
    //visibility
}

#[derive(Default, Clone, PartialEq, Eq)]
pub struct Unresolved {
    pub imports: Vec<Import>,
    pub names: Vec<Name>,//mainly for bases
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScopeRef {
    root: SourceRootId,
    scope: ScopeId,
}

#[derive(Clone,PartialEq, Eq)]
pub struct FileData {
    id: DefId,
    scope: ScopeId,
    imported_scopes: Vec<ScopeRef>,
}


#[derive(Clone, PartialEq, Eq)]//TODO: manually implement partialeq so == can be cheaper while also helping cache invalidation. what does it mean for a defmap to  change?. a change in defs, should be all we need, no? instead of comparing field by field.
pub struct DefMap {
    pub root: SourceRootId,
    pub files: FxHashMap<FileId, FileData>,
    pub scopes: Arena<Scope>,
    pub defs: FxHashMap<DefId, DefData>,
    pub unresolved: FxHashMap<FileId, Unresolved>,
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
                    let import = &self.item_tree[id];
                    self.collector.unresolved.entry(self.file).or_default().imports.push(import.clone());
                }
                ItemId::Contract(id) => {
                    let c = &self.item_tree[id];
                    let id = DefId::Contract(ContractId { file: self.file, id });
                    self.collect_container(id, scope_id, c.name.clone(), &c.members,Some(&c.bases));
                }
                ItemId::Interface(id) => {
                    let i = &self.item_tree[id];
                    let id = DefId::Interface(InterfaceId { file: self.file, id });
                    self.collect_container(id, scope_id, i.name.clone(), &i.members, Some(&i.bases));
                }
                ItemId::Library(id) => {
                    let l = &self.item_tree[id];
                    let id = DefId::Library(LibraryId { file: self.file, id });
                    self.collect_container(id, scope_id, l.name.clone(), &l.members, None);
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
                    let id = DefId::Variable(VariableId { file: self.file, id });
                    self.collect_def(id, scope_id, None, v.name.clone());
                }
            }
        }
        
    }

    fn collect_file(&mut self) -> ScopeId {
        let scope_id = self.collector.scopes.alloc(Scope::default());

        let id = DefId::SourceFile(self.file);
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

    fn collect_container(&mut self, id: DefId, scope_id: ScopeId, name: Name, members: &[ItemId], bases: Option<&[Name]>) {
        let sub_scope = Scope {
            parent: Some(scope_id),
            ..Default::default()
        };
        //FIXME: this is unused
        if let Some(bases) = bases {
            let mut unresolved = Vec::new();
            for base in  bases {
                if !self.collector.scopes.index(scope_id).by_name.contains_key(base) {
                    unresolved.push(base.clone());
                }
            }
            self.collector.unresolved.entry(self.file).or_default().names.extend(unresolved);
        }
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
                ItemId::Var(id) => (DefId::Variable(VariableId { file: self.file, id }), self.item_tree[id].name.clone()),
                _ => continue
            };

            self.collect_def(id, parent_scope, None, name);
        }
    }



    fn collect_def(&mut self,id: DefId, scope_id: ScopeId, sub_scope: Option<ScopeId>, name: Name) {
        let scope = self.collector.scopes.index_mut(scope_id);
        scope.by_name.entry(name.clone()).or_default().push(id);
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
    pub unresolved: FxHashMap<FileId, Unresolved>,
}


impl<'db> Collector<'db> {
    pub fn collect_defmap(db: &'db dyn RootDatabase, source_root_id: SourceRootId) -> DefMap {
        let files = source_root_id.source_root(db).files.as_ref();

        //we should collect package data here ie remappings
        let mut collector = Collector {
            db,
            root: source_root_id,
            files: FxHashMap::default(),
            scopes: Arena::default(),
            defs: FxHashMap::default(),
            unresolved: FxHashMap::default(),
        };
        //collect items from all the item trees in the sourceroot
        //resolve imports: we can pull in other  defmaps from other roots
        //resolve inheritance for all contracts? we can make this its own salsa query so it can be lazy
        //linearizing for all contracts in root will be a lot of work

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
            unresolved: collector.unresolved,
        }
    }


    /// Resolving imports is tricky because a global import imports its own imported global scope
    /// 
    /// i.e. A imports B but B imports C, so A auto imports C via B (i.e has C's global scope as well)
    /// 
    /// But Solidity also supports cyclic imports, which mean C can also import A (inheriting A's global scope too) typical chicken<->hen problem
    /// 
    /// Now we have an import loop. where all files(A,B,C) essentially have/share the same global scope.
    fn resolve_imports(&mut self) {
        while let Some(&file_id) = self.unresolved.keys().next() {
            let mut seen = FxHashSet::default();
            seen.insert(file_id);
            assert_eq!(self.resolve(file_id, &mut seen),Resolver::Finished, "Unable to resolve imports due to cyclic dependency in file {:?}", file_id);
        }
    }


    
    fn resolve(&mut self, file_id: FileId, chain: &mut FxHashSet<FileId>) -> Resolver {
        let mut unresolved = match self.unresolved.get_mut(&file_id) {
            Some(u) => mem::take(u),
            None => Unresolved::default(),
        };
        let mut resolved_paths = FxHashSet::default();
        let mut resolver = Resolver::Finished;

        for import in unresolved.imports.iter() {

            // If we can't resolve path to file we skip it. TODO: record it instead.
            let Some(dep) = self.db.resolve_to_file( file_id, import.path.as_str()) else {continue;};
            let other_root = self.db.file_source_root(dep);

            // same root
            if self.root == other_root {
                if self.unresolved.contains_key(&dep) {
                    if chain.insert(dep){
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
                //a cycle isn't we've seen this file before, but we've seen this file in this chain/dependency path before.
                // if a path resolves it is removed from the chain as we unwind
                self.unresolved.remove(&file_id);//its empty atp
                chain.remove(&file_id);
                f
            }
            c @ Resolver::Cycle(_) => {
                //unresolved, so we prune instead and reinsert
                unresolved.imports.retain(|i| !resolved_paths.contains(&i.path));
                self.unresolved.insert(file_id, unresolved);
                c
            }
        }
    }   


    fn extend_scope(&mut self, file: FileId, dep: FileId, dep_root: SourceRootId, import_type: &ImportType) {
        match import_type {
            ImportType::Full => {
                let dep_data = self.file_data(dep_root, dep);
                let file_data = self.files.get_mut(&file).unwrap();
                push_unique_scope(&mut file_data.imported_scopes, ScopeRef { root: dep_root, scope: dep_data.scope });
                for scope in dep_data.imported_scopes {
                    push_unique_scope(&mut file_data.imported_scopes, scope);
                }
            }
            ImportType::Named { symbols } => {
                let file_scope = self.files.get(&file).unwrap().scope;
                for symbol in symbols {
                    let Some(ids) = self.find_name_in_file(dep_root, dep, &symbol.name) else { continue; };
                    let name = symbol.alias.clone().unwrap_or(symbol.name.clone());
                    let entry = self.scopes.index_mut(file_scope).by_name.entry(name).or_default();
                    extend_unique_defs(entry, ids);
                }
            }
            ImportType::Namespace { alias } => {
                let dep_def_id = self.file_data(dep_root, dep).id;
                let file_scope = self.files.get(&file).unwrap().scope;
                let scope = self.scopes.index_mut(file_scope);
                let entry = scope.by_name.entry(alias.clone()).or_default();
                push_unique_def(entry, dep_def_id);
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

    fn find_name_in_file(&self, root: SourceRootId, file: FileId, name: &Name) -> Option<Vec<DefId>> {
        let file_data = self.file_data(root, file);
        let local_scope = ScopeRef { root, scope: file_data.scope };
        self.find_name_in_scope(local_scope, name).or_else(|| {
            file_data
                .imported_scopes
                .iter()
                .find_map(|scope| self.find_name_in_scope(*scope, name))
        })
    }

    fn find_name_in_scope(&self, scope: ScopeRef, name: &Name) -> Option<Vec<DefId>> {
        if self.root == scope.root {
            self.scopes.index(scope.scope).by_name.get(name).cloned()
        } else {
            self.db.root_def_map(scope.root).scopes.index(scope.scope).by_name.get(name).cloned()
        }
    }
}

fn push_unique_scope(scopes: &mut Vec<ScopeRef>, scope: ScopeRef) {
    if !scopes.contains(&scope) {
        scopes.push(scope);
    }
}

fn push_unique_def(defs: &mut Vec<DefId>, def: DefId) {
    if !defs.contains(&def) {
        defs.push(def);
    }
}

fn extend_unique_defs(defs: &mut Vec<DefId>, new_defs: impl IntoIterator<Item = DefId>) {
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



