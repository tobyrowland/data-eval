//! The inventory scanner: walks the user's selections, expands supported
//! archives in memory with bounded readers (nothing is extracted to disk),
//! and gives every logical item one explicit status.

use crate::analyze::{slack::SlackRecogniser, tabular, text, Outcome};
use crate::detect::{detect, SNIFF_LEN};
use crate::guard::{limit_reason, read_head, Budget, MeteredReader, RatioGuard};
use crate::limits::Limits;
use crate::model::{Collection, Format, Item, Reason, SourceKind, Status};
use crate::placeholder::is_online_only;
use std::collections::BTreeSet;
use std::fs::{self, File, Metadata};
use std::io::{self, Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Debug, Clone, Default)]
pub struct ScanOptions {
    pub limits: Limits,
    /// Paths to exclude, relative to a selection and '/'-separated, e.g.
    /// `export.zip/random` or `Drive/Finance`. Prefix match on whole segments.
    pub exclude: Vec<String>,
}

/// Per-collection text statistics gathered by content parsers.
#[derive(Debug, Clone, Default)]
pub struct TextTotals {
    pub text_bytes: u64,
    pub words: u64,
}

/// The private local result of a scan. Contains local paths and hashes and
/// must never be shared directly; reports are derived views of it.
pub struct Inventory {
    pub items: Vec<Item>,
    pub collections: Vec<Collection>,
    pub slack: Vec<SlackRecogniser>,
    pub text: Vec<TextTotals>,
    pub limits_reached: BTreeSet<Reason>,
    pub expanded_bytes_read: u64,
    pub limits: Limits,
}

struct Ctx {
    parent: Option<u64>,
    collection: u32,
    path: String,
    depth: u8,
    stored: u64,
}

struct Scanner {
    opts: ScanOptions,
    items: Vec<Item>,
    collections: Vec<Collection>,
    slack: Vec<SlackRecogniser>,
    text: Vec<TextTotals>,
    budget: Budget,
    records_total: u64,
    limits_reached: BTreeSet<Reason>,
}

/// Scans the selected files and folders. Reads only what was selected.
pub fn scan(inputs: &[PathBuf], opts: ScanOptions) -> Inventory {
    let mut s = Scanner {
        budget: Budget::new(opts.limits.max_expanded_bytes),
        opts,
        items: Vec::new(),
        collections: Vec::new(),
        slack: Vec::new(),
        text: Vec::new(),
        records_total: 0,
        limits_reached: BTreeSet::new(),
    };
    for (idx, input) in inputs.iter().enumerate() {
        let collection = idx as u32;
        let label = input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| format!("selection-{}", idx + 1));
        s.collections.push(Collection {
            id: collection,
            label: label.clone(),
            source_kind: SourceKind::Generic,
        });
        s.slack.push(SlackRecogniser::default());
        s.text.push(TextTotals::default());
        s.visit_path(input, label, collection);
    }
    for c in &mut s.collections {
        if !s.slack[c.id as usize].inventories().is_empty() {
            c.source_kind = SourceKind::SlackExport;
        }
    }
    Inventory {
        expanded_bytes_read: s.budget.used(),
        items: s.items,
        collections: s.collections,
        slack: s.slack,
        text: s.text,
        limits_reached: s.limits_reached,
        limits: s.opts.limits,
    }
}

impl Scanner {
    fn file_limit_reached(&mut self) -> bool {
        if self.items.len() as u64 >= self.opts.limits.max_logical_files {
            self.limits_reached.insert(Reason::LogicalFileLimit);
            true
        } else {
            false
        }
    }

    fn is_excluded(&self, path: &str) -> bool {
        self.opts.exclude.iter().any(|ex| {
            let ex = ex.trim_matches('/');
            !ex.is_empty() && (path == ex || path.starts_with(&format!("{ex}/")))
        })
    }

    fn push(&mut self, ctx: &Ctx, format: Format, status: Status, reason: Option<Reason>) -> u64 {
        let id = self.items.len() as u64;
        self.items.push(Item {
            id,
            parent: ctx.parent,
            collection: ctx.collection,
            local_path: ctx.path.clone(),
            archive_depth: ctx.depth,
            format,
            status,
            reason,
            stored_bytes: ctx.stored,
            expanded_bytes: 0,
            sha256: None,
            records: None,
        });
        id
    }

    fn set(&mut self, id: u64, status: Status, reason: Option<Reason>) {
        let it = &mut self.items[id as usize];
        it.status = status;
        it.reason = reason;
    }

    // ---------------------------------------------------------------- disk

    fn visit_path(&mut self, path: &Path, logical: String, collection: u32) {
        if self.file_limit_reached() {
            return;
        }
        let ctx = Ctx {
            parent: None,
            collection,
            path: logical.clone(),
            depth: 0,
            stored: 0,
        };
        let md = match fs::symlink_metadata(path) {
            Ok(m) => m,
            Err(e) => {
                let (st, r) = io_status(&e);
                self.push(&ctx, detect(&logical, &[]), st, Some(r));
                return;
            }
        };
        if md.file_type().is_symlink() {
            self.push(
                &ctx,
                detect(&logical, &[]),
                Status::Inventoried,
                Some(Reason::SymlinkNotFollowed),
            );
        } else if md.is_dir() {
            self.visit_dir(path, &logical, collection);
        } else if md.is_file() {
            self.visit_file(path, &md, ctx);
        }
    }

    fn visit_dir(&mut self, dir: &Path, logical: &str, collection: u32) {
        let entries = match fs::read_dir(dir) {
            Ok(rd) => {
                let mut v: Vec<_> = rd.filter_map(Result::ok).collect();
                v.sort_by_key(|e| e.file_name());
                v
            }
            Err(e) => {
                let ctx = Ctx {
                    parent: None,
                    collection,
                    path: logical.to_string(),
                    depth: 0,
                    stored: 0,
                };
                let (st, r) = io_status(&e);
                self.push(&ctx, Format::Unknown, st, Some(r));
                return;
            }
        };
        for e in entries {
            let child = format!("{logical}/{}", e.file_name().to_string_lossy());
            self.visit_path(&e.path(), child, collection);
            if self.limits_reached.contains(&Reason::LogicalFileLimit) {
                return;
            }
        }
    }

    fn visit_file(&mut self, path: &Path, md: &Metadata, mut ctx: Ctx) {
        ctx.stored = md.len();
        let by_name = detect(&ctx.path, &[]);
        if self.is_excluded(&ctx.path) {
            self.push(&ctx, by_name, Status::Excluded, Some(Reason::UserExcluded));
            return;
        }
        // Checked from metadata before any open: never trigger a cloud recall.
        if is_online_only(md) {
            self.push(
                &ctx,
                by_name,
                Status::Unavailable,
                Some(Reason::OnlineOnlyPlaceholder),
            );
            return;
        }
        if self.budget.exhausted() {
            self.limits_reached.insert(Reason::ExpansionLimit);
            self.push(
                &ctx,
                by_name,
                Status::StoppedSafely,
                Some(Reason::ExpansionLimit),
            );
            return;
        }
        let mut f = match File::open(path) {
            Ok(f) => f,
            Err(e) => {
                let (st, r) = io_status(&e);
                self.push(&ctx, by_name, st, Some(r));
                return;
            }
        };
        let before = fingerprint(md);
        let first = self.items.len() as u64;
        self.process_stream(ctx, &mut f, Some(path));
        // A file that changed while being read is not silently mixed in.
        let after = fs::symlink_metadata(path).ok().map(|m| fingerprint(&m));
        if after != Some(before) && (first as usize) < self.items.len() {
            self.set(
                first,
                Status::StoppedSafely,
                Some(Reason::ChangedDuringScan),
            );
        }
    }

    // -------------------------------------------------------------- stream

    /// Handles one logical file from any source. `disk` is set when the
    /// stream is a file on disk, which lets ZIP use random access directly.
    fn process_stream(&mut self, ctx: Ctx, reader: &mut dyn Read, disk: Option<&Path>) {
        let head = match read_head(reader, SNIFF_LEN) {
            Ok(h) => h,
            Err(e) => {
                let reason = limit_reason(&e).unwrap_or(Reason::ReadError);
                self.note_limit(&reason);
                self.push(
                    &ctx,
                    detect(&ctx.path, &[]),
                    Status::StoppedSafely,
                    Some(reason),
                );
                return;
            }
        };
        let format = detect(&ctx.path, &head);
        let id = self.push(&ctx, format, Status::Inventoried, None);
        let mut full = Cursor::new(head).chain(reader);

        if !format.is_container() {
            self.analyze_leaf(id, format, &ctx, &mut full);
            return;
        }
        if !matches!(format, Format::Zip | Format::Tar | Format::Gzip) {
            self.set(id, Status::Inventoried, Some(Reason::NoParserInThisRelease));
            return;
        }
        if ctx.depth > self.opts.limits.max_archive_nesting {
            self.note_limit(&Reason::NestingLimit);
            self.set(id, Status::StoppedSafely, Some(Reason::NestingLimit));
            return;
        }
        if ctx.stored > self.opts.limits.max_archive_bytes {
            self.set(id, Status::StoppedSafely, Some(Reason::ObjectTooLarge));
            return;
        }
        let result = match format {
            Format::Zip => match disk {
                Some(p) => match File::open(p) {
                    Ok(f) => self.expand_zip(id, &ctx, f),
                    Err(e) => Err(io_status(&e).1),
                },
                None => {
                    let cap = self.opts.limits.max_in_memory_archive_bytes as usize;
                    match read_head(&mut full, cap + 1) {
                        Ok(buf) if buf.len() <= cap => self.expand_zip(id, &ctx, Cursor::new(buf)),
                        Ok(_) => Err(Reason::ObjectTooLarge),
                        Err(e) => Err(limit_reason(&e).unwrap_or(Reason::ReadError)),
                    }
                }
            },
            Format::Tar => self.expand_tar(id, &ctx, &mut full),
            _ => self.expand_gzip(id, &ctx, &mut full),
        };
        let expanded: u64 = self.items[id as usize + 1..]
            .iter()
            .filter(|i| !(i.format.is_container() && i.status.content_examined()))
            .map(|i| i.expanded_bytes)
            .sum();
        self.items[id as usize].expanded_bytes = expanded;
        match result {
            Ok(None) => self.set(id, Status::Assessed, None),
            Ok(Some(r)) => {
                self.note_limit(&r);
                self.set(id, Status::PartlyAssessed, Some(r));
            }
            Err(r) => {
                self.note_limit(&r);
                let st = if r == Reason::PermissionDenied {
                    Status::Locked
                } else {
                    Status::StoppedSafely
                };
                self.set(id, st, Some(r));
            }
        }
    }

    fn note_limit(&mut self, r: &Reason) {
        if matches!(
            r,
            Reason::ExpansionLimit
                | Reason::CompressionRatio
                | Reason::NestingLimit
                | Reason::MemberCountLimit
                | Reason::LogicalFileLimit
                | Reason::RecordLimit
        ) {
            self.limits_reached.insert(r.clone());
        }
    }

    fn child_ctx(&self, parent: u64, ctx: &Ctx, name: &str, stored: u64, depth: u8) -> Ctx {
        Ctx {
            parent: Some(parent),
            collection: ctx.collection,
            path: format!("{}/{}", ctx.path, name.trim_start_matches('/')),
            depth,
            stored,
        }
    }

    /// Returns Ok(None) for a complete enumeration, Ok(Some(reason)) for a
    /// partial one, Err for an unreadable container.
    fn expand_zip<R: Read + Seek>(
        &mut self,
        id: u64,
        ctx: &Ctx,
        r: R,
    ) -> Result<Option<Reason>, Reason> {
        let mut zip = zip::ZipArchive::new(r).map_err(|_| Reason::CorruptStructure)?;
        let lim = self.opts.limits.clone();
        let mut partial = None;
        let n = zip.len();
        let take = n.min(lim.max_archive_members as usize);
        if take < n {
            partial = Some(Reason::MemberCountLimit);
        }
        for i in 0..take {
            let (name, is_dir, encrypted, csize, safe, symlink) = match zip.by_index_raw(i) {
                Ok(f) => (
                    f.name().to_string(),
                    f.is_dir(),
                    f.encrypted(),
                    f.compressed_size(),
                    f.enclosed_name().is_some(),
                    f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000),
                ),
                Err(_) => {
                    partial = Some(Reason::CorruptStructure);
                    continue;
                }
            };
            if is_dir {
                continue;
            }
            if self.file_limit_reached() {
                return Ok(Some(Reason::LogicalFileLimit));
            }
            let child = self.child_ctx(id, ctx, &name, csize, ctx.depth + 1);
            let by_name = detect(&name, &[]);
            if !safe {
                self.push(
                    &child,
                    by_name,
                    Status::StoppedSafely,
                    Some(Reason::UnsafeMemberPath),
                );
            } else if symlink {
                self.push(
                    &child,
                    by_name,
                    Status::Inventoried,
                    Some(Reason::SymlinkNotFollowed),
                );
            } else if encrypted {
                self.push(
                    &child,
                    by_name,
                    Status::Locked,
                    Some(Reason::PasswordProtected),
                );
            } else if self.is_excluded(&child.path) {
                self.push(
                    &child,
                    by_name,
                    Status::Excluded,
                    Some(Reason::UserExcluded),
                );
            } else if self.budget.exhausted() {
                self.limits_reached.insert(Reason::ExpansionLimit);
                self.push(
                    &child,
                    by_name,
                    Status::StoppedSafely,
                    Some(Reason::ExpansionLimit),
                );
            } else {
                match zip.by_index(i) {
                    Ok(f) => {
                        let mut g = RatioGuard::new(
                            f,
                            csize,
                            lim.max_compression_ratio,
                            lim.ratio_check_floor_bytes,
                            lim.max_content_object_bytes,
                        );
                        self.process_stream(child, &mut g, None);
                    }
                    Err(zip::result::ZipError::UnsupportedArchive(_)) => {
                        self.push(
                            &child,
                            by_name,
                            Status::Inventoried,
                            Some(Reason::NoParserInThisRelease),
                        );
                    }
                    Err(_) => {
                        self.push(
                            &child,
                            by_name,
                            Status::StoppedSafely,
                            Some(Reason::CorruptStructure),
                        );
                    }
                }
            }
        }
        Ok(partial)
    }

    fn expand_tar(
        &mut self,
        id: u64,
        ctx: &Ctx,
        r: &mut dyn Read,
    ) -> Result<Option<Reason>, Reason> {
        let mut ar = tar::Archive::new(r);
        let entries = ar.entries().map_err(|_| Reason::CorruptStructure)?;
        let mut members: u64 = 0;
        for entry in entries {
            let mut entry = match entry {
                Ok(e) => e,
                Err(e) => return Ok(Some(limit_reason(&e).unwrap_or(Reason::CorruptStructure))),
            };
            let kind = entry.header().entry_type();
            if kind.is_dir() || kind.is_pax_global_extensions() || kind.is_pax_local_extensions() {
                continue;
            }
            members += 1;
            if members > self.opts.limits.max_archive_members {
                return Ok(Some(Reason::MemberCountLimit));
            }
            if self.file_limit_reached() {
                return Ok(Some(Reason::LogicalFileLimit));
            }
            let name = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
            let size = entry.header().size().unwrap_or(0);
            let child = self.child_ctx(id, ctx, &name, size, ctx.depth + 1);
            let by_name = detect(&name, &[]);
            if !member_path_is_safe(&name) {
                self.push(
                    &child,
                    by_name,
                    Status::StoppedSafely,
                    Some(Reason::UnsafeMemberPath),
                );
            } else if kind.is_symlink() || kind.is_hard_link() {
                self.push(
                    &child,
                    by_name,
                    Status::Inventoried,
                    Some(Reason::SymlinkNotFollowed),
                );
            } else if !kind.is_file() && !kind.is_contiguous() && !kind.is_gnu_sparse() {
                self.push(
                    &child,
                    by_name,
                    Status::Inventoried,
                    Some(Reason::NoParserInThisRelease),
                );
            } else if self.is_excluded(&child.path) {
                self.push(
                    &child,
                    by_name,
                    Status::Excluded,
                    Some(Reason::UserExcluded),
                );
            } else if self.budget.exhausted() {
                self.limits_reached.insert(Reason::ExpansionLimit);
                self.push(
                    &child,
                    by_name,
                    Status::StoppedSafely,
                    Some(Reason::ExpansionLimit),
                );
            } else {
                self.process_stream(child, &mut entry, None);
            }
        }
        Ok(None)
    }

    fn expand_gzip(
        &mut self,
        id: u64,
        ctx: &Ctx,
        r: &mut dyn Read,
    ) -> Result<Option<Reason>, Reason> {
        let lim = self.opts.limits.clone();
        let base = ctx.path.rsplit('/').next().unwrap_or("");
        let lower = base.to_ascii_lowercase();
        let inner = if lower.ends_with(".tgz") {
            format!("{}.tar", &base[..base.len() - 4])
        } else if lower.ends_with(".gz") {
            base[..base.len() - 3].to_string()
        } else {
            format!("{base}.decompressed")
        };
        // A gzip wrapper (e.g. .tar.gz) does not consume a nesting level,
        // except gzip directly inside gzip, which keeps recursion bounded.
        let parent_is_gzip = ctx
            .parent
            .is_some_and(|p| self.items[p as usize].format == Format::Gzip);
        let depth = if parent_is_gzip {
            ctx.depth + 1
        } else {
            ctx.depth
        };
        let mut g = RatioGuard::new(
            flate2::read::MultiGzDecoder::new(r),
            ctx.stored,
            lim.max_compression_ratio,
            lim.ratio_check_floor_bytes,
            lim.max_content_object_bytes,
        );
        let child = self.child_ctx(id, ctx, &inner, ctx.stored, depth);
        if self.is_excluded(&child.path) {
            self.push(
                &child,
                detect(&inner, &[]),
                Status::Excluded,
                Some(Reason::UserExcluded),
            );
            return Ok(None);
        }
        self.process_stream(child, &mut g, None);
        Ok(None)
    }

    // ---------------------------------------------------------------- leaf

    fn analyze_leaf(&mut self, id: u64, format: Format, ctx: &Ctx, reader: &mut dyn Read) {
        let lim = self.opts.limits.clone();
        let mut m = MeteredReader::new(reader, self.budget.clone(), lim.max_content_object_bytes);
        let structured = matches!(
            format,
            Format::Json | Format::Jsonl | Format::Csv | Format::Tsv | Format::Mbox
        );
        let res: io::Result<Outcome> =
            if structured && self.records_total >= lim.max_structured_records {
                self.limits_reached.insert(Reason::RecordLimit);
                Ok(Outcome::with(Status::PartlyAssessed, Reason::RecordLimit))
            } else {
                match format {
                    Format::Json => tabular::json(&mut m, lim.max_json_document_bytes as usize),
                    Format::Jsonl => tabular::jsonl(&mut m),
                    Format::Csv => tabular::delimited(&mut m, b','),
                    Format::Tsv => tabular::delimited(&mut m, b'\t'),
                    Format::PlainText | Format::Markdown | Format::Html | Format::Xml => {
                        text::analyze(&mut m, false, false)
                    }
                    Format::Mbox => text::analyze(&mut m, true, false),
                    Format::Eml => text::analyze(&mut m, false, true),
                    _ => Ok(Outcome::inventoried()),
                }
            };
        let (mut outcome, complete) = match res {
            Ok(o) => match m.drain() {
                Ok(()) => (o, true),
                Err(e) => {
                    let reason = limit_reason(&e).unwrap_or(Reason::ReadError);
                    let st = if o.status.content_examined() {
                        Status::PartlyAssessed
                    } else {
                        Status::StoppedSafely
                    };
                    (
                        Outcome {
                            status: st,
                            reason: Some(reason),
                            ..o
                        },
                        false,
                    )
                }
            },
            Err(e) => {
                let reason = limit_reason(&e).unwrap_or(Reason::CorruptStructure);
                (Outcome::with(Status::StoppedSafely, reason), false)
            }
        };
        if let Some(r) = &outcome.reason {
            self.note_limit(&r.clone());
        }
        let bytes = m.bytes_read();
        let sha = complete.then(|| m.finish_hash());
        if let Some(doc) = outcome.json.take() {
            self.slack[ctx.collection as usize].offer(&ctx.path, &doc);
        }
        if let Some(n) = outcome.records {
            self.records_total += n;
        }
        let t = &mut self.text[ctx.collection as usize];
        t.text_bytes += outcome.text_bytes;
        t.words += outcome.words;
        let it = &mut self.items[id as usize];
        it.status = outcome.status;
        it.reason = outcome.reason;
        it.records = outcome.records;
        it.expanded_bytes = bytes;
        it.sha256 = sha;
    }
}

fn io_status(e: &io::Error) -> (Status, Reason) {
    if e.kind() == io::ErrorKind::PermissionDenied {
        (Status::Locked, Reason::PermissionDenied)
    } else {
        (Status::StoppedSafely, Reason::ReadError)
    }
}

fn fingerprint(md: &Metadata) -> (u64, Option<SystemTime>) {
    (md.len(), md.modified().ok())
}

/// Archive member names must stay inside their container. Nothing is ever
/// extracted to disk, but unsafe names are flagged rather than trusted.
pub fn member_path_is_safe(name: &str) -> bool {
    let n = name.replace('\\', "/");
    !(n.starts_with('/')
        || n.as_bytes().get(1) == Some(&b':')
        || n.split('/').any(|seg| seg == ".."))
}
