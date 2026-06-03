#![allow(unused)]
use std::fs;
use std::sync::mpsc;

use camino::{Utf8Path, Utf8PathBuf};
use ignore::{WalkBuilder, WalkState};
use rustc_hash::FxHashMap;


/// @TODO add support for hybrid projects (e.g., foundry + hardhat)
/// Hardhat still needs more work
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProjectKind {
    #[default]
    Foundry,
    Hardhat,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Remapping {
    pub prefix: String,
    pub target: Utf8PathBuf,
}

#[derive(Debug,Default)]
pub(crate) struct Project {
    pub kind: ProjectKind,
    pub root: Utf8PathBuf,
    pub config: Utf8PathBuf,
    pub source_dirs: Vec<Utf8PathBuf>,
    pub dependency_roots: Vec<Utf8PathBuf>,
    pub files: Vec<Utf8PathBuf>,
    pub remappings: Vec<Remapping>,
}

#[derive(Debug)]
pub(crate) struct Workspace {
    pub root: Utf8PathBuf,
    pub projects: Vec<Project>,
    pub project_id: FxHashMap<Utf8PathBuf, usize>,
}

const PRUNED_DIRS: [&str; 5] = [".git", "node_modules", "out", "artifacts", "cache"];

pub(crate) fn discover_workspace(root: &Utf8Path) -> Workspace {
    let mut projects = Vec::new();
    let mut project_id = FxHashMap::default();

    for (kind, project_root, config) in find_project_roots(root) {
        let (source_dirs, dependency_roots, remappings) = match kind {
            ProjectKind::Foundry => foundry_layout(&project_root, &config),
            ProjectKind::Hardhat => hardhat_layout(&project_root),
        };
        let files = collect_sol_files(&source_dirs);

        projects.push(Project {
            kind,
            root: project_root.clone(),
            config,
            source_dirs,
            dependency_roots,
            files,
            remappings,
        });

        project_id.insert(project_root, projects.len() - 1);
    }

    Workspace {
        root: root.to_owned(),
        projects,
        project_id,
    }
}

fn detect_project(dir: &Utf8Path) -> Option<(ProjectKind, Utf8PathBuf)> {
    let foundry = dir.join("foundry.toml");
    if foundry.is_file() {
        return Some((ProjectKind::Foundry, foundry));
    }

    for name in ["hardhat.config.ts", "hardhat.config.js", "hardhat.config.cjs"] {
        let config = dir.join(name);
        if config.is_file() {
            return Some((ProjectKind::Hardhat, config));
        }
    }

    None
}

fn find_project_roots(root: &Utf8Path) -> Vec<(ProjectKind, Utf8PathBuf, Utf8PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];

    while let Some(dir) = stack.pop() {
        if let Some((kind, config)) = detect_project(&dir) {
            out.push((kind, dir, config));
            continue;
        }

        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            if let Ok(ft) = entry.file_type() && ft.is_dir() {
                let Ok(path) = Utf8PathBuf::from_path_buf(entry.path()) else {
                    continue;
                };
                let name = path.file_name().unwrap_or_default();
                if PRUNED_DIRS.contains(&name) {
                    continue;
                }
                stack.push(path);
            }
        }
    }

    out
}

fn foundry_layout(root: &Utf8Path, config: &Utf8Path) -> (Vec<Utf8PathBuf>, Vec<Utf8PathBuf>, Vec<Remapping>) {
    let remappings_txt = root.join("remappings.txt");
    let mut remappings = Vec::new();

    if remappings_txt.is_file() {
        // remappings.txt overrides foundry.toml entirely
        remappings = parse_remappings_txt(&remappings_txt).unwrap_or_default();
    }

    let (src, toml_remappings) = fs::read_to_string(config)
        .ok()
        .and_then(|text| parse_foundry_toml(&text, remappings.is_empty()))
        .unwrap_or_else(|| ("src".to_string(), Vec::new()));

    
    let source_dirs = [src.as_str(), "test", "script"]
    .iter()
    .map(|d| root.join(d))
        .filter(|d| d.is_dir())
        .collect();

    let dependency_roots = ["lib", "node_modules"]
        .iter()
        .map(|d| root.join(d))
        .filter(|d| d.is_dir())
        .collect();
    
    
    remappings.extend(toml_remappings);
    (source_dirs, dependency_roots, remappings)
}

pub(crate) fn generate_project(root: &Utf8Path, config: Utf8PathBuf, kind: ProjectKind) -> Project {
    match kind {
        ProjectKind::Foundry => {
            let remappings_txt = root.join("remappings.txt");
            let mut remappings = Vec::new();

            if remappings_txt.is_file() {
                // remappings.txt overrides foundry.toml entirely
                remappings = parse_remappings_txt(&remappings_txt).unwrap_or_default();
            }

            let (_, toml_remappings) = fs::read_to_string(&config)
                .ok()
                .and_then(|text| parse_foundry_toml(&text, remappings.is_empty()))
                .unwrap_or_else(|| ("src".to_string(), Vec::new()));

            remappings.extend(toml_remappings);

            Project { 
                kind, 
                root: root.to_path_buf(), 
                config, 
                source_dirs: vec![], 
                dependency_roots: vec![], 
                files: Vec::new(), 
                remappings
            }
        },
        ProjectKind::Hardhat => {
            Project { 
                kind, 
                root: root.to_path_buf(), 
                config, 
                source_dirs: vec![], 
                dependency_roots: vec![], 
                files: Vec::new(), 
                remappings: vec![]
            }
        }
    }

}

///@TODO hardhat layout still needs work
fn hardhat_layout(root: &Utf8Path) -> (Vec<Utf8PathBuf>, Vec<Utf8PathBuf>, Vec<Remapping>) {
    let source_dirs = std::iter::once(root.join("contracts"))
        .filter(|d| d.is_dir())
        .collect();

    let dependency_roots = std::iter::once(root.join("node_modules"))
        .filter(|d| d.is_dir())
        .collect();

    (source_dirs, dependency_roots, Vec::new())
}

/// Extracts (src_dir, remappings) from foundry.toml.
/// Handles [profile.default], [default] (backwards-compat), and top-level keys.
fn parse_foundry_toml(text: &str, parse_remappings: bool) -> Option<(String, Vec<Remapping>)> {
    let doc = toml::from_str::<toml::Value>(text).ok()?;

    // Try profile sections ([profile.default] or [default]) first
    if let Some((src, remaps)) = extract_from_profile(&doc, "default", parse_remappings) {
        return Some((src.unwrap_or_else(|| "src".to_string()), remaps));
    }

    // Fall back to top-level keys (no profile wrapper)
    let src = doc
        .get("src")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let remaps = if parse_remappings {
        doc.get("remappings")
            .and_then(|v| v.as_array())
            .map(parse_remap_array)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Some((src.unwrap_or_else(|| "src".to_string()), remaps))
}

/// Look inside `[profile.<name>]` or `[<<name>]` for src + remappings.
fn extract_from_profile(doc: &toml::Value, name: &str, parse_remappings: bool) -> Option<(Option<String>, Vec<Remapping>)> {
    let profile = doc
        .get("profile")
        .and_then(|v| v.as_table())
        .and_then(|table| table.get(name))
        .or_else(|| doc.as_table().and_then(|table| table.get(name)))?;

    let src = profile
        .get("src")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    let remaps = if parse_remappings {
        profile
            .get("remappings")
            .and_then(|v| v.as_array())
            .map(parse_remap_array)
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    Some((src, remaps))
}

fn parse_remap_array(arr: &Vec<toml::Value>) -> Vec<Remapping> {
    let mut out = Vec::new();
    for v in arr {
        let s = v.as_str().unwrap_or("");
        if let Some((prefix, target)) = s.split_once('=') {
            out.push(Remapping {
                prefix: prefix.trim().to_string(),
                target: Utf8PathBuf::from(target.trim()),
            });
        }
    }
    out
}

fn parse_remappings_txt(path: &Utf8Path) -> Option<Vec<Remapping>> {
    let text = fs::read_to_string(path).ok()?;
    let mut remappings = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((prefix, target)) = line.split_once('=') {
            remappings.push(Remapping {
                prefix: prefix.trim().to_string(),
                target: Utf8PathBuf::from(target.trim()),
            });
        }
    }
    Some(remappings)
}

fn collect_sol_files(dirs: &[Utf8PathBuf]) -> Vec<Utf8PathBuf> {
    let mut iter = dirs.iter();
    let Some(first) = iter.next() else {
        return Vec::new();
    };

    let mut builder = WalkBuilder::new(first);
    for dir in iter {
        builder.add(dir);
    }
    builder.standard_filters(true);

    let (tx, rx) = mpsc::channel::<Utf8PathBuf>();
    builder.build_parallel().run(|| {
        let tx = tx.clone();
        Box::new(move |result| {
            let Ok(entry) = result else {
                return WalkState::Continue;
            };
            if !entry.file_type().is_some_and(|ft| ft.is_file()) {
                return WalkState::Continue;
            }
            let path = entry.into_path();
            if path.extension().and_then(|e| e.to_str()) == Some("sol") {
                if let Ok(path) = Utf8PathBuf::from_path_buf(path) {
                    let _ = tx.send(path);
                }
            }
            WalkState::Continue
        })
    });
    drop(tx);

    rx.into_iter().collect()
}
