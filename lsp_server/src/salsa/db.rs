use parking_lot::Mutex;
use triomphe::Arc;

use camino::Utf8PathBuf;
use lsp_types::TextDocumentContentChangeEvent;
use ropey::Rope;
use salsa::{self, Durability, Setter};
use smol_str::SmolStr;
use tree_sitter::{InputEdit, Node};

use super::files::Files;
use super::incremental_parser::IncrementalParser;
use super::root_db::RootDatabase;
use crate::ast::kinds::NodeKind;
use crate::ast::{self, AstNode, NodeRange};
use crate::ir::def_map::DefId;
use crate::hir::body_map::ByteOffset;
use crate::loader::SourceRootBundle;
use crate::utilities::{byte_to_point, to_rope_idx};
use crate::workspace::{PackageConfig, PackageId, Workspace};


pub type FileId = File;
#[salsa::input]
#[derive(Debug)]
pub struct File {
    pub text: Rope,
    #[returns(ref)]
    pub path: Utf8PathBuf,
}



///////////////SOURCEROOT///////////////////
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SourceRootData {
    pub package_id: PackageId,
    pub files: Arc<[FileId]>,
    pub is_dependency: bool,
}

//Triomphe::Arc does NOT impl Default for [T] unlike std::Arc
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
    #[returns(ref)]
    pub source_root: Arc<SourceRootData>,
}



//////////////PACKAGE//////////////
#[derive(Clone, PartialEq)]
pub struct Package {
    pub root: Utf8PathBuf,
    pub config: PackageConfig,
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
    // TsParser is "Send" but "!Sync", however, salsa can run queries in parallel so we need Sync
    // queries don't support "&mut dyn"(i.e db mutation) but parser requires mutable borrows so, interior mutability!!
    pub parser: Mutex<IncrementalParser>,
}

#[salsa::db]
impl salsa::Database for SalsaDatabase {}


impl SalsaDatabase {

    pub(crate) fn new(workspace: Workspace, roots: Vec<SourceRootBundle> ) -> Self {
        let mut salsa = SalsaDatabase::default();
        salsa.files = Arc::new(Files::new(&mut salsa, roots));

        let packages: Vec<Package> = workspace.packages.into_iter().map(|p| Package {
            root: p.root,
            config: p.config,
        }).collect();
        
        // Seed the singleton Packages input
        let _ = Packages::builder(packages.into())
            .durability(Durability::MEDIUM)
            .new(&salsa);


        salsa
    }


    pub(crate) fn file(&self, path: &Utf8PathBuf) -> Option<File> {
        self.files.get(path)
    }

    pub(crate) fn source_root_files(&self, source_root_id: SourceRootId) -> &[File] {
        &source_root_id.source_root(self).files
    }

    pub(crate) fn source_root_for_file(&self, file_id: FileId) -> Option<SourceRootId> {
        Some(self.file_source_root(file_id))
    }

    pub fn resolve_path(&self, file: FileId, rel_path: &str) -> Option<FileId> {
        self.resolve_to_file(file, rel_path)
    }

    pub(crate) fn get_package(&self, root_id: SourceRootId) -> &Package {
        self.package_config(root_id)
    }    

    pub fn set_packages(&mut self, packages: Vec<Package>) {
        Packages::get(self).set_packages(self).to(packages.into());
    }

    pub fn rope(&self, file: File) -> Rope {
        file.text(self)
    }

    pub fn convert(&self, path: &Utf8PathBuf, position: lsp_types::Position) -> (File, ByteOffset) {
        let file = self.file(path).unwrap();
        let rope = self.rope(file);
        let char_idx = to_rope_idx(&rope, position);
        let byte_offset = rope.char_to_byte(char_idx);
        (file, byte_offset as u32)
    }

    /// Returns the identifier text at the given LSP position, if any.
    pub fn identifier_at_position(
        &self,
        path: &Utf8PathBuf,
        position: lsp_types::Position,
    ) -> Option<SmolStr> {
        let node = self.node_at_position(path, position)?;
        //the innermost named nodes are
        //Identifier: for vars/user types
        //Primitive_type: for primitives
        //literals: we ignore for now
        //and builtin stuffs eg visibility, state mutability etc
        if node.node().kind_id() == NodeKind::IDENTIFIER {
            Some(node.text().into())
        } else {
            None
        }
    }

    pub fn node_at_position(&self, path: &Utf8PathBuf, position: lsp_types::Position) -> Option<AstNode> {
        let (file, offset)  = self.convert(path, position);
        self.node_at(file, offset)
    }

    pub fn node_at(&self, file: File, offset: ByteOffset) -> Option<AstNode> {
        assert!(offset > 0);
        let root = self.root(file);
        let range = NodeRange { start: offset, end: offset };
        root.named_child_node(range)
    }


    pub fn enclosing_containers(&self, node: Node, file: File) -> Vec<DefId> {
        // Single top-down traversal from root to leaf, recording path

        //TS parent walking is very in-efficient. apparently nodes don't keep parent pointers
        //so every parent call walks the ast from root!.
        let root = self.root(file);
        let root_id = DefId::File(file);
        let mut path = vec![root_id];
        let mut current = root.node();
        loop {
            let next = current.child_with_descendant(node).unwrap();
            if next.id() == node.id() { break; }
            crate::map_def_id! {
                match self.ast_id_map(file), file, next, path => {
                    CONTRACT_DEFINITION => Contract => ContractId,
                    INTERFACE_DEFINITION => Interface => InterfaceId,
                    LIBRARY_DEFINITION => Library => LibraryId,
                    IMPORT_DIRECTIVE => Import => ImportId => break,
                    USING_DIRECTIVE => Using => UsingId => break,
                    USER_DEFINED_TYPE_DEFINITION => Udvt => UdvtId => break,
                    FUNCTION_DEFINITION => Function => FunctionId => break,
                    MODIFIER_DEFINITION => Modifier => ModifierId => break,
                    STRUCT_DEFINITION => Struct => StructId => break,
                    ENUM_DEFINITION => Enum => EnumId => break,
                    EVENT_DEFINITION => Event => EventId => break,
                    ERROR_DEFINITION => Error => ErrorId => break,
                    STATE_VAR_DECLARATION => Var => VarId => break,
                    CONST_VAR_DECLARATION => Var => VarId => break,
                }
            }
            current = next;
        }
        path
    }

    pub fn open(&mut self, path: Utf8PathBuf, text: String) {
        let rope = Rope::from_str(&text);
        if let Some(file) = self.file(&path) {
            file.set_text(self).to(rope);
        } else {
            // New file not discovered during workspace load — create it.
            // Source root assignment is deferred; the file will resolve
            // imports via path-based package lookup.
            // we can't yet determine thr source root of a new file, so we do nothing it for now
            //let _file = File::new(&mut self, rope, path);
        }
    }

    pub fn apply_changes(&mut self, path: Utf8PathBuf, changes: Vec<TextDocumentContentChangeEvent>) {
        let Some(file) = self.file(&path) else { return; };

        for change in changes {
            let mut rope = file.text(self);
            let start = to_rope_idx(&rope, change.range.unwrap().start);
            let end = to_rope_idx(&rope, change.range.unwrap().end);

            let start_byte = rope.char_to_byte(start);
            let end_byte = rope.char_to_byte(end);
            let start_position = byte_to_point(&rope, start_byte);//we can provide row/line here no need too recalculate
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
                new_end_position: byte_to_point(&rope, new_end_byte),//but here i think we have to calculate
            };

            file.set_text(self).to(rope);
            self.parser.lock().apply_tree_edit(file, &edit);
        }
    }
}



#[macro_export]
macro_rules! map_def_id {

    (//Arm#1
        match $id_map:expr, $file:expr, $node:expr, $container:expr => {
            $($node_kind:ident => $kind:ident => $id_kind:ident $(=> $break:ident)?),* $(,)?
        }
    ) => {
        match $node.kind_id().into() {
            $(NodeKind::$node_kind => {
                let id = $id_map.id_of_node::<ast::$kind>($node).unwrap();
                let def_id = $crate::ir::def_map::DefId::$kind($crate::ast::$id_kind{file:$file,id});
                $container.push(def_id);
                $($break;)?
            },)*
            _ => {}
        }
    };
}