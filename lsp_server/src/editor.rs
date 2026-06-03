#![allow(unused)]
use std::{collections::hash_map::Entry, fs};

use anyhow::Context;
use lsp_types::{Position, TextDocumentContentChangeEvent};
use ropey::Rope;
use rustc_hash::FxHashMap;
use camino::{Utf8Path, Utf8PathBuf};
use tree_sitter::{InputEdit, Parser, Tree};

use crate::{lowering::{File, lower, summarized_lower}, utilities::{byte_to_point, to_rope_byte_idx, to_rope_idx}, workspace::{Project, ProjectKind, Remapping, Workspace, generate_project}};

struct FileData {
    rope: Rope,
    tree: Tree,
    has_changes: bool,
    project_root: Utf8PathBuf,
}

pub(crate) struct EditorHost {
    parser: Parser,
    files: FxHashMap<Utf8PathBuf, FileData>,
    workspace: Workspace,
}

impl EditorHost {
    pub(crate) fn new(workspace: Workspace) -> Self {
        let mut parser = Parser::new();
        parser.set_language(&tree_sitter_solidity::LANGUAGE.into()).expect("failed to set parser language");
        
        Self {
            parser,
            files: FxHashMap::default(),
            workspace
        }
    }

    pub fn has_file(&self, path: &Utf8PathBuf) -> bool {
        self.files.contains_key(path)
    }

    fn get_rope(&self, path: &Utf8PathBuf) -> &Rope {
        &self.files.get(path).unwrap().rope
    }

    fn get_tree(&self, path: &Utf8PathBuf) -> &Tree {
        &self.files.get(path).unwrap().tree
    }

    fn find_project_for_file(&self, path: &Utf8Path) -> Option<(&Utf8Path, &[Remapping])> {
        self.workspace
            .projects
            .iter()
            .filter(|project| path.starts_with(&project.root))
            // Longest match means the deepest matching root path.
            .max_by_key(|project| project.root.components().count())
            .map(|project| (project.root.as_path(), project.remappings.as_slice()))
    }

    fn get_root(&self, path: &Utf8Path) -> &Utf8Path {
        self.find_project_for_file(path).map(|(root, _)| root).unwrap_or(&Utf8Path::new(""))//A root must exist for any file
    }

    fn get_root_remappings(&self, root: &Utf8Path) -> &[Remapping] {
        match self.workspace.project_id.get(root) {
            Some(&id) => &self.workspace.projects[id].remappings,
            None => &[]
        }
    }

    fn get_remappings_for_file(&self, path: &Utf8PathBuf) -> &[Remapping] {
        if self.has_file(path) {
            let file_data = self.files.get(path).unwrap();
            return self.get_root_remappings(&file_data.project_root);
        }
        self.find_project_for_file(path).map(|(_, remappings)| remappings).unwrap_or(&[])
    }

    pub(crate) fn insert_file(&mut self, path: Utf8PathBuf, text: String) -> anyhow::Result<File> {
        let rope = Rope::from_str(&text);
        let tree = self.parser.parse(&text, None).expect("failed to parse file");
        let (project_root, remappings) = self.find_project_for_file(&path).context("no matching root")?;
        let file = lower(&path, &text, &tree, project_root, remappings);
        self.files.insert(path, FileData {
            rope,
            tree,
            has_changes: false,
            project_root: project_root.to_owned(),
        });
        Ok(file)
    }

    ///NOTE: Returns a File if changes are lowered else NONE
    pub fn update(&mut self, path: &Utf8PathBuf, change: TextDocumentContentChangeEvent) {
        
        match self.files.entry(path.clone()) {
            Entry::Occupied(mut entry) => {
                let file_data = entry.get_mut();
                let rope = &mut file_data.rope;
                let start = to_rope_idx(rope, change.range.unwrap().start);
                let end = to_rope_idx(rope, change.range.unwrap().end);

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

                file_data.tree.edit(&edit);
                let new_tree = self.parser.parse_with_options( &mut |b, _| {
                    let (chunk, b1, _, _) = rope.chunk_at_byte(b);
                    &chunk[(b - b1)..]
                }, Some(&file_data.tree), None);
                file_data.tree = new_tree.unwrap();
                file_data.has_changes = true;
            }
            Entry::Vacant(_) => {}
        }
    }

    pub fn apply_changes(&mut self, path: &Utf8PathBuf) -> Option<File> {
        if let Some(file_data) = self.files.get(path) && file_data.has_changes {
            let project_root = &file_data.project_root;
            let remappings = self.get_root_remappings(project_root);
            let file = lower(
                path,
                &file_data.rope.to_string(),
                &file_data.tree,
                project_root,
                remappings,
            );
            self.files.get_mut(path).unwrap().has_changes = false;
            Some(file)
        } else {
            None
        }
    }

    fn get_dep_root(&mut self, dep: &Utf8PathBuf) -> (&Utf8Path, &[Remapping]) {
        //@TODO Theres a more convinient way to do this using project.dependency_roots
        //if there's no lib/node_modules directory between root and dep_path then we can assume we're in the dep_root
        let pj_root = self.get_root(dep).to_path_buf();
        let trailing_path = dep.strip_prefix(&pj_root).unwrap();
        let is_dep_root = trailing_path.as_str().split("/").any(|dir| dir == "lib" || dir == "node_modules");
        if is_dep_root {
            if let Some(&id) = self.workspace.project_id.get(&pj_root) {
                let project = &self.workspace.projects[id];
                return (&project.root, &project.remappings);
            }
            return (&Utf8Path::new(""), &[]);
        } else {
            
            let mut project = Project::default();
            let mut cur = dep.parent().unwrap();
            while cur.starts_with(&pj_root) {
                if cur.join("foundry.toml").is_file() {
                    project = generate_project(cur, cur.join("foundry.toml"), ProjectKind::Foundry);
                    break;
                }
                if let Some(config) = ["hardhat.config.ts", "hardhat.config.js", "hardhat.config.cjs"]
                .iter().find(|f| cur.join(f).is_file())
                {
                    project = generate_project(cur, cur.join(config), ProjectKind::Hardhat);
                    break;
                }
                cur = cur.parent().unwrap();
            }
            let id = self.workspace.projects.len();
            self.workspace.project_id.insert(project.root.clone(), id);
            self.workspace.projects.push(project);
            let project = &self.workspace.projects[id];
            (&project.root, &project.remappings)
        }
    }

    pub fn resolve_deps(&mut self, deps: &[Utf8PathBuf]) -> Vec<File> {
        //get dep project root
        //extract remappings
        //lower
        let mut files = Vec::new();
        for dep in deps {
            let source = fs::read_to_string(dep).unwrap_or_default();
            let tree = self.parser.parse(&source, None).unwrap();
            let dep_root = self.get_dep_root(dep);
            let file = summarized_lower(dep, &source, &tree, dep_root.0, dep_root.1);
            files.push(file);
        }
        files
    }


    ///Returns the Node at the given position if it exists
    /// With tree-sitter we get name resolution for free 😊
    pub fn get_node_at_position(&self, path: &Utf8PathBuf, pos: Position) -> anyhow::Result<tree_sitter::Node<'_>> {
        let byte_idx = to_rope_byte_idx(self.get_rope(path), pos);
        self.get_tree(path).root_node().descendant_for_byte_range(byte_idx, byte_idx).context("couldn't find node at byte index")
    }

    pub fn get_node_identifier(&self, path: &Utf8PathBuf, node: &tree_sitter::Node<'_>) -> anyhow::Result<String> {
        if node.kind() == "identifier" {
            Ok(self.get_rope(path).slice(node.byte_range()).to_string())
        } else {
            anyhow::bail!("Node at position is not an identifier, node is {}", node.kind())
        }
    }

    pub fn get_node_and_identifier(&self, path: &Utf8PathBuf, pos: Position) -> anyhow::Result<(String, tree_sitter::Node<'_>)> {
        let node = self.get_node_at_position(path, pos)?;
        let identifier = self.get_node_identifier(path, &node)?;
        Ok((identifier, node))
    }

    pub fn get_identifier(&self, path: &Utf8PathBuf, pos: Position) -> anyhow::Result<String> {
        let node = self.get_node_at_position(path, pos)?;
        self.get_node_identifier(path, &node)
    }
    
    
}
