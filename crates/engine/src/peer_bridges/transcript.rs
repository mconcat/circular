
use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TailedLine {
    pub path: PathBuf,
    pub file: FileIdentity,
    pub offset: u64,
    pub line: String,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileIdentity {
    pub device: u64,
    pub inode: u64,
}

impl FileIdentity {
    fn of(metadata: &std::fs::Metadata) -> Self {
        use std::os::unix::fs::MetadataExt as _;
        Self {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }
}

struct Tracked {
    file: Option<File>,
    identity: FileIdentity,
    offset: u64,
}

enum Unfollowed {
    Gone,
    Unreadable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum GlobRejection {
    NotAbsolute,
    UnpublishedSyntax { character: char },
    MixedGlobstar,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Glob {
    segments: Vec<Segment>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Segment {
    AnyDepth,
    Pattern(Vec<Piece>),
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum Piece {
    Literal(String),
    AnyRun,
}

impl Glob {
    pub fn parse(pattern: &str) -> Result<Self, GlobRejection> {
        if !pattern.starts_with('/') {
            return Err(GlobRejection::NotAbsolute);
        }
        let mut segments = Vec::new();
        for raw in pattern.split('/').filter(|part| !part.is_empty()) {
            if let Some(character) = raw
                .chars()
                .find(|character| matches!(character, '?' | '[' | ']' | '{' | '}'))
            {
                return Err(GlobRejection::UnpublishedSyntax { character });
            }
            if raw.contains("**") {
                if raw != "**" {
                    return Err(GlobRejection::MixedGlobstar);
                }
                segments.push(Segment::AnyDepth);
                continue;
            }
            let mut pieces = Vec::new();
            let mut literal = String::new();
            for character in raw.chars() {
                if character == '*' {
                    if !literal.is_empty() {
                        pieces.push(Piece::Literal(std::mem::take(&mut literal)));
                    }
                    pieces.push(Piece::AnyRun);
                } else {
                    literal.push(character);
                }
            }
            if !literal.is_empty() {
                pieces.push(Piece::Literal(literal));
            }
            segments.push(Segment::Pattern(pieces));
        }
        Ok(Self { segments })
    }

    #[must_use]
    pub fn matches(&self, path: &Path) -> bool {
        let parts = path
            .components()
            .filter_map(|component| match component {
                std::path::Component::Normal(part) => part.to_str(),
                _ => None,
            })
            .collect::<Vec<_>>();
        matches_from(&self.segments, &parts)
    }

    #[must_use]
    pub fn root(&self) -> PathBuf {
        let mut root = PathBuf::from("/");
        for segment in &self.segments {
            match segment {
                Segment::Pattern(pieces) => match pieces.as_slice() {
                    [Piece::Literal(literal)] => root.push(literal),
                    _ => break,
                },
                Segment::AnyDepth => break,
            }
        }
        root
    }
}

fn matches_from(segments: &[Segment], parts: &[&str]) -> bool {
    match segments.split_first() {
        None => parts.is_empty(),
        Some((Segment::AnyDepth, rest)) => {
            (0..=parts.len()).any(|skipped| matches_from(rest, &parts[skipped..]))
        }
        Some((Segment::Pattern(pieces), rest)) => match parts.split_first() {
            None => false,
            Some((part, remaining)) => {
                matches_pieces(pieces, part) && matches_from(rest, remaining)
            }
        },
    }
}

fn matches_pieces(pieces: &[Piece], part: &str) -> bool {
    match pieces.split_first() {
        None => part.is_empty(),
        Some((Piece::Literal(literal), rest)) => part
            .strip_prefix(literal.as_str())
            .is_some_and(|remaining| matches_pieces(rest, remaining)),
        Some((Piece::AnyRun, rest)) => (0..=part.len())
            .any(|taken| part.is_char_boundary(taken) && matches_pieces(rest, &part[taken..])),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TailDiagnostic {
    RootUnreadable { root: PathBuf },
    FileUnreadable { path: PathBuf },
    NotUtf8 { path: PathBuf, offset: u64 },
}

pub(crate) fn tail_path_allowed(roots: &circular_runtime::PathScopes, path: &Path) -> bool {
    let Ok(normal) = circular_runtime::NormalizedPath::new(path) else {
        return false;
    };
    let Ok(resolved) = std::fs::canonicalize(path) else {
        return false;
    };
    roots.iter().any(|scope| {
        scope.contains(&normal)
            && std::fs::canonicalize(scope.root().as_path())
                .is_ok_and(|root| resolved.starts_with(root))
    })
}

pub struct FileTail {
    glob: Glob,
    roots: circular_runtime::PathScopes,
    files: BTreeMap<PathBuf, Tracked>,
    unreadable: BTreeSet<PathBuf>,
    root_unreadable: bool,
    primed: bool,
    diagnostics: Vec<TailDiagnostic>,
}

impl FileTail {
    #[must_use]
    pub fn new(glob: Glob, roots: circular_runtime::PathScopes) -> Self {
        Self {
            glob,
            roots,
            files: BTreeMap::new(),
            unreadable: BTreeSet::new(),
            root_unreadable: false,
            primed: false,
            diagnostics: Vec::new(),
        }
    }

    pub fn take_diagnostics(&mut self) -> Vec<TailDiagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    #[must_use]
    pub fn watched(&self) -> usize {
        self.files.len()
    }

    pub fn replay_from_start(&mut self) {
        for tracked in self.files.values_mut() {
            tracked.offset = 0;
        }
        self.primed = true;
    }

    pub fn poll(&mut self) -> Vec<TailedLine> {
        let discovered = self.discover();
        let present = discovered.iter().cloned().collect::<BTreeSet<_>>();
        let mut produced = Vec::new();
        let gone = self
            .files
            .keys()
            .filter(|path| !present.contains(*path))
            .cloned()
            .collect::<Vec<_>>();
        for path in gone {
            let tracked = self.files.remove(&path).expect("a tracked path");
            self.drain(&path, tracked, &mut produced);
        }
        self.unreadable.retain(|path| present.contains(path));
        for path in discovered {
            match self.follow(&path, &mut produced) {
                Ok(()) => {
                    self.unreadable.remove(&path);
                }
                Err(Unfollowed::Gone) => {}
                Err(Unfollowed::Unreadable) => {
                    if self.unreadable.insert(path.clone()) {
                        self.diagnostics
                            .push(TailDiagnostic::FileUnreadable { path });
                    }
                }
            }
        }
        self.primed = true;
        produced
    }

    pub(super) fn discover(&mut self) -> Vec<PathBuf> {
        let root = self.glob.root();
        let mut found = Vec::new();
        if root.is_file() && self.glob.matches(&root) {
            self.root_unreadable = false;
            return vec![root];
        }
        let mut stack = vec![root.clone()];
        let mut opened_any = false;
        while let Some(directory) = stack.pop() {
            let entries = match std::fs::read_dir(&directory) {
                Ok(entries) => {
                    opened_any = true;
                    entries
                }
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                match entry.file_type() {
                    Ok(kind) if kind.is_dir() => stack.push(path),
                    Ok(kind) if kind.is_file() && self.glob.matches(&path) => found.push(path),
                    _ => {}
                }
            }
        }
        if opened_any {
            self.root_unreadable = false;
        } else if !self.root_unreadable {
            self.root_unreadable = true;
            self.diagnostics
                .push(TailDiagnostic::RootUnreadable { root });
        }
        found.sort();
        found
    }

    fn follow(&mut self, path: &Path, produced: &mut Vec<TailedLine>) -> Result<(), Unfollowed> {
        let current = std::fs::metadata(path).map_err(unfollowed)?;
        if !tail_path_allowed(&self.roots, path) {
            return Err(Unfollowed::Unreadable);
        }
        let identity = FileIdentity::of(&current);
        if self
            .files
            .get(path)
            .is_some_and(|tracked| tracked.identity != identity)
        {
            let old = self.files.remove(path).expect("a tracked path");
            self.drain(path, old, produced);
        }
        let primed = self.primed;
        let tracked = self
            .files
            .entry(path.to_path_buf())
            .or_insert_with(|| Tracked {
                file: None,
                identity,
                offset: if primed { 0 } else { current.len() },
            });
        if tracked.file.is_none() {
            let file = File::open(path).map_err(unfollowed)?;
            let opened = FileIdentity::of(&file.metadata().map_err(|_| Unfollowed::Unreadable)?);
            if opened != tracked.identity {
                tracked.identity = opened;
                tracked.offset = 0;
            }
            tracked.file = Some(file);
        }
        let lines = read_lines(path, tracked, &mut self.diagnostics)
            .map_err(|()| Unfollowed::Unreadable)?;
        produced.extend(lines);
        Ok(())
    }

    fn drain(&mut self, path: &Path, mut tracked: Tracked, produced: &mut Vec<TailedLine>) {
        match read_lines(path, &mut tracked, &mut self.diagnostics) {
            Ok(lines) => produced.extend(lines),
            Err(()) => self.diagnostics.push(TailDiagnostic::FileUnreadable {
                path: path.to_path_buf(),
            }),
        }
    }
}

fn unfollowed(error: std::io::Error) -> Unfollowed {
    if error.kind() == std::io::ErrorKind::NotFound {
        Unfollowed::Gone
    } else {
        Unfollowed::Unreadable
    }
}

fn read_lines(
    path: &Path,
    tracked: &mut Tracked,
    diagnostics: &mut Vec<TailDiagnostic>,
) -> Result<Vec<TailedLine>, ()> {
    let Some(file) = tracked.file.as_mut() else {
        return Ok(Vec::new());
    };
    let length = file.metadata().map_err(|_| ())?.len();
    let start = if length < tracked.offset {
        0
    } else {
        tracked.offset
    };
    tracked.offset = start;
    if length == start {
        return Ok(Vec::new());
    }
    file.seek(SeekFrom::Start(start)).map_err(|_| ())?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).map_err(|_| ())?;

    let mut lines = Vec::new();
    let mut consumed = 0_usize;
    for chunk in buffer.split_inclusive(|byte| *byte == b'\n') {
        if !chunk.ends_with(b"\n") {
            break;
        }
        let line_offset = start + consumed as u64;
        consumed += chunk.len();
        let body = &chunk[..chunk.len() - 1];
        let body = body.strip_suffix(b"\r").unwrap_or(body);
        match std::str::from_utf8(body) {
            Ok(text) => lines.push(TailedLine {
                path: path.to_path_buf(),
                file: tracked.identity,
                offset: line_offset,
                line: text.to_owned(),
            }),
            Err(_) => diagnostics.push(TailDiagnostic::NotUtf8 {
                path: path.to_path_buf(),
                offset: line_offset,
            }),
        }
    }
    tracked.offset = start + consumed as u64;
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::{FileTail, Glob, GlobRejection, TailDiagnostic};
    use std::io::Write as _;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicU32, Ordering};

    static NEXT: AtomicU32 = AtomicU32::new(0);

    pub(super) struct Root(pub(super) PathBuf);

    impl Root {
        pub(super) fn new() -> Self {
            let sequence = NEXT.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("cir-tail-{}-{sequence}", std::process::id()));
            std::fs::create_dir_all(&path).expect("create the root");
            Self(path)
        }

        pub(super) fn glob(&self) -> Glob {
            Glob::parse(&format!("{}/**/*.jsonl", self.0.display())).expect("the pattern opens")
        }

        pub(super) fn roots(&self) -> circular_runtime::PathScopes {
            roots_at(&self.0)
        }

        pub(super) fn file(&self, relative: &str) -> PathBuf {
            let path = self.0.join(relative);
            std::fs::create_dir_all(path.parent().expect("has a parent")).expect("directory");
            path
        }
    }

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    pub(super) fn roots_at(path: &Path) -> circular_runtime::PathScopes {
        circular_runtime::PathScopes::new([circular_runtime::PathScope::new(
            circular_runtime::NormalizedPath::new(path).expect("absolute path"),
        )])
    }

    pub(super) fn identity(path: &Path) -> super::FileIdentity {
        use std::os::unix::fs::MetadataExt as _;
        let metadata = std::fs::metadata(path).expect("the file exists");
        super::FileIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        }
    }

    fn append(path: &Path, text: &str) {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .expect("open");
        file.write_all(text.as_bytes()).expect("write");
    }

    fn lines(tail: &mut FileTail) -> Vec<String> {
        tail.poll().into_iter().map(|line| line.line).collect()
    }

    #[test]
    fn a_file_present_at_the_first_poll_starts_at_its_end() {
        let root = Root::new();
        let path = root.file("a/session.jsonl");
        append(&path, "old one\nold two\n");

        let mut tail = FileTail::new(root.glob(), root.roots());
        assert!(
            lines(&mut tail).is_empty(),
            "history poured out as arrivals"
        );

        append(&path, "new one\n");
        assert_eq!(lines(&mut tail), vec!["new one".to_owned()]);
    }

    #[test]
    fn a_file_that_appears_later_starts_at_byte_zero() {
        let root = Root::new();
        let mut tail = FileTail::new(root.glob(), root.roots());
        assert!(lines(&mut tail).is_empty());

        let path = root.file("b/session.jsonl");
        append(&path, "first\nsecond\n");
        assert_eq!(
            lines(&mut tail),
            vec!["first".to_owned(), "second".to_owned()]
        );
    }

    #[test]
    fn a_half_written_line_waits_for_its_newline() {
        let root = Root::new();
        let mut tail = FileTail::new(root.glob(), root.roots());
        lines(&mut tail);

        let path = root.file("c/session.jsonl");
        append(&path, "{\"partial\":");
        assert!(lines(&mut tail).is_empty(), "a half line became a record");

        append(&path, "true}\n");
        assert_eq!(lines(&mut tail), vec!["{\"partial\":true}".to_owned()]);
    }

    #[test]
    fn a_shortened_file_is_read_from_the_start_again() {
        let root = Root::new();
        let path = root.file("d/session.jsonl");
        append(&path, "long line one\nlong line two\n");

        let mut tail = FileTail::new(root.glob(), root.roots());
        lines(&mut tail);

        std::fs::write(&path, "short\n").expect("swap in place");
        assert_eq!(lines(&mut tail), vec!["short".to_owned()]);
    }

    #[test]
    fn a_replay_yields_every_line_again() {
        let root = Root::new();
        let path = root.file("e/session.jsonl");
        append(&path, "one\ntwo\n");

        let mut tail = FileTail::new(root.glob(), root.roots());
        assert!(
            lines(&mut tail).is_empty(),
            "the first scan starts at the end"
        );

        tail.replay_from_start();
        assert_eq!(lines(&mut tail), vec!["one".to_owned(), "two".to_owned()]);

        assert!(lines(&mut tail).is_empty());

        tail.replay_from_start();
        assert_eq!(
            lines(&mut tail).len(),
            2,
            "the second re-emission also yields everything"
        );
    }

    #[test]
    fn a_line_carries_the_offset_it_started_at() {
        let root = Root::new();
        let path = root.file("f/session.jsonl");
        append(&path, "abc\ndefgh\n");

        let mut tail = FileTail::new(root.glob(), root.roots());
        lines(&mut tail);
        tail.replay_from_start();

        let produced = tail.poll();
        assert_eq!(produced[0].offset, 0);
        assert_eq!(produced[1].offset, 4, "the second line follows the first");
        assert_eq!(produced[0].path, path);
    }

    #[test]
    fn a_file_outside_the_pattern_is_not_watched() {
        let root = Root::new();
        let mut tail = FileTail::new(root.glob(), root.roots());
        lines(&mut tail);

        append(&root.file("g/notes.txt"), "ignored\n");
        append(&root.file("g/session.jsonl"), "watched\n");
        assert_eq!(lines(&mut tail), vec!["watched".to_owned()]);
        assert_eq!(
            tail.watched(),
            1,
            "a cursor appeared on a file with a different extension"
        );
    }

    #[test]
    fn a_line_that_is_not_utf8_is_dropped_with_a_diagnostic() {
        let root = Root::new();
        let mut tail = FileTail::new(root.glob(), root.roots());
        lines(&mut tail);

        let path = root.file("h/session.jsonl");
        std::fs::write(&path, b"good\n\xff\xfe\nalso good\n").expect("write");

        assert_eq!(
            lines(&mut tail),
            vec!["good".to_owned(), "also good".to_owned()]
        );
        let diagnostics = tail.take_diagnostics();
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| matches!(diagnostic, TailDiagnostic::NotUtf8 { .. })),
            "a dropped line left no trace: {diagnostics:?}"
        );
    }

    #[test]
    fn a_file_it_cannot_open_is_named_once_and_read_when_it_can_be() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = Root::new();
        let path = root.file("i/session.jsonl");
        append(&path, "secret one\nsecret two\n");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000))
            .expect("revoke the permission");

        let mut tail = FileTail::new(root.glob(), root.roots());
        assert!(tail.poll().is_empty(), "the first scan yielded content");
        assert_eq!(
            tail.take_diagnostics(),
            vec![TailDiagnostic::FileUnreadable { path: path.clone() }],
            "a file that could not be opened left no trace"
        );
        assert!(tail.poll().is_empty());
        assert_eq!(
            tail.take_diagnostics(),
            Vec::new(),
            "the same fact repeated while the file stayed unopenable"
        );

        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .expect("restore the permission");
        assert!(
            lines(&mut tail).is_empty(),
            "history of a file present at the first scan poured out"
        );
        append(&path, "after\n");
        assert_eq!(lines(&mut tail), vec!["after".to_owned()]);
        assert_eq!(tail.take_diagnostics(), Vec::new());
    }

    #[test]
    fn a_line_that_is_not_utf8_names_its_offset_and_the_next_poll_still_reads() {
        let root = Root::new();
        let path = root.file("j/session.jsonl");
        append(&path, "head\n");
        let mut tail = FileTail::new(root.glob(), root.roots());
        lines(&mut tail);

        std::fs::OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("open")
            .write_all(b"\xff\xfe\nnext\n")
            .expect("write");
        assert_eq!(lines(&mut tail), vec!["next".to_owned()]);
        assert_eq!(
            tail.take_diagnostics(),
            vec![TailDiagnostic::NotUtf8 {
                path: path.clone(),
                offset: 5,
            }]
        );
        append(&path, "later\n");
        assert_eq!(lines(&mut tail), vec!["later".to_owned()]);
    }

    #[test]
    fn a_rename_rotation_drains_the_old_file_then_reads_the_new_one_from_zero() {
        for single_file in [false, true] {
            let root = Root::new();
            let path = root.file("app.log");
            append(&path, "a\n");
            let glob = if single_file {
                Glob::parse(path.to_str().expect("UTF-8 path")).expect("opens")
            } else {
                Glob::parse(&format!("{}/*.log", root.0.display())).expect("opens")
            };
            let mut tail = FileTail::new(glob, root.roots());
            assert!(tail.poll().is_empty());
            let old = identity(&path);

            append(&path, "b\n");
            let rotated = root.0.join("app.log.1");
            std::fs::rename(&path, &rotated).expect("move aside");
            append(&path, "c\n");
            let new = identity(&path);
            assert_ne!(
                old, new,
                "after rotation the path points at a different file"
            );

            let produced = tail.poll();
            assert_eq!(
                produced
                    .iter()
                    .map(|line| (line.line.as_str(), line.file, line.offset))
                    .collect::<Vec<_>>(),
                vec![("b", old, 2), ("c", new, 0)],
                "single_file={single_file}"
            );
            append(&path, "d\n");
            assert_eq!(lines(&mut tail), vec!["d".to_owned()]);
            assert_eq!(tail.take_diagnostics(), Vec::new());
            assert_eq!(tail.watched(), 1, "did not let go of the old file");
        }
    }

    #[test]
    fn a_single_file_removed_and_recreated_is_followed_with_one_root_diagnostic() {
        let root = Root::new();
        let path = root.file("app.log");
        append(&path, "a\n");
        let mut tail = FileTail::new(
            Glob::parse(path.to_str().expect("UTF-8 path")).expect("opens"),
            root.roots(),
        );
        assert!(tail.poll().is_empty());
        append(&path, "b\n");
        std::fs::remove_file(&path).expect("delete");

        assert_eq!(
            lines(&mut tail),
            vec!["b".to_owned()],
            "missed a line written before the delete"
        );
        assert_eq!(
            tail.take_diagnostics(),
            vec![TailDiagnostic::RootUnreadable { root: path.clone() }]
        );
        assert_eq!(tail.watched(), 0, "keeps holding the deleted file");
        assert!(tail.poll().is_empty());
        assert_eq!(
            tail.take_diagnostics(),
            Vec::new(),
            "the same fact repeated on every scan while the file was missing"
        );

        append(&path, "again\n");
        assert_eq!(lines(&mut tail), vec!["again".to_owned()]);
        assert_eq!(tail.take_diagnostics(), Vec::new());

        std::fs::remove_file(&path).expect("delete");
        assert!(tail.poll().is_empty());
        assert_eq!(
            tail.take_diagnostics(),
            vec![TailDiagnostic::RootUnreadable { root: path }]
        );
    }

    #[test]
    fn a_missing_root_is_named() {
        let glob = Glob::parse("/nonexistent-root-for-a-test/**/*.jsonl").expect("opens");
        let mut tail = FileTail::new(glob, roots_at(Path::new("/nonexistent-root-for-a-test")));
        assert!(tail.poll().is_empty());
        assert_eq!(
            tail.take_diagnostics(),
            vec![TailDiagnostic::RootUnreadable {
                root: PathBuf::from("/nonexistent-root-for-a-test"),
            }]
        );
    }

    #[test]
    fn unpublished_pattern_syntax_is_refused() {
        assert_eq!(
            Glob::parse("relative/**/*.jsonl"),
            Err(GlobRejection::NotAbsolute)
        );
        assert_eq!(
            Glob::parse("/a/?/x.jsonl"),
            Err(GlobRejection::UnpublishedSyntax { character: '?' })
        );
        assert_eq!(
            Glob::parse("/a/[ab]/x.jsonl"),
            Err(GlobRejection::UnpublishedSyntax { character: '[' })
        );
        assert_eq!(
            Glob::parse("/a/x**/y.jsonl"),
            Err(GlobRejection::MixedGlobstar)
        );
    }

    #[test]
    fn a_globstar_spans_zero_or_more_segments() {
        let glob = Glob::parse("/root/**/*.jsonl").expect("opens");
        assert!(glob.matches(Path::new("/root/a.jsonl")), "zero fields");
        assert!(glob.matches(Path::new("/root/x/a.jsonl")));
        assert!(glob.matches(Path::new("/root/x/y/z/a.jsonl")));
        assert!(!glob.matches(Path::new("/root/a.txt")));
        assert!(!glob.matches(Path::new("/other/a.jsonl")));
    }
}

/// Existing raw transcript field names, owned by the adapter.
pub const TRANSCRIPT_LINE_BODY: &str = "body";
pub const TRANSCRIPT_LINE_PATH: &str = "path";
pub const TRANSCRIPT_LINE_OFFSET: &str = "offset";

pub fn transcript_payload(line: &TailedLine) -> Option<circular_actors::ProductPayload> {
    use circular_actors::{BaseShape, Name};
    use circular_actors::{FieldMap, GroundShape, Shape};
    use circular_core::Value;

    let name = |text: &'static str| Name::from_static(text);

    let fields = FieldMap::try_new(vec![
        (name(TRANSCRIPT_LINE_BODY), Shape::Base(BaseShape::String)),
        (name(TRANSCRIPT_LINE_OFFSET), Shape::Base(BaseShape::Int)),
        (name(TRANSCRIPT_LINE_PATH), Shape::Base(BaseShape::String)),
    ])
    .ok()?;
    let shape = GroundShape::try_new(Shape::Object { fields, open: true }).ok()?;
    let value = Value::object([
        (TRANSCRIPT_LINE_BODY, Value::String(line.line.clone())),
        (
            TRANSCRIPT_LINE_OFFSET,
            Value::Int(i64::try_from(line.offset).unwrap_or(i64::MAX)),
        ),
        (
            TRANSCRIPT_LINE_PATH,
            Value::string(line.path.display().to_string()),
        ),
    ])
    .ok()?;
    Some(circular_actors::ProductPayload::new(shape, value))
}

#[cfg(test)]
mod capture_integration_tests {
    use super::*;
    use std::fs;
    #[test]
    fn replay_preserves_lines_and_offsets() {
        let root = super::tests::Root::new();
        let path = root.file("demo/session.jsonl");
        fs::write(&path, "{\"a\":1}\n{\"a\":2}\n{\"a\":3}\n").unwrap();
        let mut source = FileTail::new(root.glob(), root.roots());
        assert!(source.poll().is_empty());
        assert_eq!(source.watched(), 1);
        assert!(source.poll().is_empty(), "startup history must stay at EOF");
        let file = super::tests::identity(&path);
        let expected = vec![
            TailedLine {
                path: path.clone(),
                file,
                offset: 0,
                line: "{\"a\":1}".into(),
            },
            TailedLine {
                path: path.clone(),
                file,
                offset: 8,
                line: "{\"a\":2}".into(),
            },
            TailedLine {
                path,
                file,
                offset: 16,
                line: "{\"a\":3}".into(),
            },
        ];
        source.replay_from_start();
        assert_eq!(
            source.poll(),
            expected,
            "replay preserves each line and its origin"
        );
        assert!(source.poll().is_empty());
        source.replay_from_start();
        assert_eq!(
            source.poll(),
            expected,
            "same file and offset give the same origin"
        );
    }
}
