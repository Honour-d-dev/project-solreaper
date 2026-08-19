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
use crate::utilities::{byte_to_point, normalize_path, to_rope_idx};
use crate::workspace::{Package, PackageConfig, PackageId, Workspace};


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


pub type FileId = File;
#[salsa::input]
#[derive(Debug)]
pub struct File {
    pub text: Rope,
    #[returns(ref)]
    pub path: Utf8PathBuf,
}

pub type SourceRootId = SourceRoot;
#[salsa::input]
#[derive(Debug)]
pub(crate) struct SourceRoot {
    #[returns(ref)]
    pub root: Utf8PathBuf,
    pub package_id: PackageId,
    #[returns(ref)]
    pub files: Arc<[FileId]>,
    pub is_dependency: bool,
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
    pub files: Files,
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
        salsa.files = Files::new(&mut salsa, roots);

        // Seed the singleton Packages input
        let _ = Packages::builder(workspace.packages.into())
            .durability(Durability::MEDIUM)
            .new(&salsa);


        salsa
    }


    pub(crate) fn file(&self, path: &Utf8PathBuf) -> Option<File> {
        self.files.get(path)
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

    pub fn node_at_position(&self, path: &Utf8PathBuf, position: lsp_types::Position) -> Option<AstNode> {
        let (file, offset)  = self.convert(path, position);
        self.named_node_at(file, offset)
    }

    pub fn named_node_at(&self, file: File, offset: ByteOffset) -> Option<AstNode> {
        let root = self.root(file);
        let range = NodeRange { start: offset, end: offset };
        root.named_child_node(range)
    }

    pub fn node_at(&self, file: File, offset: ByteOffset) -> Option<AstNode> {
        let root = self.root(file);
        let range = NodeRange { start: offset, end: offset };
        root.child_node(range)
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
            let Some(next)  = current.child_with_descendant(node) else { break;};
            if next.id() == node.id() { break; }
            map_def_id! {
                match self.ast_id_map(file), file, next, path => {
                    CONTRACT_DEFINITION => Contract => ContractId,
                    INTERFACE_DEFINITION => Interface => InterfaceId,
                    LIBRARY_DEFINITION => Library => LibraryId,
                    IMPORT_DIRECTIVE => Import => ImportId => break,
                    USING_DIRECTIVE => Using => UsingId => break,
                    USER_DEFINED_TYPE_DEFINITION => Udvt => UdvtId => break,
                    FUNCTION_DEFINITION => Function => FunctionId => break,
                    CONSTRUCTOR_DEFINITION => Function => FunctionId => break,
                    FALLBACK_RECEIVE_DEFINITION => Function => FunctionId => break,
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
        let path = normalize_path(&path);
        let rope = Rope::from_str(&text);
        if let Some(file) = self.file(&path) {
            //Existing file. reset because on-disk version can vary from editor version
            self.reset(file, rope);
        } else if let Some(source_root) = self.files.source_root_for_path(self, &path) {
            //New file
            let durability = if source_root.is_dependency(self) { Durability::HIGH } else { Durability::LOW };
            let file = File::builder(rope, path.clone()).durability(durability).new(self);
            self.files.insert_file(path, file, source_root);

            let mut files = source_root.files(self).to_vec();
            files.push(file);
            source_root.set_files(self)
            .with_durability(Durability::HIGH)
            .to(files.into());
        };
    }

    pub fn apply_changes(&mut self, path: Utf8PathBuf, changes: Vec<TextDocumentContentChangeEvent>) {
        let path = normalize_path(&path);
        let Some(file) = self.file(&path) else { return; };

        for change in changes {
            if change.range.is_none() {
                //Whole file edit/change
                self.reset(file, Rope::from_str(&change.text));
                continue;
            }
            let mut rope = file.text(self);
            let range = change.range.expect("checked above");
            let start = to_rope_idx(&rope, range.start);
            let end = to_rope_idx(&rope, range.end);

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

            self.update(file, rope, &edit);
        }
    }

    fn update(&mut self, file: File, rope: Rope, edit: &InputEdit) {
        file.set_text(self).to(rope);
        self.parser.lock().update(file, edit);
    }

    fn reset(&mut self, file: File, rope: Rope) {
        file.set_text(self).to(rope);
        self.parser.lock().invalidate(file);
    }
}
