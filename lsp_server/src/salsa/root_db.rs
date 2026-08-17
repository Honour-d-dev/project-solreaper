use triomphe::Arc;

use camino::Utf8PathBuf;
use ropey::Rope;
use tree_sitter::Tree;

use crate::ast::{Ast, AstIdMap, AstNode};
use crate::ir::def_map::{Collector, DefMap};
use crate::ir::item_tree::{ItemTree, Lowerer};
use super::db::{File, FileId, Package, Packages, SalsaDatabase, SourceRootId};

//caching this so we only do it once per revision. but does this means we store 2 copies of all files?
#[salsa::tracked]//manage lru
pub fn text(db: &dyn salsa::Database, file: File) -> Arc<str> {
    file.text(db).to_string().into()
}

#[salsa::tracked(returns(ref))]
pub fn package_config(db: &dyn salsa::Database, root: SourceRootId) -> Package {
    let package_id = root.source_root(db).package_id;
    Packages::get(db).packages(db).get(package_id.0).cloned().unwrap()
}

#[salsa::tracked]
pub fn parse(db: &dyn RootDatabase, file: File) -> Arc<Ast> {
    tracing::debug!(?file, "parse: cache miss");
    // !untracked state/data used here to enable incremental reparsing
    // However, the untracked data is kept in sync with salsa by the Incremental parser.
    let tree = db.ts_incremental_parse(file);
    Arc::new(Ast::new(tree, text(db, file)))
}

#[salsa::tracked]
pub fn ast_id_map(db: &dyn RootDatabase, file: File) -> Arc<AstIdMap> {
    tracing::debug!(?file, "ast_id_map: cache miss");
    let ast = parse(db, file);
    Arc::new(AstIdMap::new(&ast.root()))
}

#[salsa::tracked(returns(ref))]
pub fn item_tree(db: &dyn RootDatabase, file: File) -> ItemTree {
    tracing::debug!(?file, "item_tree: cache miss");
    Lowerer::lower(db, file)
}

#[salsa::tracked]
pub fn root_def_map(db: &dyn RootDatabase, root: SourceRootId) -> Arc<DefMap> {
    tracing::debug!(?root, "root_def_map: cache miss");
    Arc::new(Collector::collect_defmap(db, root))
}

#[salsa::db]
pub trait RootDatabase: salsa::Database {
    fn text(&self, file: FileId) -> Arc<str>;
    fn path(&self, file: FileId) -> &Utf8PathBuf;
    fn rope(&self, file: FileId) -> Rope;
    fn file_source_root(&self, file: FileId) -> SourceRootId;
    fn package_config(&self, root_id: SourceRootId) -> &Package;

    fn ts_incremental_parse(&self, file: File) -> Tree;
    fn ast(&self, file: File) -> Arc<Ast>;
    fn root(&self, file: File) -> AstNode;
    fn ast_id_map(&self, file: File) -> Arc<AstIdMap>;
    fn item_tree(&self, file: File) -> &ItemTree;
    fn root_def_map(&self, root: SourceRootId) -> Arc<DefMap>;
    fn resolve_to_file(&self, from: FileId, import_path: &str) -> Option<FileId>;
}

#[salsa::db]
impl RootDatabase for SalsaDatabase {
    fn text(&self, file: FileId) -> Arc<str> {
        text(self, file)
    }

    fn path(&self, file: FileId) -> &Utf8PathBuf {
        file.path(self)
    }

    fn rope(&self, file: FileId) -> Rope {
        file.text(self)
    }

    fn file_source_root(&self, file: FileId) -> SourceRootId {
        self.files.file_source_root.get(&file).cloned().unwrap()
    }

    fn package_config(&self, root: SourceRootId) -> &Package {
        package_config(self, root)
    }

    fn ts_incremental_parse(&self, file: File) -> Tree {
        self.parser.lock().parse(self, file)
    }

    fn ast(&self, file: File) -> Arc<Ast> {
        parse(self, file)
    }

    fn root(&self, file: File) -> AstNode {
        self.ast(file).root()
    }

    fn ast_id_map(&self, file: File) -> Arc<AstIdMap> {
        ast_id_map(self, file)
    }

    fn item_tree(&self, file: File) -> &ItemTree {
        item_tree(self, file)
    }

    fn root_def_map(&self, root: SourceRootId) -> Arc<DefMap> {
        root_def_map(self, root)
    }

    fn resolve_to_file(&self, from: FileId, rel_path: &str) -> Option<FileId> {
        self.files.resolve_to_file(self, from, rel_path)
    }
}
