//! Extract zip / tar archives into a destination directory.

use crate::{Error, Result};
use flate2::read::GzDecoder;
use std::fs::{self, File};
use std::io::{self, Cursor, Read};
use std::path::{Component, Path, PathBuf};
use tar::Archive as TarArchive;
use zip::ZipArchive;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArchiveKind {
    Zip,
    Tar,
    TarGz,
}

impl ArchiveKind {
    pub fn from_type_and_url(dist_type: Option<&str>, url: &str) -> Result<Self> {
        if let Some(t) = dist_type {
            match t.to_ascii_lowercase().as_str() {
                "zip" => return Ok(Self::Zip),
                "tar" => return Ok(Self::Tar),
                "tar.gz" | "tgz" => return Ok(Self::TarGz),
                other => return Err(Error::UnsupportedArchive(other.to_owned())),
            }
        }
        let lower = url.to_ascii_lowercase();
        if lower.contains(".tar.gz") || lower.ends_with(".tgz") {
            Ok(Self::TarGz)
        } else if lower.contains(".tar") {
            Ok(Self::Tar)
        } else {
            Ok(Self::Zip)
        }
    }
}

/// Extract `bytes` into `dest`, stripping a single top-level directory when present
/// (Composer / GitHub zipball layout).
pub fn extract_archive(bytes: &[u8], kind: ArchiveKind, dest: &Path) -> Result<()> {
    fs::create_dir_all(dest)?;
    match kind {
        ArchiveKind::Zip => extract_zip(bytes, dest),
        ArchiveKind::Tar => extract_tar_bytes(bytes, false, dest),
        ArchiveKind::TarGz => extract_tar_bytes(bytes, true, dest),
    }
}

fn extract_zip(bytes: &[u8], dest: &Path) -> Result<()> {
    let mut archive =
        ZipArchive::new(Cursor::new(bytes)).map_err(|e| Error::Extract(e.to_string()))?;
    let strip = zip_strip_prefix(&mut archive)?;

    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| Error::Extract(e.to_string()))?;
        let Some(raw_name) = file.enclosed_name().map(|p| p.to_path_buf()) else {
            continue;
        };
        let rel = strip_prefix_path(&raw_name, strip.as_deref());
        if rel.as_os_str().is_empty() {
            continue;
        }
        let out = dest.join(&rel);
        deny_path_escape(&rel)?;
        if file.is_dir() {
            fs::create_dir_all(&out)?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut outfile = File::create(&out)?;
            io::copy(&mut file, &mut outfile)?;
        }
    }
    Ok(())
}

fn zip_strip_prefix<R: Read + io::Seek>(archive: &mut ZipArchive<R>) -> Result<Option<PathBuf>> {
    let mut roots = Vec::new();
    for i in 0..archive.len() {
        let file = archive
            .by_index(i)
            .map_err(|e| Error::Extract(e.to_string()))?;
        let Some(name) = file.enclosed_name() else {
            continue;
        };
        if let Some(Component::Normal(first)) = name.components().next() {
            let first = PathBuf::from(first);
            if !roots.iter().any(|r: &PathBuf| r == &first) {
                roots.push(first);
            }
        }
    }
    if roots.len() == 1 {
        Ok(Some(roots.remove(0)))
    } else {
        Ok(None)
    }
}

fn extract_tar_bytes(bytes: &[u8], gzip: bool, dest: &Path) -> Result<()> {
    let strip = {
        let reader: Box<dyn Read> = if gzip {
            Box::new(GzDecoder::new(Cursor::new(bytes)))
        } else {
            Box::new(Cursor::new(bytes))
        };
        let mut archive = TarArchive::new(reader);
        let mut roots = Vec::new();
        for entry in archive
            .entries()
            .map_err(|e| Error::Extract(e.to_string()))?
        {
            let entry = entry.map_err(|e| Error::Extract(e.to_string()))?;
            let path = entry.path().map_err(|e| Error::Extract(e.to_string()))?;
            if let Some(Component::Normal(first)) = path.components().next() {
                let first = PathBuf::from(first);
                if !roots.iter().any(|r: &PathBuf| r == &first) {
                    roots.push(first);
                }
            }
        }
        if roots.len() == 1 {
            Some(roots.remove(0))
        } else {
            None
        }
    };

    let reader: Box<dyn Read> = if gzip {
        Box::new(GzDecoder::new(Cursor::new(bytes)))
    } else {
        Box::new(Cursor::new(bytes))
    };
    let mut archive = TarArchive::new(reader);
    for entry in archive
        .entries()
        .map_err(|e| Error::Extract(e.to_string()))?
    {
        let mut entry = entry.map_err(|e| Error::Extract(e.to_string()))?;
        let path = entry
            .path()
            .map_err(|e| Error::Extract(e.to_string()))?
            .into_owned();
        let rel = strip_prefix_path(&path, strip.as_deref());
        if rel.as_os_str().is_empty() {
            continue;
        }
        deny_path_escape(&rel)?;
        let out = dest.join(&rel);
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        entry
            .unpack(&out)
            .map_err(|e| Error::Extract(e.to_string()))?;
    }
    Ok(())
}

fn strip_prefix_path(path: &Path, strip: Option<&Path>) -> PathBuf {
    match strip {
        Some(prefix) => path.strip_prefix(prefix).unwrap_or(path).to_path_buf(),
        None => path.to_path_buf(),
    }
}

fn deny_path_escape(rel: &Path) -> Result<()> {
    if rel.is_absolute() || rel.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err(Error::Extract(
            "archive entry escapes destination directory".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::ZipWriter;
    use zip::write::SimpleFileOptions;

    #[test]
    fn extracts_zip_stripping_root() {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            let opts = SimpleFileOptions::default();
            zip.start_file("pkg-root/hello.txt", opts).expect("start");
            zip.write_all(b"hi").expect("write");
            zip.finish().expect("finish");
        }
        let bytes = cursor.into_inner();
        let dir = tempdir().expect("temp");
        extract_archive(&bytes, ArchiveKind::Zip, dir.path()).expect("extract");
        assert_eq!(
            fs::read_to_string(dir.path().join("hello.txt")).expect("read"),
            "hi"
        );
    }
}
