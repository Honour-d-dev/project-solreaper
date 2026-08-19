

use camino::{Utf8Path, Utf8PathBuf};
use rustc_hash::FxHashMap;
use salsa::{Durability, Setter};
use triomphe::Arc;

use crate::loader::SourceRootBundle;
use crate::utilities::resolve_import;
use super::db::{File, FileId, Packages, SourceRoot, SourceRootId};
use super::root_db::RootDatabase;

#[derive(Default)]
pub struct Files {
    file: FxHashMap<Utf8PathBuf, File>,//TODO use fxDashMaps
    source_roots: Vec<SourceRoot>,
    pub(crate) file_source_root: FxHashMap<FileId, SourceRootId>,
}

impl Files {
    pub fn new(db: &mut dyn salsa::Database, roots: Vec<SourceRootBundle>) -> Files {
        let mut file = FxHashMap::default();
        let mut file_source_root = FxHashMap::default();
        let mut source_roots = Vec::new();
        for root in roots {
            let is_dependency = root.is_dependency;
            let source_root = SourceRootId::builder(
                root.root,
                root.package_id,
                Arc::from(Vec::new()),
                is_dependency,
            )
            .durability(Durability::HIGH)
            .new(db);
            source_roots.push(source_root);

            let files = root.files.into_iter().map(|f| {
                let durability = if is_dependency { Durability::HIGH } else { Durability::LOW };
                let file_text = File::builder(f.text, f.path.clone()).durability(durability).new(db);
                file.insert(f.path, file_text);
                file_source_root.insert(file_text, source_root);
                file_text
            }).collect::<Vec<_>>();

            source_root.set_files(db)
                .with_durability(Durability::HIGH)
                .to(files.into());
        }

        Files { file, file_source_root, source_roots }
    }

    pub fn get(&self, path: &Utf8PathBuf) -> Option<File> {
        self.file.get(path).cloned()
    }

    pub fn source_root_for_path(&self, db: &dyn RootDatabase, path: &Utf8Path) -> Option<SourceRootId> {
        self.source_roots.iter()
            .filter(|root| path.starts_with(root.root(db)))
            .max_by_key(|root| root.root(db).components().count())
            .copied()
    }

    pub fn insert_file(&mut self, path: Utf8PathBuf, file: File, source_root_id: SourceRootId) {
        self.file.insert(path, file);
        self.file_source_root.insert(file, source_root_id);
    }

    pub fn resolve_to_file(&self, db: &dyn RootDatabase, from: FileId, rel_path: &str) -> Option<FileId> {
        let source_root = self.file_source_root.get(&from).unwrap();
        let package = &Packages::get(db).packages(db)[source_root.package_id(db).0];
        let resolved_path = resolve_import(
            from.path(db),
            rel_path,
            &package.root,
            &package.config.remappings,
        )
        .ok()?;
        
        self.file.get(&resolved_path).copied()
    }
}

