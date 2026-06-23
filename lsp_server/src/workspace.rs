#![allow(unused)]
use std::fs;
use std::sync::mpsc;

use camino::{Utf8Path, Utf8PathBuf};
use ignore::{WalkBuilder, WalkState};
use rustc_hash::FxHashMap;


/// @TODO add support for hybrid Packages (e.g., foundry + hardhat)
/// Hardhat still needs more work
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PackageKind {
    #[default]
    Foundry,
    Hardhat,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct PackageId(pub usize);

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct SourceRootId(pub u32);

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Remapping {
    pub prefix: String,
    pub target: Utf8PathBuf,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct PackageConfig {
    pub remappings: Vec<Remapping>,
}

#[derive(Debug, Default, Clone)]
pub(crate) struct Package {
    pub kind: PackageKind,
    pub root: Utf8PathBuf,
    pub source_roots: Vec<SourceRootId>,
    pub config: PackageConfig,
    pub is_dependency: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredSourceRoot {
    pub id: SourceRootId,
    pub package_id: PackageId,
    pub files: Vec<Utf8PathBuf>,
    pub is_dependency: bool,
}

#[derive(Debug, Clone)]
pub(crate) struct DiscoveredWorkspace {
    pub workspace: Workspace,
    pub source_roots: Vec<DiscoveredSourceRoot>,
}

#[derive(Debug, Clone)]
pub(crate) struct Workspace {
    pub root: Utf8PathBuf,
    pub packages: Vec<Package>,
    pub package_id: FxHashMap<Utf8PathBuf, PackageId>,
}

impl Workspace {
    pub(crate) fn empty(root: Utf8PathBuf) -> Self {
        Self {
            root,
            packages: Vec::new(),
            package_id: FxHashMap::default(),
        }
    }
}

const PRUNED_DIRS: [&str; 5] = [".git", "node_modules", "out", "artifacts", "cache"];

pub(crate) fn discover_workspace(root: &Utf8Path) -> DiscoveredWorkspace {
    let mut workspace = Workspace::empty(root.to_owned());
    let mut source_roots = Vec::<DiscoveredSourceRoot>::new();
    let mut pending_foundry = Vec::<(Utf8PathBuf, Utf8PathBuf, bool)>::new();

    for (kind, package_root, config) in find_package_roots(root) {
        let is_dependency = is_dependency_package_path(root, &package_root);
        match kind {
            PackageKind::Foundry => pending_foundry.push((package_root, config, is_dependency)),
            PackageKind::Hardhat => {
                if is_dependency {
                    // Eager dependency loading is currently Foundry-only.
                    continue;
                }
                if workspace.package_id.contains_key(&package_root) {
                    continue;
                }
                let (source_root_dirs, package_config) = hardhat_layout(&package_root, &config);
                add_package(
                    &mut workspace,
                    &mut source_roots,
                    kind,
                    package_root,
                    source_root_dirs,
                    package_config,
                    false,
                );
            }
        }
    }

    while let Some((package_root, config, is_dependency)) = pending_foundry.pop() {
        if workspace.package_id.contains_key(&package_root) {
            continue;
        }

        let (source_root_dirs, dependency_roots, package_config) =
            foundry_layout(&package_root, &config, !is_dependency);

        add_package(
            &mut workspace,
            &mut source_roots,
            PackageKind::Foundry,
            package_root,
            source_root_dirs,
            package_config,
            is_dependency,
        );

        // Eager dependency package discovery for Foundry only.
        for dep_root in dependency_roots {
            for (kind, dep_package_root, dep_config) in find_package_roots(&dep_root) {
                if kind != PackageKind::Foundry {
                    continue;
                }

                if workspace.package_id.contains_key(&dep_package_root) {
                    continue;
                }

                pending_foundry.push((dep_package_root, dep_config, true));
            }
        }
    }

    DiscoveredWorkspace {
        workspace,
        source_roots,
    }
}

fn is_dependency_package_path(workspace_root: &Utf8Path, package_root: &Utf8Path) -> bool {
    package_root
        .strip_prefix(workspace_root)
        .map(|rel| {
            rel.as_str()
                .split('/')//FIXME: windows split is \ 
                .any(|segment| segment == "lib" || segment == "node_modules")
        })
        .unwrap_or(false)
}

fn add_package(
    workspace: &mut Workspace,
    discovered_source_roots: &mut Vec<DiscoveredSourceRoot>,
    kind: PackageKind,
    package_root: Utf8PathBuf,
    source_root_dirs: Vec<Utf8PathBuf>,
    config: PackageConfig,
    is_dependency: bool,
) {
    let package_id = PackageId(workspace.packages.len());
    workspace.package_id.insert(package_root.clone(), package_id);

    let mut source_roots = Vec::new();

    for root in source_root_dirs {//par_iter
        let root_files = collect_sol_files(std::slice::from_ref(&root));
        let source_root_id = SourceRootId(discovered_source_roots.len() as u32);

        source_roots.push(source_root_id);
        discovered_source_roots.push(DiscoveredSourceRoot {
            id: source_root_id,
            package_id,
            files: root_files,
            is_dependency,
        });
    }

    workspace.packages.push(Package {
        kind,
        root: package_root,
        source_roots,
        config,
        is_dependency,
    });
}

fn detect_package(dir: &Utf8Path) -> Option<(PackageKind, Utf8PathBuf)> {
    let foundry = dir.join("foundry.toml");
    if foundry.is_file() {
        return Some((PackageKind::Foundry, foundry));
    }

    for name in ["hardhat.config.ts", "hardhat.config.js", "hardhat.config.cjs"] {
        let config = dir.join(name);
        if config.is_file() {
            return Some((PackageKind::Hardhat, config));
        }
    }

    None
}

fn find_package_roots(root: &Utf8Path) -> Vec<(PackageKind, Utf8PathBuf, Utf8PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_owned()];

    while let Some(dir) = stack.pop() {//FIXME: add depth limit. This keeps going down a path until it finds a package
        if let Some((kind, config)) = detect_package(&dir) {
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

fn foundry_layout(
    root: &Utf8Path,
    config: &Utf8Path,
    include_dev_source_roots: bool,
) -> (
    Vec<Utf8PathBuf>,
    Vec<Utf8PathBuf>,
    PackageConfig,
) {
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

    let mut source_root_dirs = Vec::new();
    let src_dir = root.join(src.as_str());
    if src_dir.is_dir() {//FIXME: src directory could be contracts 
        source_root_dirs.push(src_dir);
    }

    let contracts_dir = root.join("contracts");
    if contracts_dir.is_dir() {
        source_root_dirs.push(contracts_dir);
    }

    if include_dev_source_roots {
        let test_dir = root.join("test");
        if test_dir.is_dir() {
            source_root_dirs.push(test_dir);
        }

        let script_dir = root.join("script");
        if script_dir.is_dir() {
            source_root_dirs.push(script_dir);
        }
    }

    let dependency_roots = ["lib"/* , "node_modules"*/]//we don't want to search node_modules, but i think there're other aliases to lib in foudry
        .iter()
        .map(|d| root.join(d))
        .filter(|d| d.is_dir())
        .collect();
    
    
    if remappings.is_empty() {
        remappings = toml_remappings;
    }
    (
        source_root_dirs,
        dependency_roots,
        PackageConfig { remappings },
    )
}

///used by editor, remove after editor refactor
pub(crate) fn generate_package(root: &Utf8Path, config: Utf8PathBuf, kind: PackageKind) -> Package {
    match kind {
        PackageKind::Foundry => {
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

            Package { 
                kind, 
                root: root.to_path_buf(), 
                source_roots: vec![],
                config: PackageConfig { remappings },
                is_dependency: false,
            }
        },
        PackageKind::Hardhat => {
            Package { 
                kind, 
                root: root.to_path_buf(), 
                source_roots: vec![],
                config: PackageConfig::default(),
                is_dependency: false,
            }
        }
    }

}

///@TODO hardhat layout still needs work
fn hardhat_layout(
    root: &Utf8Path,
    _config: &Utf8Path,
) -> (
    Vec<Utf8PathBuf>,
    PackageConfig,
) {
    let source_root_dirs = std::iter::once(root.join("contracts"))
        .filter(|d| d.is_dir())
        .collect::<Vec<_>>();

    (source_root_dirs, PackageConfig::default())
}

/// Extracts (src_dir, remappings) from foundry.toml.
/// Handles [profile.default], [default] (backwards-compat), and top-level keys.
fn parse_foundry_toml(text: &str, parse_remappings: bool) -> Option<(String, Vec<Remapping>)> {//@TODO: write tests for these
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
