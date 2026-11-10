//! Browsing the files inside a collection, for the owner's own screen.
//!
//! Names and paths come from the private inventory and are shown only on
//! this computer; they never go into the data map, a saved report or a pack
//! summary.

use crate::model::Status;
use crate::scan::Inventory;
use serde::Serialize;
use std::collections::BTreeMap;

/// One folder or file in a collection.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// Logical path, starting with the selection's own name. Used to exclude.
    pub path: String,
    pub folder: bool,
    /// Files at or under this entry (archives themselves are not counted).
    pub files: u64,
    pub bytes: u64,
    /// For a file, its detected format.
    pub format: Option<&'static str>,
    pub status: Option<Status>,
    /// Files at or under this entry that are excluded.
    pub excluded_files: u64,
    /// This exact path is on the exclusion list (so it can be restored).
    pub excluded_here: bool,
    /// Links to known services in the files at or under this entry.
    pub links: u64,
}

/// The name every path in the collection starts with.
pub fn root_of(inv: &Inventory, collection: u32) -> Option<String> {
    inv.items
        .iter()
        .find(|it| it.collection == collection)
        .map(|it| it.local_path.split('/').next().unwrap_or("").to_string())
}

struct Acc {
    folder: bool,
    files: u64,
    bytes: u64,
    excluded: u64,
    links: u64,
    leaf: Option<(&'static str, Status)>,
}

/// Folders and files directly under `prefix` (a logical path). With no
/// prefix, the top of the collection: for a ZIP or a folder, what is inside
/// it; for a single file, the file itself.
pub fn children(
    inv: &Inventory,
    collection: u32,
    prefix: Option<&str>,
    service: Option<&str>,
) -> Vec<Entry> {
    let Some(root) = root_of(inv, collection) else {
        return Vec::new();
    };
    let prefix = prefix.map(|p| p.trim_matches('/').to_string());
    let has_children = |p: &str| {
        let pre = format!("{p}/");
        inv.items
            .iter()
            .any(|it| it.collection == collection && it.local_path.starts_with(&pre))
    };
    // A single selected file has nothing inside: show the file itself.
    let base = match prefix {
        Some(p) => p,
        None if has_children(&root) => root.clone(),
        None => {
            return entries_at(inv, collection, "", service)
                .into_iter()
                .filter(|e| e.path == root)
                .collect();
        }
    };
    entries_at(inv, collection, &base, service)
}

fn entries_at(inv: &Inventory, collection: u32, base: &str, service: Option<&str>) -> Vec<Entry> {
    let pre = if base.is_empty() {
        String::new()
    } else {
        format!("{base}/")
    };
    let mut acc: BTreeMap<String, Acc> = BTreeMap::new();
    // Paths that have something under them (folders and archives).
    let parents: std::collections::HashSet<&str> = inv
        .items
        .iter()
        .filter(|it| it.collection == collection)
        .filter_map(|it| it.local_path.rsplit_once('/').map(|(p, _)| p))
        .collect();
    for it in inv.items.iter().filter(|it| it.collection == collection) {
        let Some(rest) = it.local_path.strip_prefix(&pre) else {
            continue;
        };
        if rest.is_empty() {
            continue;
        }
        let (name, deeper) = match rest.split_once('/') {
            Some((n, _)) => (n, true),
            None => (rest, false),
        };
        let is_parent = parents.contains(it.local_path.as_str());
        let a = acc.entry(name.to_string()).or_insert(Acc {
            folder: false,
            files: 0,
            bytes: 0,
            excluded: 0,
            links: 0,
            leaf: None,
        });
        if deeper || is_parent {
            a.folder = true;
        }
        if !deeper && !is_parent {
            a.leaf = Some((it.format.label(), it.status));
        }
        if is_parent {
            continue; // an archive: its members are counted instead
        }
        a.files += 1;
        a.bytes += it.expanded_bytes.max(it.stored_bytes);
        if it.status == Status::Excluded {
            a.excluded += 1;
        }
        if let Some(l) = inv.links_by_item.get(&it.id) {
            a.links += match service {
                Some(s) => l.get(s).copied().unwrap_or(0) as u64,
                None => l.values().map(|&v| v as u64).sum(),
            };
        }
    }
    let mut out: Vec<Entry> = acc
        .into_iter()
        .map(|(name, a)| {
            let path = format!("{pre}{name}");
            Entry {
                excluded_here: inv.exclude.iter().any(|e| e.trim_matches('/') == path),
                name,
                path,
                folder: a.folder,
                files: a.files,
                bytes: a.bytes,
                format: if a.folder { None } else { a.leaf.map(|l| l.0) },
                status: if a.folder { None } else { a.leaf.map(|l| l.1) },
                excluded_files: a.excluded,
                links: a.links,
            }
        })
        .collect();
    // Folders first, then files; each by name.
    out.sort_by(|a, b| b.folder.cmp(&a.folder).then(a.name.cmp(&b.name)));
    out
}

/// Files in a collection with links to `service`, most links first.
pub fn linking_files(inv: &Inventory, collection: u32, service: &str, limit: usize) -> Vec<Entry> {
    let mut out: Vec<Entry> = inv
        .items
        .iter()
        .filter(|it| it.collection == collection)
        .filter_map(|it| {
            let n = inv.links_by_item.get(&it.id)?.get(service).copied()?;
            Some(Entry {
                name: it.local_path.rsplit('/').next().unwrap_or("").to_string(),
                path: it.local_path.clone(),
                folder: false,
                files: 1,
                bytes: it.expanded_bytes.max(it.stored_bytes),
                format: Some(it.format.label()),
                status: Some(it.status),
                excluded_files: u64::from(it.status == Status::Excluded),
                excluded_here: inv
                    .exclude
                    .iter()
                    .any(|e| e.trim_matches('/') == it.local_path),
                links: n as u64,
            })
        })
        .collect();
    out.sort_by(|a, b| b.links.cmp(&a.links).then(a.path.cmp(&b.path)));
    out.truncate(limit);
    out
}

/// How many files in a collection link to each service. Local only.
pub fn linking_file_counts(inv: &Inventory, collection: u32) -> BTreeMap<&'static str, u64> {
    let mut out = BTreeMap::new();
    for it in inv.items.iter().filter(|it| it.collection == collection) {
        if let Some(l) = inv.links_by_item.get(&it.id) {
            for &s in l.keys() {
                *out.entry(s).or_default() += 1;
            }
        }
    }
    out
}
