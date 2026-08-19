use rustc_hash::FxHashMap;
use tree_sitter::{InputEdit, Parser, Tree};

use super::db::File;
use super::root_db::RootDatabase;

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
        parser.set_language(&tree_sitter_solidity::LANGUAGE.into()).expect("parser set language");
        Self {
            parser,
            cache: FxHashMap::default(),
        }
    }

    pub fn update(&mut self, file: File, edit: &InputEdit) {
        self.cache.get_mut(&file).map(|tree| tree.edit(edit));
    }

    pub fn invalidate(&mut self, file: File) {
        self.cache.remove(&file);
    }

    pub fn parse(&mut self, db: &dyn RootDatabase, file: File) -> Tree {
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