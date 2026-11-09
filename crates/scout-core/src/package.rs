//! "Create data pack": copies the owner's selections into one folder with
//! clear, standard names, ready to put on a drive.
//!
//! - Slack exports go to `Slack/`, downloaded attachments to `Slack/Attachments/`.
//! - Google Takeout archives are unpacked and merged by product, so several
//!   `takeout-…-001.zip` parts become `Google Drive/`, `Google Mail/` and so on.
//! - Mailboxes go to `Email/`, Git repositories (with history) to `Code/<repo>/`.
//! - Everything else goes to `Documents/` or `Other/`.
//!
//! Originals are never modified. Items the owner excluded are not copied.
//! Symbolic links are not followed, and archive members whose names would
//! escape their folder are skipped. Nothing here touches the network.

use crate::analyze::sources::takeout_product;
use crate::model::{Format, SourceKind, Status};
use crate::scan::Inventory;
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

pub const README_NAME: &str = "README.txt";
pub const MANIFEST_NAME: &str = "Working Minds data pack.json";

#[derive(Debug, Clone)]
enum Action {
    /// Copy a file into `folder`.
    CopyFile(PathBuf),
    /// Copy a folder's contents into `folder/<name>`, or straight into
    /// `folder` when `name` is None (Slack attachments, repositories).
    CopyDir { src: PathBuf, name: Option<String> },
    /// Unpack a Takeout ZIP, routing `Takeout/<Product>/…` to `Google <Product>/…`.
    TakeoutZip(PathBuf),
    /// Copy an unpacked Takeout folder the same way.
    TakeoutDir(PathBuf),
}

/// One selection and where it will go in the pack.
#[derive(Debug, Clone, Serialize)]
pub struct PlanEntry {
    /// The selection's readable name.
    pub collection: String,
    /// Top-level folders in the pack this selection fills.
    pub folders: Vec<String>,
    pub kind: &'static str,
    pub files: u64,
    pub bytes: u64,
    #[serde(skip)]
    label: String,
    #[serde(skip)]
    action: Option<Action>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Plan {
    pub entries: Vec<PlanEntry>,
    pub total_files: u64,
    pub total_bytes: u64,
    /// Files left out because the owner excluded them.
    pub excluded_files: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Progress {
    pub files_copied: u64,
    pub bytes_copied: u64,
    pub total_files: u64,
    pub total_bytes: u64,
    pub current: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Summary {
    pub folder: String,
    pub files_copied: u64,
    pub bytes_copied: u64,
    pub skipped_unsafe: u64,
    pub stopped: bool,
    pub error: Option<String>,
}

fn product_folder(product: &str) -> String {
    match product {
        "Mail" => "Google Mail".into(),
        p if p.starts_with("Google ") => p.to_string(),
        p => format!("Google {p}"),
    }
}

fn safe_name(s: &str) -> String {
    let mut n: String = s
        .chars()
        .map(|c| {
            if c.is_control() || "<>:\"/\\|?*".contains(c) {
                '_'
            } else {
                c
            }
        })
        .collect();
    n = n.trim().trim_end_matches(['.', ' ']).to_string();
    if n.is_empty() {
        "Untitled".into()
    } else {
        n
    }
}

/// Plans where each selection goes. Reads only the inventory.
pub fn plan(inv: &Inventory) -> Plan {
    let mut excluded = 0;
    let mut per: BTreeMap<u32, (u64, u64, Vec<Format>)> = BTreeMap::new();
    for it in &inv.items {
        if it.status == Status::Excluded {
            excluded += 1;
            continue;
        }
        if it.format.is_container() && it.status.content_examined() {
            continue;
        }
        let e = per.entry(it.collection).or_default();
        e.0 += 1;
        e.1 += it.expanded_bytes.max(if it.parent.is_none() {
            it.stored_bytes
        } else {
            0
        });
        e.2.push(it.format);
    }
    // Collections that hold attachments matched to a Slack export.
    let attachment_cols: HashSet<u32> = inv
        .attachments
        .values()
        .flatten()
        .map(|&id| inv.items[id as usize].collection)
        .collect();

    let snapshots = crate::report::code_snapshots(inv);
    let mut entries = Vec::new();
    for c in &inv.collections {
        let t = &inv.text[c.id as usize];
        let (files, bytes, formats) = per.get(&c.id).cloned().unwrap_or_default();
        let is_dir = c.source.is_dir();
        let snapshot = snapshots.get(&c.id);
        let display = match snapshot {
            Some(snap) => snap.display_label(),
            None => crate::report::display_label_for(c, t),
        };
        let (kind, folders, action): (&'static str, Vec<String>, Action) =
            if c.source_kind == SourceKind::SlackExport {
                (
                    "Slack export",
                    vec!["Slack".into()],
                    if is_dir {
                        Action::CopyDir {
                            src: c.source.clone(),
                            name: Some(c.label.clone()),
                        }
                    } else {
                        Action::CopyFile(c.source.clone())
                    },
                )
            } else if attachment_cols.contains(&c.id) && is_dir {
                (
                    "Slack attachments",
                    vec!["Slack/Attachments".into()],
                    Action::CopyDir {
                        src: c.source.clone(),
                        name: None,
                    },
                )
            } else if !t.takeout_products.is_empty() {
                let folders = t
                    .takeout_products
                    .iter()
                    .map(|p| product_folder(p))
                    .collect();
                let ext = c.label.to_ascii_lowercase();
                let action = if is_dir {
                    Action::TakeoutDir(c.source.clone())
                } else if ext.ends_with(".zip") {
                    Action::TakeoutZip(c.source.clone())
                } else {
                    // .tgz Takeouts are copied as they are.
                    Action::CopyFile(c.source.clone())
                };
                ("Google Takeout", folders, action)
            } else if let Some(snap) = snapshot {
                (
                    "Code download",
                    vec![format!("Code/{}", snap.repo().replace('/', "-"))],
                    if is_dir {
                        Action::CopyDir {
                            src: c.source.clone(),
                            name: None,
                        }
                    } else {
                        Action::CopyFile(c.source.clone())
                    },
                )
            } else if !t.git_remotes.is_empty() && is_dir {
                let r = t.git_remotes.iter().next().unwrap();
                let repo = r
                    .split_once('/')
                    .map_or(r.as_str(), |x| x.1)
                    .replace('/', "-");
                (
                    "Git repository",
                    vec![format!("Code/{repo}")],
                    Action::CopyDir {
                        src: c.source.clone(),
                        name: None,
                    },
                )
            } else if !formats.is_empty()
                && formats
                    .iter()
                    .all(|f| matches!(f, Format::Mbox | Format::Eml))
            {
                (
                    "Email",
                    vec!["Email".into()],
                    if is_dir {
                        Action::CopyDir {
                            src: c.source.clone(),
                            name: Some(c.label.clone()),
                        }
                    } else {
                        Action::CopyFile(c.source.clone())
                    },
                )
            } else if is_dir {
                (
                    "Folder",
                    vec!["Documents".into()],
                    Action::CopyDir {
                        src: c.source.clone(),
                        name: Some(c.label.clone()),
                    },
                )
            } else {
                let doc = formats
                    .iter()
                    .any(|f| f.category() == "Documents" || f.category() == "Tables");
                let folder = if doc { "Documents" } else { "Other" };
                (
                    "File",
                    vec![folder.into()],
                    Action::CopyFile(c.source.clone()),
                )
            };
        entries.push(PlanEntry {
            collection: display,
            folders,
            kind,
            files,
            bytes,
            label: c.label.clone(),
            action: Some(action),
        });
    }
    Plan {
        total_files: entries.iter().map(|e| e.files).sum(),
        total_bytes: entries.iter().map(|e| e.bytes).sum(),
        excluded_files: excluded,
        entries,
    }
}

struct Copier<'a> {
    excluded: HashSet<String>,
    cancel: Option<&'a AtomicBool>,
    progress: &'a mut dyn FnMut(&Progress),
    p: Progress,
    skipped_unsafe: u64,
}

impl Copier<'_> {
    fn stopped(&self) -> bool {
        self.cancel.is_some_and(|c| c.load(Ordering::Relaxed))
    }

    fn report(&mut self, name: &str) {
        self.p.current = name.to_string();
        (self.progress)(&self.p);
    }

    fn copy_stream(&mut self, r: &mut dyn Read, dest: &Path) -> io::Result<()> {
        if let Some(dir) = dest.parent() {
            fs::create_dir_all(dir)?;
        }
        let dest = unique(dest);
        let mut out = File::create(&dest)?;
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            if self.stopped() {
                drop(out);
                let _ = fs::remove_file(&dest);
                return Err(io::Error::other("stopped"));
            }
            let n = r.read(&mut buf)?;
            if n == 0 {
                break;
            }
            out.write_all(&buf[..n])?;
            self.p.bytes_copied += n as u64;
        }
        self.p.files_copied += 1;
        Ok(())
    }

    fn copy_file(&mut self, src: &Path, dest: &Path) -> io::Result<()> {
        self.report(
            &dest
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        let mut f = File::open(src)?;
        self.copy_stream(&mut f, dest)
    }

    /// Copies `src` into `dest`, skipping excluded logical paths
    /// (`logical` is the selection-relative path of `src`).
    fn copy_dir(&mut self, src: &Path, dest: &Path, logical: &str) -> io::Result<()> {
        let mut entries: Vec<_> = fs::read_dir(src)?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if self.stopped() {
                return Err(io::Error::other("stopped"));
            }
            let name = e.file_name().to_string_lossy().into_owned();
            let lp = format!("{logical}/{name}");
            let md = fs::symlink_metadata(e.path())?;
            if md.file_type().is_symlink() {
                continue;
            }
            if md.is_dir() {
                self.copy_dir(&e.path(), &dest.join(&name), &lp)?;
            } else if md.is_file() && !self.excluded.contains(&lp) {
                self.copy_file(&e.path(), &dest.join(&name))?;
            }
        }
        Ok(())
    }

    /// Routes `<…>/Takeout/<Product>/<rest>` to `<root>/Google <Product>/<rest>`.
    fn takeout_target(&self, root: &Path, rel: &str) -> Option<PathBuf> {
        let product = takeout_product(rel)?;
        let marker = format!("Takeout/{product}/");
        let rest = &rel[rel.find(&marker)? + marker.len()..];
        let mut out = root.join(product_folder(product));
        for part in Path::new(rest).components() {
            match part {
                Component::Normal(p) => out.push(safe_name(&p.to_string_lossy())),
                _ => return None, // "..", absolute or drive prefixes: never leave the pack
            }
        }
        Some(out)
    }

    fn takeout_zip(&mut self, src: &Path, root: &Path, label: &str) -> io::Result<()> {
        let mut zip = zip::ZipArchive::new(File::open(src)?).map_err(io::Error::other)?;
        for i in 0..zip.len() {
            if self.stopped() {
                return Err(io::Error::other("stopped"));
            }
            let mut f = zip.by_index(i).map_err(io::Error::other)?;
            if f.is_dir() {
                continue;
            }
            let name = f.name().replace('\\', "/");
            if self.excluded.contains(&format!("{label}/{name}")) {
                continue;
            }
            // Count names that try to climb out of their folder, even when the
            // ZIP library would tidy them into something harmless-looking.
            let climbs = name.starts_with('/') || name.split('/').any(|seg| seg == "..");
            if f.enclosed_name().is_none() || climbs {
                self.skipped_unsafe += 1;
                continue;
            }
            let Some(dest) = self.takeout_target(root, &name) else {
                continue;
            };
            self.report(&name);
            self.copy_stream(&mut f, &dest)?;
        }
        Ok(())
    }

    fn takeout_dir(&mut self, dir: &Path, base: &Path, root: &Path, label: &str) -> io::Result<()> {
        let mut entries: Vec<_> = fs::read_dir(dir)?.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for e in entries {
            if self.stopped() {
                return Err(io::Error::other("stopped"));
            }
            let md = fs::symlink_metadata(e.path())?;
            if md.file_type().is_symlink() {
                continue;
            }
            if md.is_dir() {
                self.takeout_dir(&e.path(), base, root, label)?;
            } else if md.is_file() {
                let rel = e
                    .path()
                    .strip_prefix(base)
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
                    .unwrap_or_default();
                if self.excluded.contains(&format!("{label}/{rel}")) {
                    continue;
                }
                if let Some(dest) = self.takeout_target(root, &rel) {
                    self.copy_file(&e.path(), &dest)?;
                }
            }
        }
        Ok(())
    }
}

/// Adds " (2)", " (3)", … before the extension when `p` already exists.
fn unique(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    for n in 2.. {
        let c = p.with_file_name(format!("{stem} ({n}){ext}"));
        if !c.exists() {
            return c;
        }
    }
    unreachable!()
}

/// Creates the pack inside `parent` and copies every planned selection.
pub fn build(
    inv: &Inventory,
    plan: &Plan,
    parent: &Path,
    pack_name: &str,
    cancel: Option<&AtomicBool>,
    progress: &mut dyn FnMut(&Progress),
) -> Summary {
    let root = unique(&parent.join(safe_name(pack_name)));
    let excluded: HashSet<String> = inv
        .items
        .iter()
        .filter(|i| i.status == Status::Excluded)
        .map(|i| i.local_path.clone())
        .collect();
    let mut c = Copier {
        excluded,
        cancel,
        progress,
        p: Progress {
            total_files: plan.total_files,
            total_bytes: plan.total_bytes,
            ..Progress::default()
        },
        skipped_unsafe: 0,
    };
    let mut error = None;
    let result = (|| -> io::Result<()> {
        fs::create_dir_all(&root)?;
        for e in &plan.entries {
            let Some(action) = &e.action else { continue };
            let folder = root.join(&e.folders[0]);
            match action {
                Action::CopyFile(src) => {
                    let name = src
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_else(|| e.label.clone());
                    c.copy_file(src, &folder.join(safe_name(&name)))?
                }
                Action::CopyDir { src, name } => {
                    let dest = match name {
                        Some(n) => unique(&folder.join(safe_name(n))),
                        None => folder.clone(),
                    };
                    fs::create_dir_all(&dest)?;
                    c.copy_dir(src, &dest, &e.label)?
                }
                Action::TakeoutZip(src) => c.takeout_zip(src, &root, &e.label)?,
                Action::TakeoutDir(src) => c.takeout_dir(src, src, &root, &e.label)?,
            }
        }
        Ok(())
    })();
    let stopped = c.stopped();
    if let Err(err) = result {
        if !stopped {
            error = Some(err.to_string());
        }
    }
    write_readme_and_manifest(inv, plan, &root, &c.p, stopped || error.is_some());
    Summary {
        folder: root.display().to_string(),
        files_copied: c.p.files_copied,
        bytes_copied: c.p.bytes_copied,
        skipped_unsafe: c.skipped_unsafe,
        stopped,
        error,
    }
}

fn write_readme_and_manifest(
    inv: &Inventory,
    plan: &Plan,
    root: &Path,
    p: &Progress,
    incomplete: bool,
) {
    let mut folders: BTreeMap<&str, Vec<&PlanEntry>> = BTreeMap::new();
    for e in &plan.entries {
        for f in &e.folders {
            folders.entry(f.as_str()).or_default().push(e);
        }
    }
    let mut readme = String::new();
    readme.push_str("WORKING MINDS DATA PACK\n=======================\n\n");
    readme.push_str(
        "This folder was created by Working Minds on your computer. It is a copy of the\n",
    );
    readme.push_str(
        "exports you added, arranged with standard folder names. Your originals were not\n",
    );
    readme.push_str("changed, and nothing was uploaded.\n\n");
    readme.push_str(
        "IMPORTANT: this folder contains your actual data (messages, documents, email and\n",
    );
    readme.push_str("code). Only share it with someone under an agreement you are happy with.\n\n");
    if incomplete {
        readme.push_str("NOTE: copying did not finish, so this pack is incomplete.\n\n");
    }
    readme.push_str("What is in each folder\n----------------------\n");
    for (folder, entries) in &folders {
        let names: Vec<&str> = entries.iter().map(|e| e.collection.as_str()).collect();
        readme.push_str(&format!("{folder}/\n    from: {}\n", names.join("; ")));
    }
    readme.push_str(&format!(
        "\nAssessment {} · {} files copied · {} excluded by you and left out.\n",
        inv.assessment_id, p.files_copied, plan.excluded_files
    ));
    let _ = fs::write(root.join(README_NAME), readme);
    let manifest = serde_json::json!({
        "tool": "Working Minds",
        "version": env!("CARGO_PKG_VERSION"),
        "assessment_id": inv.assessment_id,
        "complete": !incomplete,
        "files_copied": p.files_copied,
        "bytes_copied": p.bytes_copied,
        "excluded_files_left_out": plan.excluded_files,
        "folders": plan.entries.iter().map(|e| serde_json::json!({
            "from": e.collection, "kind": e.kind, "into": e.folders, "files": e.files,
        })).collect::<Vec<_>>(),
    });
    let _ = fs::write(
        root.join(MANIFEST_NAME),
        serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
    );
}
