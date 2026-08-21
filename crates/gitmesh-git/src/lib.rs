//! Git compatibility primitives for GitMesh.
//!
//! This crate starts at the lowest useful layer: canonical Git object bytes and
//! object IDs. Remote-helper and pack support should build on top of this.

use std::collections::BTreeMap;
use std::fmt;
use std::io::{Read, Write};
use std::str::FromStr;

use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use sha1::{Digest, Sha1};
use thiserror::Error;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitObjectKind {
    Blob,
    Tree,
    Commit,
    Tag,
}

impl GitObjectKind {
    pub fn as_bytes(self) -> &'static [u8] {
        match self {
            Self::Blob => b"blob",
            Self::Tree => b"tree",
            Self::Commit => b"commit",
            Self::Tag => b"tag",
        }
    }

    pub fn parse(bytes: &[u8]) -> Result<Self, GitError> {
        match bytes {
            b"blob" => Ok(Self::Blob),
            b"tree" => Ok(Self::Tree),
            b"commit" => Ok(Self::Commit),
            b"tag" => Ok(Self::Tag),
            _ => Err(GitError::UnknownObjectKind),
        }
    }
}

#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct GitSha1Oid([u8; 20]);

impl GitSha1Oid {
    pub fn from_digest(digest: [u8; 20]) -> Self {
        Self(digest)
    }

    pub fn digest(self) -> [u8; 20] {
        self.0
    }

    pub fn hex(self) -> String {
        self.0.iter().map(|byte| format!("{byte:02x}")).collect()
    }
}

impl FromStr for GitSha1Oid {
    type Err = GitError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if value.len() != 40 {
            return Err(GitError::InvalidOid);
        }
        let mut digest = [0_u8; 20];
        for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
            let high = hex_nibble(chunk[0]).ok_or(GitError::InvalidOid)?;
            let low = hex_nibble(chunk[1]).ok_or(GitError::InvalidOid)?;
            digest[index] = (high << 4) | low;
        }
        Ok(Self(digest))
    }
}

impl fmt::Debug for GitSha1Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("GitSha1Oid").field(&self.hex()).finish()
    }
}

impl fmt::Display for GitSha1Oid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.hex())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitObject {
    pub kind: GitObjectKind,
    pub payload: Vec<u8>,
}

impl GitObject {
    pub fn new(kind: GitObjectKind, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            kind,
            payload: payload.into(),
        }
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        canonical_object_bytes(self.kind, &self.payload)
    }

    pub fn sha1_oid(&self) -> GitSha1Oid {
        sha1_oid_for_canonical_bytes(&self.canonical_bytes())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitPackfile {
    pub version: u32,
    pub objects: Vec<GitObject>,
}

impl GitPackfile {
    pub fn new(objects: impl Into<Vec<GitObject>>) -> Self {
        Self {
            version: 2,
            objects: objects.into(),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitCommitLinks {
    pub tree: GitSha1Oid,
    pub parents: Vec<GitSha1Oid>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GitTreeEntryTarget {
    Blob,
    Tree,
    Commit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GitTreeEntry {
    pub mode: Vec<u8>,
    pub name: Vec<u8>,
    pub oid: GitSha1Oid,
    pub target: GitTreeEntryTarget,
}

pub fn parse_commit_links(payload: &[u8]) -> Result<GitCommitLinks, GitError> {
    let header_end = payload
        .windows(2)
        .position(|window| window == b"\n\n")
        .unwrap_or(payload.len());
    let mut tree = None;
    let mut parents = Vec::new();

    for line in payload[..header_end].split(|byte| *byte == b'\n') {
        if let Some(value) = line.strip_prefix(b"tree ") {
            tree = Some(parse_oid_bytes(value)?);
        } else if let Some(value) = line.strip_prefix(b"parent ") {
            parents.push(parse_oid_bytes(value)?);
        }
    }

    Ok(GitCommitLinks {
        tree: tree.ok_or(GitError::MissingCommitTree)?,
        parents,
    })
}

pub fn parse_tree_entries(payload: &[u8]) -> Result<Vec<GitTreeEntry>, GitError> {
    let mut entries = Vec::new();
    let mut cursor = 0;

    while cursor < payload.len() {
        let mode_end = payload[cursor..]
            .iter()
            .position(|byte| *byte == b' ')
            .ok_or(GitError::MalformedTree)?
            + cursor;
        let name_end = payload[mode_end + 1..]
            .iter()
            .position(|byte| *byte == 0)
            .ok_or(GitError::MalformedTree)?
            + mode_end
            + 1;
        let oid_start = name_end + 1;
        let oid_end = oid_start + 20;
        if oid_end > payload.len() || mode_end == cursor || name_end == mode_end + 1 {
            return Err(GitError::MalformedTree);
        }
        let target = tree_entry_target(&payload[cursor..mode_end])?;
        let mut digest = [0_u8; 20];
        digest.copy_from_slice(&payload[oid_start..oid_end]);
        entries.push(GitTreeEntry {
            mode: payload[cursor..mode_end].to_vec(),
            name: payload[mode_end + 1..name_end].to_vec(),
            oid: GitSha1Oid::from_digest(digest),
            target,
        });
        cursor = oid_end;
    }

    Ok(entries)
}

pub fn canonical_object_bytes(kind: GitObjectKind, payload: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(kind.as_bytes());
    bytes.push(b' ');
    bytes.extend_from_slice(payload.len().to_string().as_bytes());
    bytes.push(0);
    bytes.extend_from_slice(payload);
    bytes
}

pub fn parse_canonical_object(bytes: &[u8]) -> Result<GitObject, GitError> {
    let nul = bytes
        .iter()
        .position(|byte| *byte == 0)
        .ok_or(GitError::MissingHeaderTerminator)?;
    let header = &bytes[..nul];
    let payload = &bytes[nul + 1..];
    let space = header
        .iter()
        .position(|byte| *byte == b' ')
        .ok_or(GitError::MalformedHeader)?;
    let kind = GitObjectKind::parse(&header[..space])?;
    let len = parse_ascii_usize(&header[space + 1..])?;
    if len != payload.len() {
        return Err(GitError::LengthMismatch {
            declared: len,
            actual: payload.len(),
        });
    }
    Ok(GitObject::new(kind, payload))
}

pub fn parse_loose_object(bytes: &[u8]) -> Result<GitObject, GitError> {
    let mut decoder = ZlibDecoder::new(bytes);
    let mut canonical = Vec::new();
    decoder
        .read_to_end(&mut canonical)
        .map_err(|_| GitError::LooseObjectDecode)?;
    parse_canonical_object(&canonical)
}

pub fn parse_packfile(bytes: &[u8]) -> Result<GitPackfile, GitError> {
    if bytes.len() < 32 {
        return Err(GitError::MalformedPack);
    }
    if &bytes[..4] != b"PACK" {
        return Err(GitError::MalformedPack);
    }
    let expected_checksum = sha1_digest(&bytes[..bytes.len() - 20]);
    if bytes[bytes.len() - 20..] != expected_checksum {
        return Err(GitError::PackChecksumMismatch);
    }

    let version = read_be_u32(&bytes[4..8])?;
    if version != 2 && version != 3 {
        return Err(GitError::UnsupportedPackVersion(version));
    }
    let count = read_be_u32(&bytes[8..12])? as usize;
    let mut cursor = 12;
    let end = bytes.len() - 20;
    let mut entries = Vec::with_capacity(count);

    for _ in 0..count {
        let object_start = cursor;
        let (kind, size, header_len) = parse_pack_object_header(&bytes[cursor..end])?;
        cursor += header_len;
        let base = match kind {
            6 => {
                let (base_offset, consumed) =
                    parse_ofs_delta_base_offset(&bytes[cursor..end], object_start)?;
                cursor += consumed;
                Some(PackDeltaBase::Offset(base_offset))
            }
            7 => {
                let base_oid = parse_ref_delta_base_oid(&bytes[cursor..end])?;
                cursor += 20;
                Some(PackDeltaBase::Oid(base_oid))
            }
            _ => None,
        };
        let (payload, consumed) = inflate_pack_object(&bytes[cursor..end])?;
        if payload.len() != size {
            return Err(GitError::LengthMismatch {
                declared: size,
                actual: payload.len(),
            });
        }
        cursor += consumed;
        entries.push(PackEntry {
            offset: object_start,
            kind,
            payload,
            base,
        });
    }

    if cursor != end {
        return Err(GitError::PackTrailingBytes);
    }

    let objects = resolve_pack_entries(&entries)?;

    Ok(GitPackfile { version, objects })
}

pub fn write_packfile(objects: &[GitObject]) -> Result<Vec<u8>, GitError> {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"PACK");
    bytes.extend_from_slice(&2_u32.to_be_bytes());
    bytes.extend_from_slice(
        &u32::try_from(objects.len())
            .map_err(|_| GitError::PackObjectCountOverflow)?
            .to_be_bytes(),
    );

    for object in objects {
        write_pack_object_header(
            &mut bytes,
            pack_kind_code(object.kind),
            object.payload.len(),
        );
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(&object.payload)
            .map_err(|_| GitError::PackObjectEncode)?;
        let compressed = encoder.finish().map_err(|_| GitError::PackObjectEncode)?;
        bytes.extend_from_slice(&compressed);
    }

    let checksum = sha1_digest(&bytes);
    bytes.extend_from_slice(&checksum);
    Ok(bytes)
}

pub fn sha1_oid_for_canonical_bytes(bytes: &[u8]) -> GitSha1Oid {
    let digest = Sha1::digest(bytes);
    let mut out = [0_u8; 20];
    out.copy_from_slice(&digest);
    GitSha1Oid(out)
}

fn parse_ascii_usize(bytes: &[u8]) -> Result<usize, GitError> {
    if bytes.is_empty() || (bytes.len() > 1 && bytes[0] == b'0') {
        return Err(GitError::InvalidLength);
    }
    let mut value = 0_usize;
    for &byte in bytes {
        if !byte.is_ascii_digit() {
            return Err(GitError::InvalidLength);
        }
        value = value
            .checked_mul(10)
            .and_then(|value| value.checked_add((byte - b'0') as usize))
            .ok_or(GitError::InvalidLength)?;
    }
    Ok(value)
}

fn read_be_u32(bytes: &[u8]) -> Result<u32, GitError> {
    let bytes: [u8; 4] = bytes.try_into().map_err(|_| GitError::MalformedPack)?;
    Ok(u32::from_be_bytes(bytes))
}

fn parse_pack_object_header(bytes: &[u8]) -> Result<(u8, usize, usize), GitError> {
    let first = *bytes.first().ok_or(GitError::MalformedPack)?;
    let kind = (first >> 4) & 0b111;
    let mut size = (first & 0b1111) as usize;
    let mut shift = 4;
    let mut consumed = 1;
    let mut byte = first;

    while byte & 0b1000_0000 != 0 {
        byte = *bytes.get(consumed).ok_or(GitError::MalformedPack)?;
        let bits = (byte & 0b0111_1111) as usize;
        size = size
            .checked_add(bits.checked_shl(shift).ok_or(GitError::InvalidLength)?)
            .ok_or(GitError::InvalidLength)?;
        shift = shift.checked_add(7).ok_or(GitError::InvalidLength)?;
        consumed += 1;
    }

    Ok((kind, size, consumed))
}

fn write_pack_object_header(bytes: &mut Vec<u8>, kind: u8, mut size: usize) {
    let mut byte = ((kind & 0b111) << 4) | (size as u8 & 0b1111);
    size >>= 4;
    if size != 0 {
        byte |= 0b1000_0000;
    }
    bytes.push(byte);
    while size != 0 {
        let mut next = (size as u8) & 0b0111_1111;
        size >>= 7;
        if size != 0 {
            next |= 0b1000_0000;
        }
        bytes.push(next);
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct PackEntry {
    offset: usize,
    kind: u8,
    payload: Vec<u8>,
    base: Option<PackDeltaBase>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PackDeltaBase {
    Offset(usize),
    Oid(GitSha1Oid),
}

fn full_object_kind(kind: u8) -> Result<GitObjectKind, GitError> {
    match kind {
        1 => Ok(GitObjectKind::Commit),
        2 => Ok(GitObjectKind::Tree),
        3 => Ok(GitObjectKind::Blob),
        4 => Ok(GitObjectKind::Tag),
        _ => Err(GitError::UnsupportedPackObjectType(kind)),
    }
}

fn parse_ofs_delta_base_offset(
    bytes: &[u8],
    object_offset: usize,
) -> Result<(usize, usize), GitError> {
    let first = *bytes.first().ok_or(GitError::MalformedPack)?;
    let mut value = (first & 0b0111_1111) as usize;
    let mut consumed = 1;
    let mut byte = first;
    while byte & 0b1000_0000 != 0 {
        byte = *bytes.get(consumed).ok_or(GitError::MalformedPack)?;
        value = value
            .checked_add(1)
            .and_then(|value| value.checked_shl(7))
            .and_then(|value| value.checked_add((byte & 0b0111_1111) as usize))
            .ok_or(GitError::InvalidPackDelta)?;
        consumed += 1;
    }
    let base_offset = object_offset
        .checked_sub(value)
        .ok_or(GitError::InvalidPackDelta)?;
    Ok((base_offset, consumed))
}

fn parse_ref_delta_base_oid(bytes: &[u8]) -> Result<GitSha1Oid, GitError> {
    let digest = bytes.get(..20).ok_or(GitError::MalformedPack)?;
    let mut oid = [0_u8; 20];
    oid.copy_from_slice(digest);
    Ok(GitSha1Oid::from_digest(oid))
}

fn resolve_pack_entries(entries: &[PackEntry]) -> Result<Vec<GitObject>, GitError> {
    let mut resolved = vec![None; entries.len()];
    let mut resolving = vec![false; entries.len()];
    let offset_index = entries
        .iter()
        .enumerate()
        .map(|(index, entry)| (entry.offset, index))
        .collect::<BTreeMap<_, _>>();
    let mut oid_index = BTreeMap::new();

    for index in 0..entries.len() {
        let object = resolve_pack_entry(
            index,
            entries,
            &offset_index,
            &mut oid_index,
            &mut resolved,
            &mut resolving,
        )?;
        oid_index.insert(object.sha1_oid(), index);
    }

    resolved
        .into_iter()
        .map(|object| object.ok_or(GitError::InvalidPackDelta))
        .collect()
}

fn resolve_pack_entry(
    index: usize,
    entries: &[PackEntry],
    offset_index: &BTreeMap<usize, usize>,
    oid_index: &mut BTreeMap<GitSha1Oid, usize>,
    resolved: &mut [Option<GitObject>],
    resolving: &mut [bool],
) -> Result<GitObject, GitError> {
    if let Some(object) = &resolved[index] {
        return Ok(object.clone());
    }
    if resolving[index] {
        return Err(GitError::InvalidPackDelta);
    }

    resolving[index] = true;
    let entry = &entries[index];
    let object = match entry.base {
        None => GitObject::new(full_object_kind(entry.kind)?, entry.payload.clone()),
        Some(PackDeltaBase::Offset(base_offset)) => {
            let base_index = *offset_index
                .get(&base_offset)
                .ok_or(GitError::InvalidPackDelta)?;
            let base = resolve_pack_entry(
                base_index,
                entries,
                offset_index,
                oid_index,
                resolved,
                resolving,
            )?;
            GitObject::new(base.kind, apply_pack_delta(&base.payload, &entry.payload)?)
        }
        Some(PackDeltaBase::Oid(base_oid)) => {
            let base_index = if let Some(base_index) = oid_index.get(&base_oid).copied() {
                base_index
            } else {
                let base_index = entries
                    .iter()
                    .enumerate()
                    .find_map(|(candidate, _)| {
                        resolve_pack_entry(
                            candidate,
                            entries,
                            offset_index,
                            oid_index,
                            resolved,
                            resolving,
                        )
                        .ok()
                        .filter(|object| object.sha1_oid() == base_oid)
                        .map(|_| candidate)
                    })
                    .ok_or(GitError::InvalidPackDelta)?;
                oid_index.insert(base_oid, base_index);
                base_index
            };
            let base = resolve_pack_entry(
                base_index,
                entries,
                offset_index,
                oid_index,
                resolved,
                resolving,
            )?;
            GitObject::new(base.kind, apply_pack_delta(&base.payload, &entry.payload)?)
        }
    };
    resolving[index] = false;
    resolved[index] = Some(object.clone());
    Ok(object)
}

fn apply_pack_delta(base: &[u8], delta: &[u8]) -> Result<Vec<u8>, GitError> {
    let mut cursor = 0;
    let source_len = read_delta_varint(delta, &mut cursor)?;
    if source_len != base.len() {
        return Err(GitError::InvalidPackDelta);
    }
    let target_len = read_delta_varint(delta, &mut cursor)?;
    let mut out = Vec::with_capacity(target_len);

    while cursor < delta.len() {
        let command = delta[cursor];
        cursor += 1;
        if command & 0b1000_0000 != 0 {
            let mut offset = 0_usize;
            let mut size = 0_usize;
            for shift in 0..4 {
                if command & (1 << shift) != 0 {
                    let byte = *delta.get(cursor).ok_or(GitError::InvalidPackDelta)?;
                    cursor += 1;
                    offset |= (byte as usize) << (shift * 8);
                }
            }
            for shift in 0..3 {
                if command & (1 << (4 + shift)) != 0 {
                    let byte = *delta.get(cursor).ok_or(GitError::InvalidPackDelta)?;
                    cursor += 1;
                    size |= (byte as usize) << (shift * 8);
                }
            }
            if size == 0 {
                size = 0x10000;
            }
            let end = offset.checked_add(size).ok_or(GitError::InvalidPackDelta)?;
            let bytes = base.get(offset..end).ok_or(GitError::InvalidPackDelta)?;
            out.extend_from_slice(bytes);
        } else if command != 0 {
            let size = command as usize;
            let end = cursor.checked_add(size).ok_or(GitError::InvalidPackDelta)?;
            let bytes = delta.get(cursor..end).ok_or(GitError::InvalidPackDelta)?;
            out.extend_from_slice(bytes);
            cursor = end;
        } else {
            return Err(GitError::InvalidPackDelta);
        }
    }

    if out.len() != target_len {
        return Err(GitError::InvalidPackDelta);
    }
    Ok(out)
}

fn read_delta_varint(bytes: &[u8], cursor: &mut usize) -> Result<usize, GitError> {
    let mut value = 0_usize;
    let mut shift = 0;
    loop {
        let byte = *bytes.get(*cursor).ok_or(GitError::InvalidPackDelta)?;
        *cursor += 1;
        value = value
            .checked_add(
                ((byte & 0b0111_1111) as usize)
                    .checked_shl(shift)
                    .ok_or(GitError::InvalidPackDelta)?,
            )
            .ok_or(GitError::InvalidPackDelta)?;
        if byte & 0b1000_0000 == 0 {
            return Ok(value);
        }
        shift = shift.checked_add(7).ok_or(GitError::InvalidPackDelta)?;
    }
}

fn pack_kind_code(kind: GitObjectKind) -> u8 {
    match kind {
        GitObjectKind::Commit => 1,
        GitObjectKind::Tree => 2,
        GitObjectKind::Blob => 3,
        GitObjectKind::Tag => 4,
    }
}

fn inflate_pack_object(bytes: &[u8]) -> Result<(Vec<u8>, usize), GitError> {
    let mut decoder = ZlibDecoder::new(bytes);
    let mut payload = Vec::new();
    decoder
        .read_to_end(&mut payload)
        .map_err(|_| GitError::PackObjectDecode)?;
    let consumed = usize::try_from(decoder.total_in()).map_err(|_| GitError::MalformedPack)?;
    if consumed == 0 {
        return Err(GitError::PackObjectDecode);
    }
    Ok((payload, consumed))
}

fn sha1_digest(bytes: &[u8]) -> [u8; 20] {
    let digest = Sha1::digest(bytes);
    let mut out = [0_u8; 20];
    out.copy_from_slice(&digest);
    out
}

fn parse_oid_bytes(bytes: &[u8]) -> Result<GitSha1Oid, GitError> {
    let value = std::str::from_utf8(bytes).map_err(|_| GitError::InvalidOid)?;
    GitSha1Oid::from_str(value)
}

fn tree_entry_target(mode: &[u8]) -> Result<GitTreeEntryTarget, GitError> {
    match mode {
        b"40000" => Ok(GitTreeEntryTarget::Tree),
        b"100644" | b"100755" | b"120000" => Ok(GitTreeEntryTarget::Blob),
        b"160000" => Ok(GitTreeEntryTarget::Commit),
        _ => Err(GitError::UnsupportedTreeMode),
    }
}

#[derive(Debug, Error)]
pub enum GitError {
    #[error("unknown Git object kind")]
    UnknownObjectKind,
    #[error("canonical Git object is missing NUL header terminator")]
    MissingHeaderTerminator,
    #[error("Git object header is malformed")]
    MalformedHeader,
    #[error("Git object length is invalid")]
    InvalidLength,
    #[error("Git object length mismatch: declared {declared}, actual {actual}")]
    LengthMismatch { declared: usize, actual: usize },
    #[error("invalid SHA-1 Git object id")]
    InvalidOid,
    #[error("Git commit is missing a tree header")]
    MissingCommitTree,
    #[error("Git tree object is malformed")]
    MalformedTree,
    #[error("Git tree entry mode is unsupported")]
    UnsupportedTreeMode,
    #[error("Git loose object zlib decode failed")]
    LooseObjectDecode,
    #[error("Git packfile is malformed")]
    MalformedPack,
    #[error("Git packfile checksum mismatch")]
    PackChecksumMismatch,
    #[error("unsupported Git packfile version {0}")]
    UnsupportedPackVersion(u32),
    #[error("unsupported Git pack object type {0}")]
    UnsupportedPackObjectType(u8),
    #[error("Git pack delta object is invalid")]
    InvalidPackDelta,
    #[error("Git pack object zlib decode failed")]
    PackObjectDecode,
    #[error("Git pack object zlib encode failed")]
    PackObjectEncode,
    #[error("Git packfile contains trailing bytes")]
    PackTrailingBytes,
    #[error("Git packfile object count exceeds u32")]
    PackObjectCountOverflow,
}

fn hex_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn computes_known_blob_sha1_oid() {
        let object = GitObject::new(GitObjectKind::Blob, b"hello world\n");

        assert_eq!(
            object.sha1_oid().hex(),
            "3b18e512dba79e4c8300dd08aeb37f8e728b8dad"
        );
    }

    #[test]
    fn parses_canonical_git_object() {
        let bytes = canonical_object_bytes(GitObjectKind::Blob, b"hello");
        let parsed = parse_canonical_object(&bytes).unwrap();

        assert_eq!(parsed.kind, GitObjectKind::Blob);
        assert_eq!(parsed.payload, b"hello");
    }

    #[test]
    fn parses_loose_object_zlib_bytes() {
        use flate2::Compression;
        use flate2::write::ZlibEncoder;
        use std::io::Write;

        let canonical = canonical_object_bytes(GitObjectKind::Blob, b"hello");
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(&canonical).unwrap();
        let compressed = encoder.finish().unwrap();

        let object = parse_loose_object(&compressed).unwrap();

        assert_eq!(object, GitObject::new(GitObjectKind::Blob, b"hello"));
    }

    #[test]
    fn packfile_round_trips_full_objects() {
        let objects = vec![
            GitObject::new(GitObjectKind::Blob, b"hello"),
            GitObject::new(GitObjectKind::Tree, Vec::new()),
        ];

        let pack = write_packfile(&objects).unwrap();
        let parsed = parse_packfile(&pack).unwrap();

        assert_eq!(parsed.version, 2);
        assert_eq!(parsed.objects, objects);
    }

    #[test]
    fn packfile_rejects_bad_checksum() {
        let mut pack = write_packfile(&[GitObject::new(GitObjectKind::Blob, b"hello")]).unwrap();
        let last = pack.len() - 1;
        pack[last] ^= 1;

        assert!(matches!(
            parse_packfile(&pack).unwrap_err(),
            GitError::PackChecksumMismatch
        ));
    }

    #[test]
    fn packfile_resolves_ofs_delta_objects() {
        let mut pack = Vec::new();
        pack.extend_from_slice(b"PACK");
        pack.extend_from_slice(&2_u32.to_be_bytes());
        pack.extend_from_slice(&2_u32.to_be_bytes());

        let base_offset = pack.len();
        write_pack_object_header(&mut pack, 3, 5);
        write_zlib(&mut pack, b"hello");

        let delta_offset = pack.len();
        let delta = [5, 11, 0x90, 5, 6, b' ', b'w', b'o', b'r', b'l', b'd'];
        write_pack_object_header(&mut pack, 6, delta.len());
        write_ofs_delta_base_offset(&mut pack, delta_offset - base_offset);
        write_zlib(&mut pack, &delta);

        let checksum = sha1_digest(&pack);
        pack.extend_from_slice(&checksum);

        let parsed = parse_packfile(&pack).unwrap();

        assert_eq!(
            parsed.objects[0],
            GitObject::new(GitObjectKind::Blob, b"hello")
        );
        assert_eq!(
            parsed.objects[1],
            GitObject::new(GitObjectKind::Blob, b"hello world")
        );
    }

    #[test]
    fn packfile_resolves_ref_delta_objects() {
        let base = GitObject::new(GitObjectKind::Blob, b"hello");
        let mut pack = Vec::new();
        pack.extend_from_slice(b"PACK");
        pack.extend_from_slice(&2_u32.to_be_bytes());
        pack.extend_from_slice(&2_u32.to_be_bytes());

        write_pack_object_header(&mut pack, 3, base.payload.len());
        write_zlib(&mut pack, &base.payload);

        let delta = [
            5, 13, 0x90, 5, 8, b' ', b'g', b'i', b't', b'm', b'e', b's', b'h',
        ];
        write_pack_object_header(&mut pack, 7, delta.len());
        pack.extend_from_slice(&base.sha1_oid().digest());
        write_zlib(&mut pack, &delta);

        let checksum = sha1_digest(&pack);
        pack.extend_from_slice(&checksum);

        let parsed = parse_packfile(&pack).unwrap();

        assert_eq!(parsed.objects[0], base);
        assert_eq!(
            parsed.objects[1],
            GitObject::new(GitObjectKind::Blob, b"hello gitmesh")
        );
    }

    #[test]
    fn rejects_length_mismatch() {
        let err = parse_canonical_object(b"blob 6\0hello").unwrap_err();

        assert!(matches!(
            err,
            GitError::LengthMismatch {
                declared: 6,
                actual: 5
            }
        ));
    }

    #[test]
    fn parses_sha1_oid_hex() {
        let oid = GitSha1Oid::from_str("3b18e512dba79e4c8300dd08aeb37f8e728b8dad").unwrap();

        assert_eq!(oid.hex(), "3b18e512dba79e4c8300dd08aeb37f8e728b8dad");
        assert!(GitSha1Oid::from_str("not-an-oid").is_err());
    }

    #[test]
    fn parses_commit_tree_and_parents() {
        let links = parse_commit_links(
            b"tree 4b825dc642cb6eb9a060e54bf8d69288fbee4904\nparent 1111111111111111111111111111111111111111\nauthor A <a@example.com> 0 +0000\n\nmessage\n",
        )
        .unwrap();

        assert_eq!(
            links.tree,
            GitSha1Oid::from_str("4b825dc642cb6eb9a060e54bf8d69288fbee4904").unwrap()
        );
        assert_eq!(links.parents.len(), 1);
    }

    #[test]
    fn commit_requires_tree_header() {
        assert!(matches!(
            parse_commit_links(b"author A <a@example.com> 0 +0000\n\nmessage\n").unwrap_err(),
            GitError::MissingCommitTree
        ));
    }

    #[test]
    fn parses_tree_entries() {
        let blob_oid = GitSha1Oid::from_str("3b18e512dba79e4c8300dd08aeb37f8e728b8dad").unwrap();
        let mut payload = Vec::new();
        payload.extend_from_slice(b"100644 README.md\0");
        payload.extend_from_slice(&blob_oid.digest());

        let entries = parse_tree_entries(&payload).unwrap();

        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].target, GitTreeEntryTarget::Blob);
        assert_eq!(entries[0].name, b"README.md");
        assert_eq!(entries[0].oid, blob_oid);
    }

    #[test]
    fn tree_parser_rejects_unsupported_mode() {
        let mut payload = Vec::new();
        payload.extend_from_slice(b"777 file\0");
        payload.extend_from_slice(&[0_u8; 20]);

        assert!(matches!(
            parse_tree_entries(&payload).unwrap_err(),
            GitError::UnsupportedTreeMode
        ));
    }

    fn write_zlib(out: &mut Vec<u8>, bytes: &[u8]) {
        use flate2::Compression;
        use flate2::write::ZlibEncoder;
        use std::io::Write;

        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
        encoder.write_all(bytes).unwrap();
        out.extend_from_slice(&encoder.finish().unwrap());
    }

    fn write_ofs_delta_base_offset(out: &mut Vec<u8>, offset: usize) {
        let mut bytes = vec![(offset & 0x7f) as u8];
        let mut value = offset >> 7;
        while value != 0 {
            value -= 1;
            bytes.push(((value & 0x7f) as u8) | 0x80);
            value >>= 7;
        }
        bytes.reverse();
        out.extend_from_slice(&bytes);
    }
}
