

use camino::Utf8PathBuf;
use rustc_hash::FxHashMap;
use salsa::{Durability, Setter};
use triomphe::Arc;

use crate::loader::SourceRootBundle;
use crate::utilities::resolve_import;
use super::db::{File, FileId, Packages, SourceRootData, SourceRootId};
use super::root_db::RootDatabase;

#[derive(Default)]
pub struct Files {
    file: FxHashMap<Utf8PathBuf, File>,//TODO use fxDashMaps
    pub(crate) file_source_root: FxHashMap<FileId, SourceRootId>,
}

impl Files {
    pub fn new(db: &mut dyn salsa::Database, roots: Vec<SourceRootBundle>) -> Files {
        let mut file = FxHashMap::default();
        let mut file_source_root = FxHashMap::default();
        for root in roots {
            //place holder root to generate Id, no files yet
            let source_root_id = SourceRootId::new(db, Default::default());

            let files = root.files.into_iter().map(|f| {
                let durability  = if root.is_dependency { Durability::HIGH } else { Durability::LOW };
                let file_text = File::builder(f.text.into(), f.path.clone()).durability(durability).new(db);
                file.insert(f.path, file_text);
                file_source_root.insert(file_text, source_root_id);
                file_text
            }).collect::<Vec<_>>();

            //fill root with files
            source_root_id.set_source_root(db).with_durability(Durability::HIGH).to(Arc::new(SourceRootData {
                package_id: root.package_id,
                files: files.into(),
                is_dependency: root.is_dependency,
            }));
        }

        Files { file, file_source_root }
    }

    pub fn get(&self, path: &Utf8PathBuf) -> Option<File> {
        self.file.get(path).cloned()
    }

    pub fn resolve_to_file(&self, db: &dyn RootDatabase, from: FileId, rel_path: &str) -> Option<FileId> {
        let source_root = self.file_source_root.get(&from).unwrap();
        let source_root_data = source_root.source_root(db);
        let package = &Packages::get(db).packages(db)[source_root_data.package_id.0];
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

