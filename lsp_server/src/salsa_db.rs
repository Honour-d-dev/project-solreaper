use parking_lot::Mutex;
use triomphe::Arc;


use camino::Utf8PathBuf;
use lsp_types::TextDocumentContentChangeEvent;
use ropey::Rope;
use rustc_hash::{FxHashMap};
use salsa::{self, Durability, Setter};
use smol_str::SmolStr;
use tree_sitter::{InputEdit, Parser, Tree};

use crate::{
    ast::{
        Ast, AstIdMap, ast_id::PtrRange, kinds::NodeKind
    },
    def_map::{
        Collector, DefMap
    },
    item_tree::{
        ItemTree, Lowerer
    },
    loader::SourceRootBundle,
    utilities::{byte_to_point, resolve_import, to_rope_idx},
    workspace::{
        PackageConfig, PackageId, Workspace
    }
};



#[derive(Clone, PartialEq)]
pub struct Package {
    pub root: Utf8PathBuf,
    pub config: PackageConfig,
}



#[derive(Default)]
pub struct Files {
    file: FxHashMap<Utf8PathBuf, File>,//TODO use fxDashMaps
    file_source_root: FxHashMap<FileId, SourceRootId>,
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

    fn resolve_to_file(&self, db: &dyn RootDatabase, from: FileId, rel_path: &str) -> Option<FileId> {
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







pub(crate) struct IncrementalParser {
    pub parser: Parser,
    cache: FxHashMap<File, Tree>
}

impl Default for IncrementalParser {
    fn default() -> Self {
        Self::new()
    }
}

impl IncrementalParser {
    fn new() -> Self {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_solidity::LANGUAGE.into()).unwrap();
        Self {
            parser,
            cache: FxHashMap::default(),
        }
    }
    
    //TODO fn open_file
    //TODO fn close_file

    fn edit_tree(&mut self, file: File, edit: &InputEdit) {
        self.cache.get_mut(&file).unwrap().edit(edit);
    }

    fn parse(&mut self, db: &dyn RootDatabase, file: File) -> Tree {
        let old_tree = self.cache.get(&file);
        let rope = file.text(db);
        let new_tree = self.parser.parse_with_options( &mut |b, _| {
            let (chunk, b1, _, _) = rope.chunk_at_byte(b);
            &chunk[(b - b1)..]
        }, old_tree, None).unwrap();
        self.cache.insert(file, new_tree.clone());
        new_tree
    }
}







pub(crate) struct SalsaDb {
    db: SalsaDatabase,
}

impl SalsaDb {
    pub(crate) fn new(workspace: Workspace ,roots: Vec<SourceRootBundle>) -> Self {
        let db = SalsaDatabase::new(roots);

        let packages: Vec<Package> = workspace.packages.into_iter().map(|p| Package {
            root: p.root,
            config: p.config,
        }).collect();
        
        // Seed the singleton Packages input
        let _ = Packages::builder(packages.into())
            .durability(Durability::MEDIUM)
            .new(&db);
        
        Self {
            db,
        }
    }

    pub(crate) fn file(&self, path: &Utf8PathBuf) -> Option<File> {
        self.db.files.get(path)
    }

    pub(crate) fn source_root_files(&self, source_root_id: SourceRootId) -> &[File] {
        &source_root_id.source_root(&self.db).files
    }

    pub(crate) fn source_root_for_file(&self, file_id: FileId) -> Option<SourceRootId> {
        Some(self.db.file_source_root(file_id))
    }

    pub fn resolve_path(&self, file: FileId, rel_path: &str) -> Option<FileId> {
        self.db.resolve_to_file(file, rel_path)
    }

    pub(crate) fn get_package(&self, root_id: SourceRootId) -> &Package {
        self.db.package_config(root_id)
    }    

    pub fn set_packages(&mut self, packages: Vec<Package>) {
        Packages::get(&self.db).set_packages(&mut self.db).to(packages.into());
    }

    pub fn ast(&self, file: File) -> Arc<Ast> {
        self.db.parse(file)
    }

    pub fn rope(&self, file: File) -> Rope {
        file.text(&self.db)
    }

    /// Returns the identifier text at the given LSP position, if any.
    pub fn identifier_at_position(
        &self,
        file: File,
        position: lsp_types::Position,
    ) -> Option<SmolStr> {
        let rope = self.rope(file);
        let char_idx = to_rope_idx(&rope, position);
        let byte_offset = rope.char_to_byte(char_idx);

        let ast = self.ast(file);
        let range = PtrRange { start: byte_offset as u32, end: (byte_offset + 1) as u32 };
        let node = ast.node(range)?;
        if node.node().kind_id() == NodeKind::IDENTIFIER.as_u16() {
            Some(node.text().into())
        } else {
            None
        }
    }

    pub fn open(&mut self, path: Utf8PathBuf, text: String) {
        let rope = Rope::from_str(&text);
        if let Some(file) = self.file(&path) {
            file.set_text(&mut self.db).to(rope);
        } else {
            // New file not discovered during workspace load — create it.
            // Source root assignment is deferred; the file will resolve
            // imports via path-based package lookup.
            let _file = File::new(&mut self.db, rope, path);
        }
    }

    pub fn apply_changes(&mut self, path: Utf8PathBuf, changes: Vec<TextDocumentContentChangeEvent>) {
        let Some(file) = self.file(&path) else { return; };

        for change in changes {
            let mut rope = file.text(&self.db);
            let start = to_rope_idx(&rope, change.range.unwrap().start);
            let end = to_rope_idx(&rope, change.range.unwrap().end);

            let start_byte = rope.char_to_byte(start);
            let end_byte = rope.char_to_byte(end);
            let start_position = byte_to_point(&rope, start_byte);
            let end_position = byte_to_point(&rope, end_byte);

            rope.remove(start..end);
            rope.insert(start, &change.text);

            let new_end_byte = start_byte + change.text.len();

            let edit = InputEdit {
                start_byte,
                old_end_byte: end_byte,
                new_end_byte,
                start_position,
                old_end_position: end_position,
                new_end_position: byte_to_point(&rope, new_end_byte),
            };

            file.set_text(&mut self.db).to(rope);
            self.db.parser.lock().edit_tree(file, &edit);
        }
    }
}








pub type FileId = File;
#[salsa::input]
pub(crate) struct File {
    text: Rope,
    #[returns(ref)]
    path: Utf8PathBuf,
}


#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SourceRootData {
    pub package_id: PackageId,
    pub files: Arc<[FileId]>,
    pub is_dependency: bool,
}

//Triomphe::Arc does NOT impl Default for [T] unlike std
impl Default for SourceRootData {
    fn default() -> Self {
        Self {
            package_id: PackageId::default(),
            files: Arc::from(Vec::new()),
            is_dependency: false,
        }
    }
}

pub type SourceRootId = SourceRoot;
#[salsa::input]
pub(crate) struct SourceRoot {
    #[returns(ref)]//why??
    pub source_root: Arc<SourceRootData>,
}




#[salsa::input(singleton)]
pub struct Packages {
    #[returns(ref)]
    pub packages: Arc<[Package]>
}


#[salsa::db]
#[derive(Default)]
pub(crate) struct SalsaDatabase {
    storage: salsa::Storage<Self>,
    pub files: Arc<Files>,
    // Mutex fixes 2 issues:
    // TsParser is "Send" but "!Sync", however, salsa can run queries in parallel so we need to make it Sync
    // And queries don't support "&mut dyn" but parser requires mutable borrows so interior mutability fixes that
    pub parser: Mutex<IncrementalParser>,
}

#[salsa::db]
impl salsa::Database for SalsaDatabase {}


impl SalsaDatabase {
    pub(crate) fn new(roots: Vec<SourceRootBundle> ) -> Self {
        let mut salsa = SalsaDatabase::default();
        salsa.files = Arc::new(Files::new(&mut salsa, roots));
        salsa
    }
}

#[salsa::db]
pub trait RootDatabase: salsa::Database {
    fn text(&self, file: FileId) -> Arc<str>;
    fn path(&self, file: FileId) -> &Utf8PathBuf;
    fn file_source_root(&self, file: FileId) -> SourceRootId;
    fn package_config(&self, root_id: SourceRootId) -> &Package;

    fn parse_file(&self, file:File) -> Tree;
    fn parse(&self, file: File) -> Arc<Ast>;
    fn ast_id_map(&self, file: File) -> Arc<AstIdMap>;
    fn item_tree(&self, file: File) -> &ItemTree;
    fn root_def_map(&self, root_id:SourceRootId) -> Arc<DefMap>;
    fn resolve_to_file( &self, from: FileId, import_path: &str) -> Option<FileId>;
}

#[salsa::db]
impl RootDatabase for SalsaDatabase {

    fn text(&self, file: FileId) -> Arc<str> {
        text(self,file)
    }

    fn path(&self, file: FileId) -> &Utf8PathBuf {
        file.path(self)
    }

    fn file_source_root(&self, file:FileId) -> SourceRootId {
        self.files.file_source_root.get(&file).cloned().unwrap()
    }

    fn package_config(&self, root: SourceRootId) -> &Package {
        package_config(self, root)
    }

    fn parse_file(&self, file:File) -> Tree {
        self.parser.lock().parse(self,file)
    }

    fn parse(&self, file: File) -> Arc<Ast> {
        parse(self, file)
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

    fn resolve_to_file(&self, from: FileId, rel_path: &str ) -> Option<FileId> {
        self.files.resolve_to_file(self, from, rel_path)
    }
}

// Reference: correct tracked-method shape in Salsa.
// - impl block must be #[salsa::tracked]
// - first parameter must be `self` by value
// - method must take a db parameter
// #[salsa::tracked]
// pub(crate) struct TrackedMethodExample<'db> {
//     pub key: u32,
// }

// #[salsa::tracked]
// impl<'db> TrackedMethodExample<'db> {
//     #[salsa::tracked]
//     pub(crate) fn demo(self, db: &dyn salsa::Database) -> usize {
//         self.key(db) as usize
//     }
// }

#[salsa::tracked]//manage lru
fn text(db: &dyn salsa::Database, file: File) -> Arc<str> {
    file.text(db).to_string().into()
}

#[salsa::tracked(returns(ref))]
fn package_config(db: &dyn salsa::Database, root: SourceRootId) -> Package {
    let package_id = root.source_root(db).package_id;
    Packages::get(db).packages(db).get(package_id.0).cloned().unwrap()
}

#[salsa::tracked]
fn parse(db: &dyn RootDatabase, file: File) -> Arc<Ast> {
    // !untracked state/data used here to enable incremental reparsing
    // However, the untracked data is kept in sync with salsa by the Incremental parser.
    let tree = db.parse_file(file);
    Arc::new(Ast::new(tree, text(db, file)))
}

#[salsa::tracked]
fn ast_id_map(db: &dyn RootDatabase, file: File) ->  Arc<AstIdMap> {
    let ast = parse(db, file);
    Arc::new(AstIdMap::new(&ast.root()))
}

#[salsa::tracked(returns(ref))]
fn item_tree(db: &dyn RootDatabase, file: File) -> ItemTree {
    Lowerer::lower(db, file)
}

#[salsa::tracked]
fn root_def_map(db: &dyn RootDatabase, root: SourceRootId) -> Arc<DefMap> {
    Arc::new(Collector::collect_defmap(db, root))
}



