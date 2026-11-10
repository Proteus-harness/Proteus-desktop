use super::*;
use std::{
    fs::{self, File},
    io::{self, BufReader, BufWriter, Read, Write},
};

fn name(identity: &Hash) -> String {
    identity
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
        + ".heights"
}
fn digest(spans: &[ColdSpan]) -> Hash {
    let mut hash = Sha256::new();
    for span in spans {
        hash.update((span.rows as u64).to_le_bytes());
        hash.update(span.pixels.to_le_bytes());
        hash.update([span.kind]);
    }
    hash.finalize().into()
}
fn invalid() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "invalid scalar height cache")
}

/// Package preferences belong to extension processes. Read their durable
/// bytes once on the disk worker, with aggregate file/byte admission before
/// opening payloads. No JSON projection or settings copy is retained.
pub(super) fn preferences_fingerprint(root: Option<&Path>) -> io::Result<Hash> {
    const MAX_SETTINGS_FILES: usize = 128;
    const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;
    let mut digest = Sha256::new();
    digest.update(b"extension-preferences-v1");
    let Some(root) = root else {
        return Ok(digest.finalize().into());
    };
    let directory = match fs::read_dir(root) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(digest.finalize().into());
        }
        Err(error) => return Err(error),
    };
    let mut paths = Vec::new();
    let mut admitted = 0u64;
    for (scanned, entry) in directory.enumerate() {
        if scanned >= 512 {
            return Err(invalid());
        }
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        if paths.len() >= MAX_SETTINGS_FILES {
            return Err(invalid());
        }
        let metadata = entry.metadata()?;
        admitted = admitted.checked_add(metadata.len()).ok_or_else(invalid)?;
        if !metadata.is_file() || admitted > MAX_SETTINGS_BYTES {
            return Err(invalid());
        }
        paths.push(path);
    }
    paths.sort_unstable();
    let mut read_bytes = 0u64;
    let mut buffer = [0; 4096];
    for path in paths {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(invalid)?;
        digest.update((name.len() as u64).to_le_bytes());
        digest.update(name.as_bytes());
        let mut file = File::open(path)?;
        let mut file_digest = Sha256::new();
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            read_bytes += count as u64;
            if read_bytes > MAX_SETTINGS_BYTES {
                return Err(invalid());
            }
            file_digest.update(&buffer[..count]);
        }
        digest.update(file_digest.finalize());
    }
    Ok(digest.finalize().into())
}

pub(super) fn clear(root: &Path) -> io::Result<()> {
    let directory = match fs::read_dir(root) {
        Ok(directory) => directory,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in directory {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|extension| extension == "heights" || extension == "tmp")
        {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}
fn bytes<const N: usize>(read: &mut impl Read) -> io::Result<[u8; N]> {
    let mut bytes = [0; N];
    read.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn valid_body(read: &mut impl Read, expected: Hash) -> bool {
    let mut digest = Sha256::new();
    let mut buffer = [0; 4096];
    loop {
        match read.read(&mut buffer) {
            Ok(0) => return Hash::from(digest.finalize()) == expected,
            Ok(count) => digest.update(&buffer[..count]),
            Err(_) => return false,
        }
    }
}

pub(super) fn load(root: &Path, requested: Key, permit: Permit) -> io::Result<Option<Snapshot>> {
    let file = match File::open(root.join(name(&requested.identity))) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    };
    let length = file.metadata()?.len() as usize;
    if !(HEADER_BYTES..=MAX_FILE_BYTES).contains(&length) {
        return Err(invalid());
    }
    let mut file = BufReader::new(file);
    if &bytes::<8>(&mut file)? != MAGIC
        || bytes::<32>(&mut file)? != requested.identity
        || bytes::<32>(&mut file)? != requested.layout
    {
        return Ok(None);
    }
    let revision = bytes::<32>(&mut file)?;
    let expected = bytes::<32>(&mut file)?;
    let rows = usize::try_from(u64::from_le_bytes(bytes(&mut file)?)).map_err(|_| invalid())?;
    let count = u32::from_le_bytes(bytes(&mut file)?) as usize;
    if count > MAX_SPANS || length != HEADER_BYTES + count * SPAN_BYTES {
        return Err(invalid());
    }
    let keep = if revision == requested.revision {
        requested.prefix
    } else if let Some((_, prefix)) = requested.base.filter(|(base, _)| *base == revision) {
        prefix.min(requested.prefix)
    } else {
        return Ok(None);
    }
    .min(rows);
    if keep == 0 {
        return Ok(None);
    }
    let mut spans = Vec::with_capacity(count);
    let mut total = 0usize;
    let mut hash = Sha256::new();
    for _ in 0..count {
        let row_bytes = bytes::<8>(&mut file)?;
        let pixel_bytes = bytes::<8>(&mut file)?;
        let tag = bytes::<1>(&mut file)?;
        hash.update(row_bytes);
        hash.update(pixel_bytes);
        hash.update(tag);
        let span_rows = usize::try_from(u64::from_le_bytes(row_bytes)).map_err(|_| invalid())?;
        let pixels = f64::from_le_bytes(pixel_bytes);
        if span_rows == 0 || !pixels.is_finite() || pixels < 0. || tag[0] > 2 {
            return Err(invalid());
        }
        if total < keep {
            let take = span_rows.min(keep - total);
            spans.push(ColdSpan {
                rows: take,
                pixels: pixels * take as f64 / span_rows as f64,
                kind: tag[0],
            });
        }
        total = total.checked_add(span_rows).ok_or_else(invalid)?;
    }
    if total != rows || Hash::from(hash.finalize()) != expected {
        return Err(invalid());
    }
    let mut key = requested;
    key.prefix = keep;
    Ok(Some(Snapshot::new(key, spans, permit)))
}

pub(super) fn save(
    root: &Path,
    snapshot: &Snapshot,
    budget: u64,
    files: usize,
) -> io::Result<bool> {
    if snapshot.spans.len() > MAX_SPANS {
        return Err(invalid());
    }
    let count = snapshot.spans.len();
    let size = (HEADER_BYTES + count * SPAN_BYTES) as u64;
    if size > budget || files == 0 {
        return Ok(false);
    }
    let digest = digest(&snapshot.spans);
    fs::create_dir_all(root)?;
    let path = root.join(name(&snapshot.key.identity));
    // Normalize freshness in ColdSpan, then compare scalar digest on the disk
    // worker. A warm traversal does not update mtime or rewrite equal geometry.
    if let Ok(file) = File::open(&path) {
        let mut read = BufReader::new(file);
        if read
            .get_ref()
            .metadata()
            .is_ok_and(|metadata| metadata.len() == size)
            && bytes::<8>(&mut read).ok().as_ref() == Some(MAGIC)
            && bytes::<32>(&mut read).ok() == Some(snapshot.key.identity)
            && bytes::<32>(&mut read).ok() == Some(snapshot.key.layout)
            && bytes::<32>(&mut read).ok() == Some(snapshot.key.revision)
            && bytes::<32>(&mut read).ok() == Some(digest)
            && bytes::<8>(&mut read).ok().map(u64::from_le_bytes)
                == Some(snapshot.key.prefix as u64)
            && bytes::<4>(&mut read).ok().map(u32::from_le_bytes) == Some(count as u32)
            && valid_body(&mut read, digest)
        {
            return Ok(false);
        }
    }
    // Include the temporary file in peak disk admission, not merely the final
    // replacement. The existing target remains intact until atomic rename.
    let mut entries = Vec::new();
    for entry in fs::read_dir(root)?.filter_map(Result::ok) {
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "tmp") {
            // A crash before rename leaves only disposable scalar output.
            fs::remove_file(path)?;
            continue;
        }
        if path.extension().is_none_or(|ext| ext != "heights") {
            continue;
        }
        let metadata = entry.metadata()?;
        entries.push((metadata.modified().ok(), path, metadata.len()));
    }
    entries.sort_unstable();
    let mut bytes: u64 = entries.iter().map(|(_, _, size)| size).sum();
    let mut retained = entries.len();
    for (_, candidate, length) in entries {
        if bytes.saturating_add(size) <= budget && retained < files {
            break;
        }
        if candidate == path {
            continue;
        }
        fs::remove_file(candidate)?;
        bytes = bytes.saturating_sub(length);
        retained -= 1;
    }
    if bytes.saturating_add(size) > budget || retained >= files {
        return Ok(false);
    }
    let draft = path.with_extension("tmp");
    let result = (|| {
        let mut write = BufWriter::new(File::create(&draft)?);
        write.write_all(MAGIC)?;
        write.write_all(&snapshot.key.identity)?;
        write.write_all(&snapshot.key.layout)?;
        write.write_all(&snapshot.key.revision)?;
        write.write_all(&digest)?;
        write.write_all(&(snapshot.key.prefix as u64).to_le_bytes())?;
        write.write_all(&(count as u32).to_le_bytes())?;
        for span in &snapshot.spans {
            write.write_all(&(span.rows as u64).to_le_bytes())?;
            write.write_all(&span.pixels.to_le_bytes())?;
            write.write_all(&[span.kind])?;
        }
        write.flush()?;
        fs::rename(&draft, &path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(draft);
    }
    result.map(|_| true)
}
