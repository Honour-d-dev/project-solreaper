use std::collections::HashMap;

use camino::{Utf8Path, Utf8PathBuf};
use crossbeam_channel::Sender;
use tree_sitter::Parser;

use crate::lowering::{lower, File};
use crate::workspace::{Remapping, Workspace};

pub(crate) fn create_loader() -> (Sender<LoadMsg>, crossbeam_channel::Receiver<LoadMsg>) {
    crossbeam_channel::bounded::<LoadMsg>(100)
}

pub(crate) enum LoadMsg {
    /// A fully parsed + lowered file, ready to drop into editor/db.
    File {
        lowered: File,
    },
    Finished,
}

pub(crate) fn load(workspace: &Workspace, tx: Sender<LoadMsg>) {
    // Build a map from file path -> (project_root, remappings) so the thread
    // can resolve imports correctly without borrowing the workspace.
    let mut file_remaps: HashMap<Utf8PathBuf, (Utf8PathBuf, Vec<Remapping>)> = HashMap::new();
    for project in &workspace.projects {
        for file in &project.files {
            file_remaps.insert(
                file.clone(),
                (project.root.clone(), project.remappings.clone()),
            );
        }
    }

    let files: Vec<Utf8PathBuf> = workspace
        .projects
        .iter()
        .flat_map(|p| p.files.iter().cloned())
        .collect();

    std::thread::spawn(move || {
        let mut parser = Parser::new();
        let _ = parser
            .set_language(&tree_sitter_solidity::LANGUAGE.into());

        for path in files {
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let Some(tree) = parser.parse(&text, None) else {
                continue;
            };

            let (project_root, remappings) = file_remaps
                .get(&path)
                .map(|(r, m)| (r.as_path(), m.as_slice()))
                .unwrap_or((Utf8Path::new(""), &[]));

            let lowered = lower(&path, &text, &tree, project_root, remappings);

            if tx.send(LoadMsg::File { lowered }).is_err() {
                return;
            }
        }

        let _ = tx.send(LoadMsg::Finished);
    });
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