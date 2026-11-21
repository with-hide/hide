use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use hide_node_link::cleanup::PathState;
use hide_node_link::document::Document;
use hide_node_link::protocol::{Call, RevisionNow, RootRef};
use hide_node_link::save::Saved;
use hide_node_link::{ErrorCode, RootIdentity};
use hide_platform::path::{self, PathError, RelPath};

use crate::model::EditorDocumentSnapshot;
use crate::node_access::{LinkError, NodeLink, call_as};

/// The checkout roots the daemon opened after its registration check, as the
/// identities their handles reported. Each identity pins the folder every
/// host request names; the daemon's node holds the handles
/// (`hide_node::HeldRoots`), so that identity cannot be reused by another
/// folder while the daemon runs. A client that supplies none has its
/// requests pin the root when they first open it.
type PinnedIdentity = (PathBuf, Option<RootIdentity>);

#[derive(Clone, Debug, Default)]
pub struct FileRoots {
    identities: Arc<Vec<PinnedIdentity>>,
}

impl PartialEq for FileRoots {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identities, &other.identities)
            || (self.identities.len() == other.identities.len()
                && self.identities.iter().zip(other.identities.iter()).all(
                    |((left_path, left_id), (right_path, right_id))| {
                        left_path == right_path && left_id.is_some() && left_id == right_id
                    },
                ))
    }
}

impl Eq for FileRoots {}

impl FileRoots {
    pub fn from_identities(identities: Vec<PinnedIdentity>) -> Self {
        Self {
            identities: Arc::new(identities),
        }
    }

    /// The registered root that holds `path`, with the identity hided
    /// pinned when it opened it; the deepest one when roots nest.
    pub(crate) fn pinned_root(&self, path: &Path) -> Option<(PathBuf, RootIdentity)> {
        self.identities
            .iter()
            .filter(|(root, _)| path.starts_with(root))
            .max_by_key(|(root, _)| root.components().count())
            .and_then(|(root, identity)| identity.map(|identity| (root.clone(), identity)))
    }
}

/// A checkout as document work reaches it: the device it is on, its root
/// path there, and the root's identity when hided pinned it at
/// registration. Without one, the open reads it and the document keeps it.
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentRoot {
    pub device_id: String,
    pub path: String,
    pub identity: Option<RootIdentity>,
}

/// Where an open document lives. Every save and settle read names this
/// exact root, so a checkout replaced after the open refuses them instead
/// of writing into whatever took its path (PRD S5.5 B7, B13). The channel is
/// looked up by device each time, so a save after a reconnect goes to the
/// new helper connection.
#[derive(Clone, Debug, PartialEq)]
pub struct DocumentPlace {
    pub device_id: String,
    pub root: RootRef,
    pub relative: RelPath,
}

#[derive(Debug)]
pub enum OpenFailure {
    /// The file is not there.
    Missing,
    Failed(String),
}

impl OpenFailure {
    pub fn message(&self) -> String {
        match self {
            Self::Missing => "The file no longer exists".to_owned(),
            Self::Failed(message) => message.clone(),
        }
    }
}

/// What one save came to. `Unknown` is the only answer that does not say
/// what is on disk: the caller settles it by reading the revision again and
/// never by sending the write a second time (B14).
#[derive(Debug)]
pub enum SaveOutcome {
    Saved(Saved),
    Conflict {
        disk_revision: Option<String>,
        message: String,
    },
    Refused(String),
    Unknown(String),
}

const OPEN_TIMEOUT: Duration = Duration::from_secs(30);
const SAVE_TIMEOUT: Duration = Duration::from_secs(60);
const REVISION_TIMEOUT: Duration = Duration::from_secs(30);

/// `absolute` below `root`, both one device's paths in the wire spelling
/// (`hide_platform::path`), as the host protocol names it. By names alone,
/// so it holds for a device on any system; the root itself is not a file.
pub fn relative_under(root: &str, absolute: &str) -> Result<RelPath, String> {
    path::wire_relative(root, absolute)
        .ok()
        .filter(|relative| !relative.is_root())
        .ok_or_else(|| "The file is not inside its checkout".to_owned())
}

/// The host's checkout-relative path rule. A shell may spell a file through
/// a link to the checkout's folder (`/var` for `/private/var`) or in another
/// case on a volume that ignores it, so a path the names alone do not place
/// is placed again with its folder and the root as the node resolves them:
/// the folder holding the file is resolved, never the file itself, so a link
/// inside the checkout is still opened as the link.
pub fn relative_in_root(
    channel: &dyn NodeLink,
    root: &str,
    absolute: &str,
) -> Result<RelPath, String> {
    relative_under(root, absolute).or_else(|refused| {
        let Some((folder, name)) = absolute
            .rsplit_once('/')
            .filter(|(folder, name)| !folder.is_empty() && !name.is_empty())
        else {
            return Err(refused);
        };
        let resolved = call_as::<Vec<PathState>>(
            channel,
            Call::RealPaths {
                paths: vec![folder.to_owned(), root.to_owned()],
            },
            REVISION_TIMEOUT,
        );
        match resolved.as_deref() {
            Ok(
                [
                    PathState::Real { path: folder },
                    PathState::Real { path: root },
                ],
            ) => {
                let folder = path::to_wire_lossy(Path::new(folder));
                let root = path::to_wire_lossy(Path::new(root));
                relative_under(&root, &format!("{folder}/{name}")).map_err(|_| refused)
            }
            _ => Err(refused),
        }
    })
}

/// Opens the document at `absolute` in `root`, as a snapshot and the place
/// its saves go to. Blocks on the channel.
pub fn open_document(
    channel: &dyn NodeLink,
    root: &DocumentRoot,
    absolute: &str,
) -> Result<(EditorDocumentSnapshot, DocumentPlace), OpenFailure> {
    let relative = relative_in_root(channel, &root.path, absolute).map_err(OpenFailure::Failed)?;
    let place = DocumentPlace {
        device_id: root.device_id.clone(),
        root: root_ref(channel, root).map_err(open_failure)?,
        relative,
    };
    let document: Result<Document, _> = call_as(
        channel,
        Call::OpenDocument {
            root: place.root.clone(),
            path: place.relative.to_string(),
        },
        OPEN_TIMEOUT,
    );
    // An open is an explicit read, like a listing: a replaced root is
    // refused once and unpinned, so the operator's next open adopts the
    // folder now at that path.
    if let Err(LinkError::Refused(error)) = &document
        && error.code == ErrorCode::RootReplaced
        && root.identity.is_none()
    {
        channel.pin(&root.path, None);
    }
    let document = document.map_err(open_failure)?;
    Ok((document_snapshot(absolute, document), place))
}

fn open_failure(error: LinkError) -> OpenFailure {
    match error {
        LinkError::Refused(error) if error.code == ErrorCode::NotFound => OpenFailure::Missing,
        LinkError::Refused(error) => {
            OpenFailure::Failed(format!("The file could not be opened: {}", error.message))
        }
        other => OpenFailure::Failed(format!("The file could not be opened: {other}")),
    }
}

fn document_snapshot(absolute: &str, document: Document) -> EditorDocumentSnapshot {
    EditorDocumentSnapshot {
        path: absolute.to_owned(),
        language: document.language,
        document_kind: document.kind,
        contents_utf8: document.contents,
        revision: document.revision,
        dirty: false,
        readonly_reason: document.readonly_reason,
        conflict: None,
        save: None,
    }
}

pub fn update_draft(editor: &mut EditorDocumentSnapshot, contents: String) -> Result<(), String> {
    check_editable(editor, "the draft was not changed")?;
    editor.dirty = editor.contents_utf8.as_deref() != Some(contents.as_str());
    editor.contents_utf8 = Some(contents);
    Ok(())
}

/// Refuses a document that takes no draft; `consequence` finishes the
/// sentence the operator reads.
pub fn check_editable(editor: &EditorDocumentSnapshot, consequence: &str) -> Result<(), String> {
    if !editor.document_kind.is_editable() {
        return Err(format!(
            "The current file is not a text document; {consequence}"
        ));
    }
    if editor.readonly_reason.is_some() {
        return Err(format!("The current file is read-only; {consequence}"));
    }
    if editor.revision.is_none() {
        return Err(format!(
            "The file's revision was never read, so a save cannot be checked; {consequence}"
        ));
    }
    Ok(())
}

/// Saves `contents` at `place` if the file there still holds `expected`.
/// Blocks on the channel.
pub fn save_document(
    channel: &dyn NodeLink,
    place: &DocumentPlace,
    contents: &str,
    expected: &str,
) -> SaveOutcome {
    match call_as::<Saved>(
        channel,
        Call::Save {
            root: place.root.clone(),
            path: place.relative.to_string(),
            contents: contents.to_owned(),
            expected_revision: expected.to_owned(),
        },
        SAVE_TIMEOUT,
    ) {
        Ok(saved) => SaveOutcome::Saved(saved),
        Err(LinkError::Refused(error)) if error.code == ErrorCode::Conflict => {
            SaveOutcome::Conflict {
                disk_revision: error.actual_revision,
                message: error.message,
            }
        }
        Err(LinkError::Refused(error)) => SaveOutcome::Refused(error.message),
        Err(LinkError::NotConnected(reason)) => SaveOutcome::Refused(format!(
            "{reason}; nothing was sent and the draft was preserved"
        )),
        Err(error @ LinkError::Busy) => SaveOutcome::Refused(error.to_string()),
        Err(LinkError::Unknown(reason)) => SaveOutcome::Unknown(reason),
    }
}

/// The file's revision now, or `None` when it is gone; read after any save
/// in its folder has finished. Blocks on the channel.
pub fn revision_now(
    channel: &dyn NodeLink,
    place: &DocumentPlace,
) -> Result<Option<String>, LinkError> {
    match call_as::<RevisionNow>(
        channel,
        Call::Revision {
            root: place.root.clone(),
            path: place.relative.to_string(),
        },
        REVISION_TIMEOUT,
    ) {
        Ok(now) => Ok(Some(now.revision)),
        Err(LinkError::Refused(error)) if error.code == ErrorCode::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// One change the explorer asks the filesystem for.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExplorerOperationKind {
    FileCreate,
    DirCreate,
    PathRename,
    PathMove,
    /// An item copied into a folder of the same checkout, whole; the host
    /// names the copy.
    PathCopy,
    /// Files and folders dropped from the operator's file manager, written
    /// into a folder; the host names each top-level item.
    PathImport,
    /// The item goes to the platform Trash. There is no permanent delete;
    /// a filesystem with no Trash refuses the move and the item stays.
    PathTrash,
}

impl ExplorerOperationKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::FileCreate => "file_create",
            Self::DirCreate => "dir_create",
            Self::PathRename => "path_rename",
            Self::PathMove => "path_move",
            Self::PathCopy => "path_copy",
            Self::PathImport => "path_import",
            Self::PathTrash => "path_trash",
        }
    }
}

/// An explorer change with both of its paths already decided.
///
/// Deciding is separate from doing because the decision runs under the
/// runtime mutex, where it may read nothing from disk, and the filesystem
/// call runs on a worker with the lock released. Everything the runtime
/// refuses - a path outside the root, a name with a separator, a folder
/// moved into itself - is refused here from the paths alone. The paths are
/// the checkout device's, in the wire spelling, and are related by their
/// names alone, so a device on any system is judged the same way.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplorerOperation {
    pub kind: ExplorerOperationKind,
    /// The item the change starts from: the path a new item takes, or the
    /// current path of the item being renamed or moved.
    pub source: String,
    /// Where the item is once the change has landed. Equal to `source` for
    /// a creation, and for a trash, whose item has no path here afterwards.
    pub destination: String,
    /// The row the tree selects once the change has landed: the item itself
    /// for a creation, rename or move, and the tree's chosen neighbour for
    /// a trash, whose item is no longer there to select.
    pub selection: String,
    /// The inode the operator was shown when a trash was confirmed, when the
    /// shell could read one. The call refuses an item with another inode:
    /// what leaves is what the modal named, not whatever replaced it at that
    /// path while the modal was open. `None` leaves only the existence
    /// check, which is all a rename or move has ever had.
    pub expected_inode: Option<u64>,
    /// What a drop from the file manager carries, in the order the page listed
    /// it. Empty for every other change.
    pub import: Vec<ImportItem>,
}

/// One file or folder of a drop, below the folder it lands in. The page's
/// bytes wait in hided's staging until the worker reads them (`staged`),
/// off the runtime lock, into `data`.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImportItem {
    /// The wire spelling of a path below the destination folder.
    pub path: String,
    pub directory: bool,
    /// The staged file holding a file's bytes, absolute on the core's machine.
    pub staged: Option<String>,
    /// The bytes, base64 encoded, once read.
    pub data: String,
}

/// What a finished change tells the core beyond success: where the new item
/// ended up, when the host chose its name.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Applied {
    /// The path of the first new item, in the checkout device's spelling.
    pub destination: Option<String>,
}

/// An inode on the shell's wire, as decimal text. A browser reads a JSON
/// number as a double, which rounds a 64-bit id past 2^53 (an NTFS file id
/// once its sequence number reaches 32), and a trash that sent the rounded
/// id back would be refused as an item that changed while its prompt was
/// open. `contracts/hided-ws.schema.json` names the field a string.
pub mod inode_text {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(inode: &Option<u64>, serializer: S) -> Result<S::Ok, S::Error> {
        match inode {
            Some(inode) => serializer.serialize_str(&inode.to_string()),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<u64>, D::Error> {
        Option::<String>::deserialize(deserializer)?
            .map(|text| text.parse().map_err(serde::de::Error::custom))
            .transpose()
    }
}

/// Why an Explorer change was not made: the code the tree words
/// (`explorer.*`, or `host.*` and `link.*` from the checkout's host,
/// `node_access::link_failure_code`) with the one limit its sentence names,
/// and the sentence the log keeps.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExplorerFailure {
    pub failure: crate::model::CodedFailure,
    pub detail: String,
}

impl ExplorerFailure {
    pub fn new(code: &str, detail: impl Into<String>) -> Self {
        Self {
            failure: crate::model::CodedFailure::new(code),
            detail: detail.into(),
        }
    }

    pub fn code(&self) -> &str {
        &self.failure.code
    }
}

/// A path or name the change was planned from that is not the checkout's:
/// the screen says only that nothing changed.
impl From<String> for ExplorerFailure {
    fn from(detail: String) -> Self {
        Self::new("explorer.invalid", detail)
    }
}

impl std::fmt::Display for ExplorerFailure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.detail)
    }
}

impl ExplorerOperation {
    pub fn create(
        kind: ExplorerOperationKind,
        root: &str,
        parent: &str,
        name: &str,
    ) -> Result<Self, ExplorerFailure> {
        if !matches!(
            kind,
            ExplorerOperationKind::FileCreate | ExplorerOperationKind::DirCreate
        ) {
            return Err(format!("{} does not create an item", kind.as_str()).into());
        }
        let parent = path_inside_root(root, parent, true)?;
        let path = path::wire_join(root, &item_below(&parent, name)?);
        Ok(Self {
            kind,
            source: path.clone(),
            destination: path.clone(),
            selection: path,
            expected_inode: None,
            import: Vec::new(),
        })
    }

    pub fn rename(root: &str, path: &str, name: &str) -> Result<Self, ExplorerFailure> {
        let source = path_inside_root(root, path, false)?;
        let parent = source
            .parent()
            .ok_or_else(|| "The item has no parent folder".to_owned())?;
        let destination = item_below(&parent, name)?;
        if destination == source {
            return Err(ExplorerFailure::new(
                "explorer.name_unchanged",
                "the name is unchanged",
            ));
        }
        let destination = path::wire_join(root, &destination);
        Ok(Self {
            kind: ExplorerOperationKind::PathRename,
            source: path::wire_join(root, &source),
            destination: destination.clone(),
            selection: destination,
            expected_inode: None,
            import: Vec::new(),
        })
    }

    pub fn move_into(
        root: &str,
        path: &str,
        destination_dir: &str,
    ) -> Result<Self, ExplorerFailure> {
        let source = path_inside_root(root, path, false)?;
        let destination_dir = path_inside_root(root, destination_dir, true)?;
        let name = source
            .file_name()
            .ok_or_else(|| "The item has no name".to_owned())?;
        if destination_dir.starts_with(&source) {
            return Err(ExplorerFailure::new(
                "explorer.move_into_itself",
                "a folder cannot be moved into itself",
            ));
        }
        let destination = item_below(&destination_dir, name)?;
        if destination == source {
            return Err(ExplorerFailure::new(
                "explorer.already_there",
                "the item is already in that folder",
            ));
        }
        let destination = path::wire_join(root, &destination);
        Ok(Self {
            kind: ExplorerOperationKind::PathMove,
            source: path::wire_join(root, &source),
            destination: destination.clone(),
            selection: destination,
            expected_inode: None,
            import: Vec::new(),
        })
    }

    /// `path` copied into the folder `destination_dir` of the same checkout.
    /// The host chooses the copy's name when the item's own is taken, so the
    /// destination here is the name it would take if free; the host's answer
    /// replaces it (`Applied`). A folder is not copied into itself.
    pub fn copy_into(
        root: &str,
        path: &str,
        destination_dir: &str,
    ) -> Result<Self, ExplorerFailure> {
        let source = path_inside_root(root, path, false)?;
        let destination_dir = path_inside_root(root, destination_dir, true)?;
        let name = source
            .file_name()
            .ok_or_else(|| "The item has no name".to_owned())?;
        if destination_dir.starts_with(&source) {
            return Err(ExplorerFailure::new(
                "explorer.copy_into_itself",
                "a folder cannot be copied into itself",
            ));
        }
        let destination = path::wire_join(root, &item_below(&destination_dir, name)?);
        Ok(Self {
            kind: ExplorerOperationKind::PathCopy,
            source: path::wire_join(root, &source),
            destination: destination.clone(),
            selection: destination,
            expected_inode: None,
            import: Vec::new(),
        })
    }

    /// A drop's items written into the folder `destination_dir`. Refused from
    /// the strings alone when it holds nothing, more than the cap, or a path
    /// that is not a name below the folder; the host judges the rest.
    pub fn import_into(
        root: &str,
        destination_dir: &str,
        items: Vec<ImportItem>,
    ) -> Result<Self, ExplorerFailure> {
        let folder = path_inside_root(root, destination_dir, true)?;
        if items.is_empty() {
            return Err(ExplorerFailure::new(
                "explorer.drop_empty",
                "the drop holds nothing",
            ));
        }
        if items.len() > hide_node_link::mutate::IMPORT_MAX_ITEMS {
            let limit = hide_node_link::mutate::IMPORT_MAX_ITEMS;
            let mut failure = ExplorerFailure::new(
                "explorer.drop_limit",
                format!("a drop may hold {limit} items; nothing was added"),
            );
            failure.failure.limit = Some(limit as u64);
            return Err(failure);
        }
        for item in &items {
            RelPath::parse(&item.path)
                .ok()
                .filter(|path| !path.is_root())
                .ok_or_else(|| format!("{} is not a path below the folder", item.path))?;
        }
        let destination = path::wire_join(root, &folder);
        Ok(Self {
            kind: ExplorerOperationKind::PathImport,
            source: destination.clone(),
            destination: destination.clone(),
            selection: destination,
            expected_inode: None,
            import: items,
        })
    }

    /// The item leaves the tree for the Trash and `select_after` takes its
    /// place as the selection. Refused from the strings alone like every
    /// other change: the root itself and anything outside it never reach
    /// the call, and a selection that would leave with the item is refused
    /// rather than pointed at nothing.
    pub fn trash(
        root: &str,
        path: &str,
        select_after: &str,
        expected_inode: Option<u64>,
    ) -> Result<Self, ExplorerFailure> {
        let source = path_inside_root(root, path, false)?;
        let selection = path_inside_root(root, select_after, true)?;
        if selection.starts_with(&source) {
            return Err(
                "The selection cannot move into the item being moved to the Trash"
                    .to_owned()
                    .into(),
            );
        }
        let source = path::wire_join(root, &source);
        Ok(Self {
            kind: ExplorerOperationKind::PathTrash,
            source: source.clone(),
            destination: source,
            selection: path::wire_join(root, &selection),
            expected_inode,
            import: Vec::new(),
        })
    }
}

const CHANGE_TIMEOUT: Duration = Duration::from_secs(60);

/// `absolute` below `root` as the host protocol spells it, the root itself
/// as the empty path.
fn relative_or_root(root: &str, absolute: &str) -> Result<String, String> {
    path::wire_relative(root, absolute)
        .map(RelPath::into_string)
        .map_err(|_| "The file is not inside its checkout".to_owned())
}

/// The root as the checkout's host requests name it: the identity hided
/// pinned, or the one the channel pinned when it first touched the root, so
/// a device folder replaced after it was listed is refused rather than
/// opened or changed in its place.
pub(crate) fn root_ref(channel: &dyn NodeLink, root: &DocumentRoot) -> Result<RootRef, LinkError> {
    match root.identity {
        Some(identity) => Ok(RootRef {
            path: root.path.clone(),
            identity,
        }),
        None => crate::node_access::pinned_root(channel, &root.path, OPEN_TIMEOUT),
    }
}

/// Runs the change on the machine that holds the checkout, through the same
/// `hide_host` request on this machine as on a device: the host confines it
/// to the opened root, never replaces an existing item, and moves an item to
/// its own Trash or leaves it (PRD S5.5 B16-B18). Blocks on the channel; a
/// request whose answer was lost is reported as an unknown result, which the
/// tree settles by reading the folder again.
pub fn apply_explorer_operation(
    channel: &dyn NodeLink,
    root: &DocumentRoot,
    operation: &ExplorerOperation,
) -> Result<Applied, ExplorerFailure> {
    let root_ref = root_ref(channel, root).map_err(|error| change_failure(operation, error))?;
    let source = relative_or_root(&root.path, &operation.source)?;
    let destination = RelPath::parse(&relative_or_root(&root.path, &operation.destination)?)
        .map_err(|error| error.to_string())?;
    let call = match operation.kind {
        ExplorerOperationKind::FileCreate | ExplorerOperationKind::DirCreate => {
            let created = RelPath::parse(&source).map_err(|error| error.to_string())?;
            Call::Create {
                root: root_ref,
                parent: created
                    .parent()
                    .ok_or_else(|| "The item has no parent folder".to_owned())?
                    .into_string(),
                name: file_name(&created)?,
                directory: operation.kind == ExplorerOperationKind::DirCreate,
            }
        }
        ExplorerOperationKind::PathRename => Call::Rename {
            root: root_ref,
            path: source,
            name: file_name(&destination)?,
        },
        ExplorerOperationKind::PathMove => Call::Move {
            root: root_ref,
            path: source,
            destination: destination
                .parent()
                .ok_or_else(|| "The destination has no folder".to_owned())?
                .into_string(),
        },
        ExplorerOperationKind::PathCopy => {
            let folder = destination
                .parent()
                .ok_or_else(|| "The destination has no folder".to_owned())?;
            let copied = call_as::<hide_node_link::mutate::Copied>(
                channel,
                Call::Copy {
                    root: root_ref,
                    path: source,
                    destination: folder.as_str().to_owned(),
                },
                CHANGE_TIMEOUT,
            )
            .map_err(|error| change_failure(operation, error))?;
            let named = folder
                .join(&copied.name)
                .map_err(|error| error.to_string())?;
            return Ok(Applied {
                destination: Some(path::wire_join(&root.path, &named)),
            });
        }
        ExplorerOperationKind::PathImport => {
            let entries = operation
                .import
                .iter()
                .map(|item| hide_node_link::mutate::ImportEntry {
                    path: item.path.clone(),
                    directory: item.directory,
                    data: item.data.clone(),
                })
                .collect();
            let imported = call_as::<hide_node_link::mutate::Imported>(
                channel,
                Call::Import {
                    root: root_ref,
                    destination: source,
                    entries,
                },
                CHANGE_TIMEOUT,
            )
            .map_err(|error| change_failure(operation, error))?;
            return Ok(Applied {
                destination: imported.names.first().map(|name| {
                    path::wire_join(
                        &operation.destination,
                        &RelPath::root().join(name).expect("a name the host made"),
                    )
                }),
            });
        }
        ExplorerOperationKind::PathTrash => Call::Trash {
            root: root_ref,
            path: source,
            inode: operation.expected_inode,
        },
    };
    call_as::<hide_node_link::mutate::Changed>(channel, call, CHANGE_TIMEOUT)
        .map(|_| Applied::default())
        .map_err(|error| change_failure(operation, error))
}

/// Reads the staged bytes of a drop on the core's own node, the machine that
/// holds hided's staging, into the operation's items. A staged file that is
/// gone or changed is a failure of the whole drop: nothing has been written
/// yet.
pub fn load_dropped(
    own_node: &dyn NodeLink,
    operation: &mut ExplorerOperation,
) -> Result<(), ExplorerFailure> {
    let paths: Vec<String> = operation
        .import
        .iter()
        .filter_map(|item| item.staged.clone())
        .collect();
    if paths.is_empty() {
        return Ok(());
    }
    let read: Vec<hide_node_link::attachments::ReadFile> =
        crate::node_access::call_as(own_node, Call::ReadDropped { paths }, DROP_READ_TIMEOUT)
            .map_err(|error| {
                ExplorerFailure::new(
                    "explorer.drop_unreadable",
                    format!("the dropped files could not be read: {error}; nothing was added"),
                )
            })?;
    let mut read = read.into_iter();
    for item in &mut operation.import {
        if item.staged.is_none() {
            continue;
        }
        let file = read.next().ok_or_else(|| {
            ExplorerFailure::new(
                "explorer.drop_unreadable",
                "a dropped file was not read; nothing was added",
            )
        })?;
        item.data = file.data;
    }
    Ok(())
}

/// Reading 40 MiB of staged files from a local disk takes well under a minute.
const DROP_READ_TIMEOUT: Duration = Duration::from_secs(120);

/// [`apply_explorer_operation`] after a drop's bytes are read: the one call a
/// worker makes for any change.
pub fn run_explorer_operation(
    channel: &dyn NodeLink,
    own_node: Option<&dyn NodeLink>,
    root: &DocumentRoot,
    operation: &mut ExplorerOperation,
) -> Result<Applied, ExplorerFailure> {
    if operation.kind == ExplorerOperationKind::PathImport {
        let own_node = own_node.ok_or_else(|| {
            ExplorerFailure::new(
                "explorer.drop_unreadable",
                "this machine's files could not be reached; nothing was added",
            )
        })?;
        load_dropped(own_node, operation)?;
    }
    apply_explorer_operation(channel, root, operation)
}

fn file_name(path: &RelPath) -> Result<String, String> {
    path.file_name()
        .map(str::to_owned)
        .ok_or_else(|| "The checkout root has no name".to_owned())
}

fn change_failure(operation: &ExplorerOperation, error: LinkError) -> ExplorerFailure {
    let code = crate::node_access::link_failure_code(&error);
    let detail = match error {
        LinkError::Refused(error) => error.message,
        LinkError::NotConnected(reason) => format!("{reason}; nothing was changed"),
        error @ LinkError::Busy => error.to_string(),
        LinkError::Unknown(reason) => format!(
            "{reason}; whether {} changed is unknown, so the folder is read again",
            operation.source
        ),
    };
    ExplorerFailure::new(&code, detail)
}

/// `path` below `root`, both wire spellings of the checkout device's paths,
/// when it is absolute, normal, and inside `root`. By names alone, so
/// `/repo-other` is outside `/repo`, and lexical, so a symlink that escapes
/// the root is not followed here and cannot be created here either.
pub(crate) fn path_inside_root(
    root: &str,
    path: &str,
    allow_root: bool,
) -> Result<RelPath, String> {
    if !path::is_wire_absolute(root) {
        return Err("The workspace root is not an absolute path".to_owned());
    }
    if !path::is_wire_absolute(path) {
        return Err(format!("{path} is not an absolute path"));
    }
    let relative = path::wire_relative(root, path).map_err(|error| match error {
        PathError::NotNormal => format!("{path} is not a normal path"),
        _ => format!("{path} is outside the workspace {root}"),
    })?;
    if !allow_root && relative.is_root() {
        return Err("The workspace root itself cannot be changed".to_owned());
    }
    Ok(relative)
}

/// `name` as one item below `parent`. The wire's own rule: one name, no
/// `/`, no NUL, not `.` or `..`; whether the device's system can hold the
/// name is the device's host's to judge when it makes the item.
fn item_below(parent: &RelPath, name: &str) -> Result<RelPath, ExplorerFailure> {
    if name.is_empty() {
        return Err(ExplorerFailure::new(
            "explorer.name_required",
            "a name is required",
        ));
    }
    if name.contains('/') {
        return Err(ExplorerFailure::new(
            "explorer.name_slash",
            "a name cannot contain /",
        ));
    }
    if name.contains('\0') {
        return Err(ExplorerFailure::new(
            "explorer.name_invalid",
            "a name cannot contain NUL",
        ));
    }
    parent.join(name).map_err(|_| {
        ExplorerFailure::new(
            "explorer.name_invalid",
            format!("{name} is not a valid name"),
        )
    })
}

#[cfg(test)]
mod inode_text_tests {
    #[derive(serde::Deserialize, serde::Serialize)]
    struct Row {
        #[serde(default, with = "super::inode_text")]
        inode: Option<u64>,
    }

    #[test]
    fn an_inode_past_what_a_browser_number_holds_round_trips_as_text() {
        let row = Row {
            inode: Some((1_u64 << 53) + 1),
        };
        let text = serde_json::to_string(&row).unwrap();
        assert_eq!(text, r#"{"inode":"9007199254740993"}"#);
        let back: Row = serde_json::from_str(&text).unwrap();
        assert_eq!(back.inode, Some((1_u64 << 53) + 1));
    }

    #[test]
    fn an_inode_sent_as_a_number_or_absent_is_not_taken_for_one() {
        assert!(serde_json::from_str::<Row>(r#"{"inode":9007199254740993}"#).is_err());
        assert!(serde_json::from_str::<Row>(r#"{"inode":"12a"}"#).is_err());
        assert_eq!(serde_json::from_str::<Row>("{}").unwrap().inode, None);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::model::DocumentKind;
    use std::fs;
    #[cfg(unix)]
    use std::fs::File;
    use std::time::UNIX_EPOCH;

    #[test]
    fn draft_updates_keep_unsaved_contents_in_memory() {
        let mut editor = EditorDocumentSnapshot {
            path: "/tmp/existing.txt".to_owned(),
            language: Some("txt".to_owned()),
            document_kind: DocumentKind::Text,
            contents_utf8: Some("old".to_owned()),
            revision: Some(hide_node_link::document::revision_of(b"old")),
            dirty: false,
            readonly_reason: None,
            conflict: None,
            save: None,
        };
        update_draft(&mut editor, "new".to_owned()).unwrap();
        assert!(editor.dirty);
        assert_eq!(editor.contents_utf8.as_deref(), Some("new"));
    }

    /// Opens `path` on this machine the way the runtime does, with the
    /// parent folder as the checkout root.
    pub(crate) fn open_local(path: &Path) -> (EditorDocumentSnapshot, DocumentPlace) {
        let root = DocumentRoot {
            device_id: crate::node::TEST_NODE.to_owned(),
            path: path.parent().unwrap().to_string_lossy().into_owned(),
            identity: None,
        };
        open_document(
            &hide_node::Local::of_process(),
            &root,
            &path.to_string_lossy(),
        )
        .expect("fixture document")
    }

    /// A checkout whose path is replaced after a document opened refuses
    /// the save: it lands neither in the impostor nor, silently, in the
    /// moved original, and an explorer change is refused the same way.
    #[cfg(unix)]
    #[test]
    fn a_checkout_replaced_after_the_open_refuses_the_save_and_writes_nowhere() {
        use std::os::unix::fs::symlink;
        let sandbox = tempfile::tempdir().unwrap();
        let root = sandbox.path().join("checkout");
        let outside = sandbox.path().join("outside");
        fs::create_dir(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(root.join("note.txt"), "inside").unwrap();
        fs::write(outside.join("note.txt"), "outside").unwrap();
        let roots = FileRoots::from_identities(
            hide_node::hold_roots(vec![(root.clone(), File::open(&root).unwrap())]).1,
        );
        let (pinned_path, identity) = roots.pinned_root(&root.join("note.txt")).unwrap();
        let document_root = DocumentRoot {
            device_id: "local".to_owned(),
            path: pinned_path.to_string_lossy().into_owned(),
            identity: Some(identity),
        };
        let channel = hide_node::Local::of_process();
        let (document, place) = open_document(
            &channel,
            &document_root,
            &root.join("note.txt").to_string_lossy(),
        )
        .unwrap();
        assert_eq!(document.contents_utf8.as_deref(), Some("inside"));
        let wire_root = root.to_string_lossy();
        let create = ExplorerOperation::create(
            ExplorerOperationKind::FileCreate,
            &wire_root,
            &wire_root,
            "created.txt",
        )
        .unwrap();
        fs::rename(&root, sandbox.path().join("moved")).unwrap();
        symlink(&outside, &root).unwrap();
        match save_document(
            &channel,
            &place,
            "edited",
            document.revision.as_deref().unwrap(),
        ) {
            SaveOutcome::Refused(message) => assert!(message.contains("replaced"), "{message}"),
            other => panic!("a replaced checkout must refuse the save: {other:?}"),
        }
        let refused = apply_explorer_operation(&channel, &document_root, &create).unwrap_err();
        assert!(refused.detail.contains("replaced"), "{refused}");
        assert_eq!(
            fs::read_to_string(sandbox.path().join("moved/note.txt")).unwrap(),
            "inside"
        );
        assert!(!sandbox.path().join("moved/created.txt").exists());
        assert!(!outside.join("created.txt").exists());
        assert_eq!(
            fs::read_to_string(outside.join("note.txt")).unwrap(),
            "outside"
        );
    }

    #[test]
    fn a_path_outside_the_checkout_is_refused_before_anything_is_sent() {
        for (root, path) in [
            ("/repo", "/repository/a.txt"),
            ("/repo", "/repo"),
            ("/repo", "/repo/../etc/passwd"),
            ("/repo/", "/other/a.txt"),
        ] {
            assert!(relative_under(root, path).is_err(), "{root} {path}");
        }
        assert_eq!(
            relative_under("/repo/", "/repo/src/a.rs").unwrap().as_str(),
            "src/a.rs"
        );
    }

    #[cfg(unix)]
    #[test]
    fn a_local_file_spelled_through_a_link_to_its_checkout_opens_under_that_spelling() {
        use std::os::unix::fs::symlink;
        let sandbox = tempfile::tempdir().unwrap();
        let root = sandbox.path().canonicalize().unwrap().join("checkout");
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/a.txt"), "a").unwrap();
        symlink(&root, sandbox.path().join("alias")).unwrap();
        let spelled = sandbox.path().join("alias/src/a.txt");
        let root_path = DocumentRoot {
            device_id: crate::node::TEST_NODE.to_owned(),
            path: root.to_string_lossy().into_owned(),
            identity: None,
        };
        let (document, place) = open_document(
            &hide_node::Local::of_process(),
            &root_path,
            &spelled.to_string_lossy(),
        )
        .unwrap();
        assert_eq!(document.path, spelled.to_string_lossy());
        assert_eq!(place.relative.as_str(), "src/a.txt");
        let outside = sandbox.path().join("alias/../elsewhere.txt");
        fs::write(sandbox.path().join("elsewhere.txt"), "b").unwrap();
        assert!(
            open_document(
                &hide_node::Local::of_process(),
                &root_path,
                &outside.to_string_lossy(),
            )
            .is_err()
        );
    }

    /// Names no other Trash entry can carry, so a trashed fixture can be
    /// found and removed by exact name without touching anything else.
    pub(crate) fn unique_trash_names() -> (String, String) {
        let stamp = format!(
            "{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        (
            format!("hide-test-trash-file-{stamp}.txt"),
            format!("hide-test-trash-folder-{stamp}"),
        )
    }

    /// Removes the named fixtures from `~/.Trash`; a name that is not there
    /// is left alone. `HOME` is read because that is where macOS keeps the
    /// account Trash; the move itself ignores the variable, so a test cannot
    /// redirect it and can only clean up after it.
    pub(crate) fn remove_from_trash(names: &[&str]) {
        let Some(home) = std::env::var_os("HOME") else {
            return;
        };
        let trash = Path::new(&home).join(".Trash");
        for name in names {
            let entry = trash.join(name);
            match fs::symlink_metadata(&entry) {
                Ok(metadata) if metadata.is_dir() => {
                    fs::remove_dir_all(&entry).ok();
                }
                Ok(_) => {
                    fs::remove_file(&entry).ok();
                }
                Err(_) => {}
            }
        }
    }

    /// Records each request and answers every one, so a test reads exactly
    /// what a checkout's host is asked to do.
    #[derive(Default)]
    struct RecordingHost {
        calls: std::sync::Mutex<Vec<Call>>,
    }

    impl NodeLink for RecordingHost {
        fn call(
            &self,
            call: Call,
            _timeout: Duration,
        ) -> Result<crate::node_access::LinkAnswer, LinkError> {
            self.calls.lock().unwrap().push(call);
            Ok(serde_json::json!({}).into())
        }
    }

    #[test]
    fn a_change_asks_the_checkout_host_with_paths_under_its_pinned_root() {
        let root = "/repo";
        let identity = RootIdentity {
            device: 1,
            inode: 2,
        };
        let document_root = DocumentRoot {
            device_id: "macbook".to_owned(),
            path: "/repo".to_owned(),
            identity: Some(identity),
        };
        let pinned = RootRef {
            path: "/repo".to_owned(),
            identity,
        };
        let host = RecordingHost::default();
        let operations = [
            ExplorerOperation::create(ExplorerOperationKind::DirCreate, root, root, "new").unwrap(),
            ExplorerOperation::create(ExplorerOperationKind::FileCreate, root, "/repo/src", "a.rs")
                .unwrap(),
            ExplorerOperation::rename(root, "/repo/src/a.rs", "b.rs").unwrap(),
            ExplorerOperation::move_into(root, "/repo/src/b.rs", root).unwrap(),
            ExplorerOperation::trash(root, "/repo/src", root, Some(7)).unwrap(),
        ];
        for operation in &operations {
            apply_explorer_operation(&host, &document_root, operation).unwrap();
        }
        assert_eq!(
            *host.calls.lock().unwrap(),
            vec![
                Call::Create {
                    root: pinned.clone(),
                    parent: String::new(),
                    name: "new".to_owned(),
                    directory: true
                },
                Call::Create {
                    root: pinned.clone(),
                    parent: "src".to_owned(),
                    name: "a.rs".to_owned(),
                    directory: false
                },
                Call::Rename {
                    root: pinned.clone(),
                    path: "src/a.rs".to_owned(),
                    name: "b.rs".to_owned()
                },
                Call::Move {
                    root: pinned.clone(),
                    path: "src/b.rs".to_owned(),
                    destination: String::new()
                },
                Call::Trash {
                    root: pinned,
                    path: "src".to_owned(),
                    inode: Some(7)
                },
            ]
        );
    }

    /// Records each request and answers every one with `answer`, the shape a
    /// copy or a drop gets back from the host that named the new items.
    struct AnsweringHost {
        answer: serde_json::Value,
        calls: std::sync::Mutex<Vec<Call>>,
    }

    impl NodeLink for AnsweringHost {
        fn call(
            &self,
            call: Call,
            _timeout: Duration,
        ) -> Result<crate::node_access::LinkAnswer, LinkError> {
            self.calls.lock().unwrap().push(call);
            Ok(self.answer.clone().into())
        }
    }

    #[test]
    fn a_copy_asks_the_host_for_one_item_and_takes_the_name_the_host_chose() {
        let root = "/repo";
        let identity = RootIdentity {
            device: 1,
            inode: 2,
        };
        let document_root = DocumentRoot {
            device_id: "macbook".to_owned(),
            path: "/repo".to_owned(),
            identity: Some(identity),
        };
        let host = AnsweringHost {
            answer: serde_json::json!({ "name": "a copy.rs" }),
            calls: Default::default(),
        };
        let copy = ExplorerOperation::copy_into(root, "/repo/src/a.rs", "/repo/dest").unwrap();
        assert_eq!(copy.kind, ExplorerOperationKind::PathCopy);
        let applied = apply_explorer_operation(&host, &document_root, &copy).unwrap();
        assert_eq!(applied.destination.as_deref(), Some("/repo/dest/a copy.rs"));
        assert_eq!(
            *host.calls.lock().unwrap(),
            vec![Call::Copy {
                root: RootRef {
                    path: "/repo".to_owned(),
                    identity
                },
                path: "src/a.rs".to_owned(),
                destination: "dest".to_owned()
            }]
        );
    }

    #[test]
    fn a_copy_or_a_drop_outside_the_checkout_or_into_itself_is_refused_before_anything_is_sent() {
        let root = "/repo";
        for (path, destination, expected) in [
            ("/repo-other/a", "/repo", "outside the workspace"),
            ("/repo/a", "/repo-other", "outside the workspace"),
            ("/repo/a", "/repo/../etc", "not a normal path"),
            ("/repo", "/repo/src", "root itself"),
            ("/repo/src", "/repo/src", "into itself"),
            ("/repo/src", "/repo/src/nested", "into itself"),
        ] {
            let refused = ExplorerOperation::copy_into(root, path, destination).unwrap_err();
            assert!(
                refused.detail.contains(expected),
                "{path} -> {destination}: {refused}"
            );
        }
        let item = |path: &str| ImportItem {
            path: path.to_owned(),
            directory: false,
            staged: None,
            data: String::new(),
        };
        assert!(
            ExplorerOperation::import_into(root, "/repo", Vec::new())
                .unwrap_err()
                .detail
                .contains("holds nothing")
        );
        assert!(
            ExplorerOperation::import_into(root, "/repo-other", vec![item("a")])
                .unwrap_err()
                .detail
                .contains("outside the workspace")
        );
        for path in ["", "../escape", "/abs", "a/../b"] {
            assert!(
                ExplorerOperation::import_into(root, "/repo", vec![item(path)]).is_err(),
                "{path:?}"
            );
        }
        let many = (0..=hide_node_link::mutate::IMPORT_MAX_ITEMS)
            .map(|n| item(&format!("f{n}")))
            .collect();
        assert!(
            ExplorerOperation::import_into(root, "/repo", many)
                .unwrap_err()
                .detail
                .contains("nothing was added")
        );
    }

    #[test]
    fn a_drop_sends_the_bytes_to_the_host_and_selects_the_first_name_it_made() {
        let identity = RootIdentity {
            device: 1,
            inode: 2,
        };
        let document_root = DocumentRoot {
            device_id: "macbook".to_owned(),
            path: "/repo".to_owned(),
            identity: Some(identity),
        };
        let host = AnsweringHost {
            answer: serde_json::json!({ "names": ["a copy.txt", "photos"] }),
            calls: Default::default(),
        };
        let drop = ExplorerOperation::import_into(
            "/repo",
            "/repo/src",
            vec![
                ImportItem {
                    path: "a.txt".to_owned(),
                    directory: false,
                    staged: None,
                    data: "aGk=".to_owned(),
                },
                ImportItem {
                    path: "photos".to_owned(),
                    directory: true,
                    staged: None,
                    data: String::new(),
                },
            ],
        )
        .unwrap();
        let applied = apply_explorer_operation(&host, &document_root, &drop).unwrap();
        assert_eq!(applied.destination.as_deref(), Some("/repo/src/a copy.txt"));
        match host.calls.lock().unwrap().as_slice() {
            [
                Call::Import {
                    destination,
                    entries,
                    ..
                },
            ] => {
                assert_eq!(destination, "src");
                assert_eq!(entries.len(), 2);
                assert_eq!(entries[0].data, "aGk=");
                assert!(entries[1].directory);
            }
            other => panic!("one import call was expected: {other:?}"),
        }
    }

    #[test]
    fn explorer_refuses_to_trash_outside_the_root_or_the_root_itself() {
        let root = "/repo";
        let outside = ExplorerOperation::trash(root, "/repo-other/a", root, None);
        assert!(
            outside
                .unwrap_err()
                .detail
                .contains("outside the workspace")
        );
        let escaping = ExplorerOperation::trash(root, "/repo/../etc/passwd", root, None);
        assert!(escaping.unwrap_err().detail.contains("not a normal path"));
        let root_itself = ExplorerOperation::trash(root, root, root, None);
        assert!(root_itself.unwrap_err().detail.contains("root itself"));
        let relative = ExplorerOperation::trash(root, "src/a", root, None);
        assert!(
            relative
                .unwrap_err()
                .detail
                .contains("not an absolute path")
        );

        let selection_outside = ExplorerOperation::trash(root, "/repo/src", "/repo-other", None);
        assert!(
            selection_outside
                .unwrap_err()
                .detail
                .contains("outside the workspace")
        );
        let selection_inside = ExplorerOperation::trash(root, "/repo/src", "/repo/src/a.rs", None);
        assert!(
            selection_inside
                .unwrap_err()
                .detail
                .contains("cannot move into the item")
        );
        let selection_itself = ExplorerOperation::trash(root, "/repo/src", "/repo/src", None);
        assert!(
            selection_itself
                .unwrap_err()
                .detail
                .contains("cannot move into the item")
        );
    }

    #[test]
    fn explorer_refuses_paths_outside_the_root_and_invalid_names() {
        let root = "/repo";
        let outside =
            ExplorerOperation::create(ExplorerOperationKind::FileCreate, root, "/repo-other", "a");
        assert!(
            outside
                .unwrap_err()
                .detail
                .contains("outside the workspace")
        );
        let escaping = ExplorerOperation::rename(root, "/repo/../etc/passwd", "x");
        assert!(escaping.unwrap_err().detail.contains("not a normal path"));
        let root_itself = ExplorerOperation::rename(root, root, "x");
        assert!(root_itself.unwrap_err().detail.contains("root itself"));
        let relative = ExplorerOperation::move_into(root, "src/a", root);
        assert!(
            relative
                .unwrap_err()
                .detail
                .contains("not an absolute path")
        );

        let parent = "/repo/src";
        for (name, expected) in [
            ("", "explorer.name_required"),
            ("a/b", "explorer.name_slash"),
            ("..", "explorer.name_invalid"),
        ] {
            let error =
                ExplorerOperation::create(ExplorerOperationKind::DirCreate, root, parent, name)
                    .unwrap_err();
            assert_eq!(error.code(), expected, "{name:?}: {error}");
        }
    }

    #[test]
    fn explorer_refuses_a_move_that_changes_nothing_or_nests_a_folder_in_itself() {
        let root = "/repo";
        let same_parent = ExplorerOperation::move_into(root, "/repo/src/a.rs", "/repo/src");
        assert_eq!(same_parent.unwrap_err().code(), "explorer.already_there");
        let into_self = ExplorerOperation::move_into(root, "/repo/src", "/repo/src");
        assert_eq!(into_self.unwrap_err().code(), "explorer.move_into_itself");
        let into_child = ExplorerOperation::move_into(root, "/repo/src", "/repo/src/nested");
        assert_eq!(into_child.unwrap_err().code(), "explorer.move_into_itself");
        let unchanged = ExplorerOperation::rename(root, "/repo/src/a.rs", "a.rs");
        assert_eq!(unchanged.unwrap_err().code(), "explorer.name_unchanged");
    }
}
