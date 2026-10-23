//! One bounded reader for picked, dropped and pasted images. It runs on the
//! background executor; a batch never reads bytes it cannot fit in the draft.
use std::io::Read as _;
use std::path::{Path, PathBuf};

use proteus_contracts::domain::{ImageAttachment, image_mime_type};

#[derive(Clone, Copy)]
pub(super) struct Budget {
    pub slots: usize,
    pub bytes: usize,
}

pub(super) enum Source {
    File(PathBuf),
    Bytes(String, Vec<u8>),
}

pub(super) struct Prepared {
    pub image: ImageAttachment,
    pub bytes: Vec<u8>,
}

#[derive(Default)]
pub(super) struct Batch {
    pub images: Vec<Prepared>,
    pub errors: Vec<String>,
}

pub(super) fn prepare(name: String, bytes: Vec<u8>) -> Result<Prepared, String> {
    let image = ImageAttachment::from_bytes(name.clone(), &bytes)
        .map_err(|error| format!("{name}: {error}"))?;
    Ok(Prepared { image, bytes })
}

pub(super) fn read_batch(sources: Vec<Source>, mut budget: Budget) -> Batch {
    let mut batch = Batch::default();
    let mut omitted_errors = 0;
    for source in sources {
        // Check before metadata/open/read: the rest of a large drop is not
        // loaded merely to be rejected by the UI afterwards.
        if budget.slots == 0 || budget.bytes == 0 {
            batch.errors.push(
                "Остальные изображения не помещаются в лимит сообщения (4 изображения, 5 МиБ)"
                    .into(),
            );
            break;
        }
        let prepared = match source {
            Source::File(path) => read_file(&path, budget.bytes),
            Source::Bytes(name, bytes) => {
                if bytes.len() > budget.bytes {
                    Err(format!("{name}: изображения сообщения вместе больше 5 МиБ"))
                } else {
                    prepare(name, bytes)
                }
            }
        };
        match prepared {
            Ok(image) => {
                budget.slots -= 1;
                budget.bytes -= image.bytes.len();
                batch.images.push(image);
            }
            Err(error) if batch.errors.len() < 4 => batch.errors.push(error),
            Err(_) => omitted_errors += 1,
        }
    }
    if omitted_errors > 0 {
        batch
            .errors
            .push(format!("Ещё отклонённых файлов: {omitted_errors}"));
    }
    batch
}

fn read_file(path: &Path, limit: usize) -> Result<Prepared, String> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string());
    let metadata = std::fs::metadata(path).map_err(|error| format!("{name}: {error}"))?;
    if !metadata.is_file() {
        return Err(format!("{name}: выберите обычный файл изображения"));
    }
    if metadata.len() > limit as u64 {
        return Err(format!(
            "{name}: изображение не помещается в оставшийся лимит 5 МиБ"
        ));
    }
    let file = std::fs::File::open(path).map_err(|error| format!("{name}: {error}"))?;
    let bytes = read_limited(file, limit).map_err(|error| format!("{name}: {error}"))?;
    prepare(name, bytes)
}

/// A file may grow after metadata. `take(limit + 1)` still caps the actual
/// read, including the lookahead needed to report that growth as too large.
/// Unsupported files need only their twelve-byte magic, not their contents.
fn read_limited(mut reader: impl std::io::Read, limit: usize) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    reader
        .by_ref()
        .take((limit.saturating_add(1)).min(12) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err("изображения сообщения вместе больше 5 МиБ".into());
    }
    if image_mime_type(&bytes).is_none() {
        return Err("поддерживаются PNG, JPEG, WebP и GIF".into());
    }
    reader
        .take(limit.saturating_add(1).saturating_sub(bytes.len()) as u64)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() > limit {
        return Err("изображения сообщения вместе больше 5 МиБ".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn png(size: usize) -> Vec<u8> {
        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.resize(size.max(bytes.len()), 0);
        bytes
    }

    #[test]
    fn growing_file_read_stops_at_the_remaining_budget_plus_one() {
        let mut source = Cursor::new(png(1024));
        assert!(read_limited(&mut source, 32).is_err());
        assert_eq!(source.position(), 33);
    }

    #[test]
    fn unsupported_file_only_reads_its_signature() {
        let mut source = Cursor::new(vec![b'x'; 1024]);
        assert!(read_limited(&mut source, 1024).is_err());
        assert_eq!(source.position(), 12);
    }

    #[test]
    fn accepted_bytes_and_file_sources_share_one_remaining_budget() {
        let root =
            std::env::temp_dir().join(format!("proteus-image-budget-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let path = root.join("from-file.png");
        std::fs::write(&path, png(16)).unwrap();
        let batch = read_batch(
            vec![
                Source::Bytes("pasted.png".into(), png(16)),
                Source::File(path),
                Source::File(root.join("must-not-be-opened.png")),
            ],
            Budget {
                slots: 2,
                bytes: 32,
            },
        );
        assert_eq!(batch.images.len(), 2);
        assert_eq!(
            batch
                .images
                .iter()
                .map(|image| image.bytes.len())
                .sum::<usize>(),
            32
        );
        assert_eq!(batch.errors.len(), 1);
        assert!(batch.errors[0].starts_with("Остальные изображения"));
        std::fs::remove_dir_all(root).unwrap();
    }
}
