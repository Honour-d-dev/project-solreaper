use std::{collections::VecDeque, sync::Arc};

use camino::Utf8PathBuf;
use rustc_hash::{FxHashMap, FxHashSet};
use salsa::{self, Setter};
use tree_sitter::Range;

use crate::lowering::{File as LoweredFile, Symbol};

pub(crate) struct AnalysisHost {
    db: RootDatabase,
    files: FxHashMap<Utf8PathBuf, SalsaFile>,
}

fn symbol_name(symbol: &Symbol) -> Option<&str> {
    match symbol {
        Symbol::Contract(contract) => Some(contract.name.as_str()),
        Symbol::Function(function) => Some(function.name.as_str()),
        Symbol::Variable(variable) => variable.name.as_deref(),
    }
}

fn symbol_range(symbol: &Symbol) -> Range {
    match symbol {
        Symbol::Contract(contract) => contract.range,
        Symbol::Function(function) => function.range,
        Symbol::Variable(variable) => variable.range,
    }
}

impl AnalysisHost {
    pub(crate) fn new() -> Self {
        Self {
            db: RootDatabase::new(),
            files: FxHashMap::default(),
        }
    }

    pub(crate) fn file(&self, path: &Utf8PathBuf) -> Option<&SalsaFile> {
        self.files.get(path)
    }

    pub(crate) fn set_lowered_file(&mut self, lowered: LoweredFile) -> SalsaFile {
        let path = lowered.path.clone();
        let lowered = Arc::new(lowered);

        if let Some(existing) = self.files.get(&path).copied() {
            existing.set_lowered(&mut self.db).to(lowered);
            existing
        } else {
            let salsa_file = SalsaFile::new(&self.db, lowered, path.clone());
            self.files.insert(path, salsa_file);
            salsa_file
        }
    }

    pub(crate) fn insert(&mut self, lowered: LoweredFile) -> SalsaFile {
        self.set_lowered_file(lowered)
    }

    pub(crate) fn insert_multiple(&mut self, lowered: Vec<LoweredFile>) -> Vec<SalsaFile> {
        lowered.into_iter().map(|lowered| self.set_lowered_file(lowered)).collect()
    }

    //Libs are not eager loaded so if a file imports a lib its silently ignored
    //I need to rethink the architecture here
    pub(crate) fn resolve_symbol(
        &self,
        path: &Utf8PathBuf,
        identifier: &str,
        node_range: Range,
    ) -> Option<Symbol> {
        let root = *self.file(path)?;

        let mut best_containing: Option<(usize, Symbol)> = None;
        let mut best_preceding: Option<(usize, Symbol)> = None;
        let mut fallback: Option<Symbol> = None;

        for symbol in self.visible_symbols_for_root(root) {
            if symbol_name(&symbol) != Some(identifier) {
                continue;
            }

            let range = symbol_range(&symbol);
            if range.start_byte <= node_range.start_byte && node_range.end_byte <= range.end_byte {
                let span = range.end_byte.saturating_sub(range.start_byte);
                match best_containing {
                    Some((best_span, _)) if best_span <= span => {}
                    _ => best_containing = Some((span, symbol.clone())),
                }
            } else if range.start_byte <= node_range.start_byte {
                // For declarations (especially locals/params) whose node range does not
                // contain usage sites, prefer the closest declaration before the use.
                let distance = node_range.start_byte - range.start_byte;
                match best_preceding {
                    Some((best_distance, _)) if best_distance <= distance => {}
                    _ => best_preceding = Some((distance, symbol.clone())),
                }
            } else if fallback.is_none() {
                fallback = Some(symbol.clone());
            }
        }

        best_containing
            .map(|(_, symbol)| symbol)
            .or_else(|| best_preceding.map(|(_, symbol)| symbol))
            .or(fallback)
    }

    pub(crate) fn collect_imports(&self, root: SalsaFile) -> Vec<SalsaFile> {
        let mut seen = FxHashSet::<Utf8PathBuf>::default();
        let mut queue = VecDeque::from([root]);
        let mut out = Vec::new();

        while let Some(file) = queue.pop_front() {
            let path = file.path(&self.db).clone();
            if !seen.insert(path) {
                continue;
            }
            out.push(file);

            for import_path in direct_imports(&self.db, file).iter() {
                if let Some(imported) = self.files.get(import_path).copied() {
                    queue.push_back(imported);
                }
            }
        }

        out
    }

    pub(crate) fn collect_missing_deps(&self, root: SalsaFile) -> Vec<Utf8PathBuf> {
        let mut seen = FxHashSet::<Utf8PathBuf>::default();
        let mut queue = VecDeque::from([root]);
        let mut out = Vec::new();

        while let Some(file) = queue.pop_front() {
            let path = file.path(&self.db).clone();
            if !seen.insert(path) {
                continue;
            }

            for import_path in direct_imports(&self.db, file).iter() {
                if let Some(imported) = self.files.get(import_path).copied() {
                    queue.push_back(imported);
                } else {
                    out.push(import_path.clone());
                }
            }
        }

        out
    }

    pub(crate) fn visible_symbols_for_root(&self, root: SalsaFile) -> Vec<Symbol> {
        let mut all = Vec::new();
        for file in self.collect_imports(root) {
            all.extend(file_symbols(&self.db, file).iter().cloned());
        }
        all
    }
}
#[salsa::input]
pub(crate) struct SalsaFile {
    pub lowered: Arc<LoweredFile>,
    #[returns(ref)]
    pub path: Utf8PathBuf,
}

#[salsa::db]
pub(crate) struct RootDatabase {
    storage: salsa::Storage<Self>,
}

#[salsa::db]
impl salsa::Database for RootDatabase {}

impl RootDatabase {
    pub(crate) fn new() -> Self {
        RootDatabase {
            storage: salsa::Storage::default(),
        }
    }
}

// Reference: correct tracked-method shape in Salsa.
// - impl block must be #[salsa::tracked]
// - first parameter must be `self` by value
// - method must take a db parameter
#[allow(dead_code)]
#[salsa::tracked]
pub(crate) struct TrackedMethodExample<'db> {
    pub key: u32,
}

#[salsa::tracked]
impl<'db> TrackedMethodExample<'db> {
    #[salsa::tracked]
    pub(crate) fn demo(self, db: &dyn salsa::Database) -> usize {
        self.key(db) as usize
    }
}

#[salsa::tracked]
fn direct_imports(
    db: &dyn salsa::Database,
    file: SalsaFile,
) -> Arc<[Utf8PathBuf]> {
    file.lowered(db)
        .imports
        .iter()
        .map(|imp| imp.path.clone())
        .collect::<Vec<_>>()
        .into()
}

// optional: per-file symbol decl extraction from lowered IR
#[salsa::tracked]
fn file_symbols(
    db: &dyn salsa::Database,
    file: SalsaFile,
) -> Arc<[Symbol]> {
    let lowered = file.lowered(db);
    let mut out = Vec::new();

    for contract in lowered.contracts.iter() {
        out.push(Symbol::Contract(contract.clone()));

        for state_var in contract.state_vars.iter() {
            out.push(Symbol::Variable(state_var.clone()));
        }

        for function in contract.functions.iter() {
            out.push(Symbol::Function(function.clone()));

            for parameter in function.parameters.iter() {
                out.push(Symbol::Variable(parameter.clone()));
            }

            for local_var in function.local_vars.iter() {
                out.push(Symbol::Variable(local_var.clone()));
            }
        }
    }

    for function in lowered.free_functions.iter() {
        out.push(Symbol::Function(function.clone()));

        for parameter in function.parameters.iter() {
            out.push(Symbol::Variable(parameter.clone()));
        }

        for local_var in function.local_vars.iter() {
            out.push(Symbol::Variable(local_var.clone()));
        }
    }

    out.into()
}
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct ByteOffset(pub usize);

/// Query-shape sketch now that parsing/editing happen above Salsa.
#[allow(dead_code)]
pub(crate) trait SalsaQueryPlan {
    fn lowered_file(&self, file: SalsaFile) -> Arc<LoweredFile>;
    fn direct_imports(&self, file: SalsaFile) -> Arc<[Utf8PathBuf]>;
    fn file_symbols(&self, file: SalsaFile) -> Arc<[Symbol]>;
    fn resolve_symbol(
        &self,
        file: SalsaFile,
        identifier: &str,
        offset: ByteOffset,
    ) -> Option<Symbol>;
}
