//! Reads commit history from a `.git` folder: loose objects and packfiles
//! (with delta resolution), walking from every branch, tag and remote ref.
//! Only commits are read; trees and file contents are never opened.

use flate2::read::ZlibDecoder;
use std::collections::{HashMap, HashSet, VecDeque};
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

pub struct Commit {
    pub sha: String,
    pub time: Option<i64>,
    pub author_email: String,
    pub message: String,
}

pub struct Repo {
    pub commits: Vec<Commit>,
    /// `host/owner/repo` of the first remote, if any.
    pub remote: Option<String>,
}

const MAX_COMMITS: usize = 500_000;
const MAX_OBJECT: usize = 16 * 1024 * 1024;

struct Pack {
    pack: PathBuf,
    /// Sorted object names and their offsets.
    names: Vec<[u8; 20]>,
    offsets: Vec<u64>,
}

struct Store {
    objects: PathBuf,
    packs: Vec<Pack>,
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

fn unhex(s: &str) -> Option<[u8; 20]> {
    if s.len() != 40 {
        return None;
    }
    let mut out = [0u8; 20];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

fn read_idx(path: &Path) -> io::Result<(Vec<[u8; 20]>, Vec<u64>)> {
    let b = fs::read(path)?;
    let bad = || io::Error::new(io::ErrorKind::InvalidData, "pack index");
    if b.len() < 8 + 256 * 4 || &b[..4] != b"\xfftOc" || b[4..8] != [0, 0, 0, 2] {
        return Err(bad());
    }
    let be32 = |o: usize| -> io::Result<u32> {
        Ok(u32::from_be_bytes(
            b.get(o..o + 4).ok_or_else(bad)?.try_into().unwrap(),
        ))
    };
    let n = be32(8 + 255 * 4)? as usize;
    let names_at = 8 + 256 * 4;
    let off_at = names_at + n * 20 + n * 4;
    let big_at = off_at + n * 4;
    let mut names = Vec::with_capacity(n);
    let mut offsets = Vec::with_capacity(n);
    for i in 0..n {
        let name: [u8; 20] = b
            .get(names_at + i * 20..names_at + i * 20 + 20)
            .ok_or_else(bad)?
            .try_into()
            .unwrap();
        names.push(name);
        let o = be32(off_at + i * 4)?;
        let off = if o & 0x8000_0000 != 0 {
            let j = (o & 0x7fff_ffff) as usize;
            u64::from_be_bytes(
                b.get(big_at + j * 8..big_at + j * 8 + 8)
                    .ok_or_else(bad)?
                    .try_into()
                    .unwrap(),
            )
        } else {
            o as u64
        };
        offsets.push(off);
    }
    Ok((names, offsets))
}

impl Store {
    fn open(git: &Path) -> Store {
        let objects = git.join("objects");
        let mut packs = Vec::new();
        if let Ok(rd) = fs::read_dir(objects.join("pack")) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == "idx") {
                    if let Ok((names, offsets)) = read_idx(&p) {
                        packs.push(Pack {
                            pack: p.with_extension("pack"),
                            names,
                            offsets,
                        });
                    }
                }
            }
        }
        Store { objects, packs }
    }

    /// An object's type (1 commit, 2 tree, 3 blob, 4 tag) and content.
    fn read(&self, id: &[u8; 20], depth: u8) -> Option<(u8, Vec<u8>)> {
        let h = hex(id);
        let loose = self.objects.join(&h[..2]).join(&h[2..]);
        if let Ok(f) = File::open(&loose) {
            let mut raw = Vec::new();
            ZlibDecoder::new(f)
                .take(MAX_OBJECT as u64)
                .read_to_end(&mut raw)
                .ok()?;
            let nul = raw.iter().position(|b| *b == 0)?;
            let head = std::str::from_utf8(&raw[..nul]).ok()?;
            let kind = match head.split(' ').next()? {
                "commit" => 1,
                "tree" => 2,
                "blob" => 3,
                "tag" => 4,
                _ => return None,
            };
            return Some((kind, raw[nul + 1..].to_vec()));
        }
        for p in &self.packs {
            if let Ok(i) = p.names.binary_search(id) {
                return self.read_packed(p, p.offsets[i], depth);
            }
        }
        None
    }

    fn read_packed(&self, p: &Pack, offset: u64, depth: u8) -> Option<(u8, Vec<u8>)> {
        if depth > 50 {
            return None;
        }
        let mut f = File::open(&p.pack).ok()?;
        f.seek(SeekFrom::Start(offset)).ok()?;
        let mut r = io::BufReader::new(f);
        let mut byte = [0u8; 1];
        r.read_exact(&mut byte).ok()?;
        let kind = (byte[0] >> 4) & 7;
        let mut size = (byte[0] & 15) as u64;
        let mut shift = 4;
        while byte[0] & 0x80 != 0 {
            r.read_exact(&mut byte).ok()?;
            size |= ((byte[0] & 0x7f) as u64) << shift;
            shift += 7;
        }
        if size as usize > MAX_OBJECT {
            return None;
        }
        let inflate = |r: &mut io::BufReader<File>| -> Option<Vec<u8>> {
            let mut out = Vec::with_capacity(size as usize);
            ZlibDecoder::new(r).take(size).read_to_end(&mut out).ok()?;
            Some(out)
        };
        match kind {
            1..=4 => Some((kind, inflate(&mut r)?)),
            6 => {
                // Offset delta: base at offset - n.
                r.read_exact(&mut byte).ok()?;
                let mut n = (byte[0] & 0x7f) as u64;
                while byte[0] & 0x80 != 0 {
                    r.read_exact(&mut byte).ok()?;
                    n = ((n + 1) << 7) | (byte[0] & 0x7f) as u64;
                }
                let delta = inflate(&mut r)?;
                let (bk, base) = self.read_packed(p, offset.checked_sub(n)?, depth + 1)?;
                Some((bk, apply_delta(&base, &delta)?))
            }
            7 => {
                let mut id = [0u8; 20];
                r.read_exact(&mut id).ok()?;
                let delta = inflate(&mut r)?;
                let (bk, base) = self.read(&id, depth + 1)?;
                Some((bk, apply_delta(&base, &delta)?))
            }
            _ => None,
        }
    }
}

fn apply_delta(base: &[u8], d: &[u8]) -> Option<Vec<u8>> {
    let mut i = 0;
    let varint = |i: &mut usize| -> Option<usize> {
        let mut v = 0usize;
        let mut shift = 0;
        loop {
            let b = *d.get(*i)?;
            *i += 1;
            v |= ((b & 0x7f) as usize) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                return Some(v);
            }
        }
    };
    let src = varint(&mut i)?;
    let dst = varint(&mut i)?;
    if src != base.len() || dst > MAX_OBJECT {
        return None;
    }
    let mut out = Vec::with_capacity(dst);
    while i < d.len() {
        let op = d[i];
        i += 1;
        if op & 0x80 != 0 {
            let mut off = 0usize;
            let mut len = 0usize;
            for k in 0..4 {
                if op & (1 << k) != 0 {
                    off |= (*d.get(i)? as usize) << (8 * k);
                    i += 1;
                }
            }
            for k in 0..3 {
                if op & (0x10 << k) != 0 {
                    len |= (*d.get(i)? as usize) << (8 * k);
                    i += 1;
                }
            }
            if len == 0 {
                len = 0x10000;
            }
            out.extend_from_slice(base.get(off..off + len)?);
        } else if op > 0 {
            out.extend_from_slice(d.get(i..i + op as usize)?);
            i += op as usize;
        } else {
            return None;
        }
    }
    (out.len() == dst).then_some(out)
}

/// Every ref's target: HEAD, branches, remotes, tags, packed refs.
fn refs(git: &Path) -> Vec<[u8; 20]> {
    let mut out = Vec::new();
    let mut push = |s: &str| {
        if let Some(id) = unhex(s.trim()) {
            out.push(id);
        }
    };
    if let Ok(h) = fs::read_to_string(git.join("HEAD")) {
        push(&h);
    }
    if let Ok(p) = fs::read_to_string(git.join("packed-refs")) {
        for l in p.lines() {
            let l = l.trim_start_matches('^');
            if let Some((id, _)) = l.split_once(' ') {
                push(id);
            }
        }
    }
    fn walk(dir: &Path, out: &mut dyn FnMut(&str), depth: u8) {
        if depth > 8 {
            return;
        }
        if let Ok(rd) = fs::read_dir(dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, out, depth + 1);
                } else if let Ok(s) = fs::read_to_string(&p) {
                    out(&s);
                }
            }
        }
    }
    walk(&git.join("refs"), &mut push, 0);
    out
}

/// Reads the commits reachable from any ref in a `.git` folder.
pub fn read_repo(git: &Path) -> Repo {
    let store = Store::open(git);
    let remote = fs::read_to_string(git.join("config"))
        .ok()
        .and_then(|c| crate::analyze::sources::git_remotes(&c).into_iter().next());
    let mut seen: HashSet<[u8; 20]> = HashSet::new();
    let mut queue: VecDeque<[u8; 20]> = refs(git).into_iter().collect();
    let mut commits = Vec::new();
    let mut tag_depth: HashMap<[u8; 20], u8> = HashMap::new();
    while let Some(id) = queue.pop_front() {
        if commits.len() >= MAX_COMMITS || !seen.insert(id) {
            continue;
        }
        let Some((kind, body)) = store.read(&id, 0) else {
            continue;
        };
        let text = String::from_utf8_lossy(&body);
        if kind == 4 {
            // Annotated tag: follow to its object.
            let d = tag_depth.get(&id).copied().unwrap_or(0);
            if let Some(t) = text
                .lines()
                .find_map(|l| l.strip_prefix("object "))
                .and_then(unhex)
            {
                if d < 4 {
                    tag_depth.insert(t, d + 1);
                    queue.push_back(t);
                }
            }
            continue;
        }
        if kind != 1 {
            continue;
        }
        let (head, message) = text.split_once("\n\n").unwrap_or((&text, ""));
        let mut time = None;
        let mut email = String::new();
        for l in head.lines() {
            if let Some(p) = l.strip_prefix("parent ") {
                if let Some(pid) = unhex(p) {
                    queue.push_back(pid);
                }
            } else if let Some(a) = l.strip_prefix("author ") {
                if let (Some(s), Some(e)) = (a.find('<'), a.find('>')) {
                    email = a[s + 1..e].to_ascii_lowercase();
                    time = a[e + 1..]
                        .split_whitespace()
                        .next()
                        .and_then(|t| t.parse().ok());
                }
            }
        }
        commits.push(Commit {
            sha: hex(&id),
            time,
            author_email: email,
            message: message.trim().to_string(),
        });
    }
    Repo { commits, remote }
}
