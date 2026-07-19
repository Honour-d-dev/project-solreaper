use camino::Utf8PathBuf;
use crossbeam_channel::{Receiver, Sender};
use ropey::Rope;
use std::fs::File;

use crate::workspace::{
    discover_workspace, DiscoveredSourceRoot, PackageId, SourceRootId, Workspace,
};

pub(crate) fn create_loader() -> (Sender<LoadMsg>, Receiver<LoadMsg>) {
    crossbeam_channel::bounded::<LoadMsg>(100)
}

pub(crate) enum LoadMsg {
    Workspace {
        workspace: Workspace,
    },

    SourceRootBundle {
        bundle: SourceRootBundle,
    },
    Finished,
}

#[derive(Debug, Clone)]
pub(crate) struct LoadedFile {
    pub path: Utf8PathBuf,
    pub text: Rope,
}

#[derive(Debug, Clone)]
pub(crate) struct SourceRootBundle {
    pub source_root_id: SourceRootId,
    pub package_id: PackageId,
    pub is_dependency: bool,
    pub files: Vec<LoadedFile>,
}

struct Loader {
    root: Utf8PathBuf,
    tx: Sender<LoadMsg>,
    workspace: Workspace,
    source_roots: Vec<DiscoveredSourceRoot>,
}

impl Loader {
    fn new(root: Utf8PathBuf, tx: Sender<LoadMsg>) -> Self {
        Self {
            workspace: Workspace::empty(root.clone()),
            root,
            tx,
            source_roots: Vec::new(),
        }
    }

    fn discover(&mut self) {
        let discovered = discover_workspace(&self.root);
        self.workspace = discovered.workspace;
        self.source_roots = discovered.source_roots;
    }

    fn send_workspace(&self) -> bool {
        self.tx
            .send(LoadMsg::Workspace {
                workspace: self.workspace.clone(),
            })
            .is_ok()
    }

    fn send_source_root_bundles(&self) -> bool {
        for source_root in &self.source_roots/*par_iter*/ {
            let mut files = Vec::new();
            for path in &source_root.files {
               
                let Ok(text) = (if let Ok(mut reader) = File::open(path.as_std_path()) {
                    Rope::from_reader(&mut reader)
                }  else {
                    continue;
                }) else { continue };

                files.push(LoadedFile {
                    path: path.clone(),
                    text,
                });
            }

            if self
                .tx
                .send(LoadMsg::SourceRootBundle {
                    bundle: SourceRootBundle {
                        source_root_id: source_root.id,
                        package_id: source_root.package_id,
                        is_dependency: source_root.is_dependency,
                        files,
                    },
                })
                .is_err()
            {
                return false;
            }
        }
        true
    }

    fn run(mut self) {
        self.discover();
        if !self.send_workspace() {
            return;
        }
        if !self.send_source_root_bundles() {
            return;
        }
        let _ = self.tx.send(LoadMsg::Finished);
    }
}

pub(crate) fn load(root: Utf8PathBuf, tx: Sender<LoadMsg>) {
    std::thread::spawn(move || Loader::new(root, tx).run());
}

pub fn load_workspace(root: Utf8PathBuf) -> (Workspace, Vec<SourceRootBundle>) {
    let (tx, rx) = create_loader();
    load(root, tx);

    let mut workspace = None;
    let mut bundles = Vec::new();
    for msg in rx {
        match msg {
            LoadMsg::Workspace { workspace: w } => workspace = Some(w),
            LoadMsg::SourceRootBundle { bundle } => bundles.push(bundle),
            LoadMsg::Finished => break,
        }
    }

    (workspace.unwrap(), bundles)
}


// for project in &workspace.projects {
//             log_info(
//                 &sender,
//                 format!(
//                     "Loading project {:?} at {} ({} files)",
//                     project.kind,
//                     project.root,
//                     project.files.len()
//                 ),
//             )?;

//             for file_path in &project.files {
//                 let text = match std::fs::read_to_string(file_path) {
//                     Ok(text) => text,
//                     Err(err) => {
//                         log_info(&sender, format!("Failed to read {file_path}: {err}"))?;
//                         continue;
//                     }
//                 };
//                 match editor.insert_file(file_path.clone(), text) {
//                     Ok(lowered) => {
//                         db.insert(lowered);
//                     }
//                     Err(err) => {
//                         log_info(&sender, format!("Failed to lower {file_path}: {err:#}"))?;
//                     }
//                 }
//             }
//         }