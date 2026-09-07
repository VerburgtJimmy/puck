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
        // Symlinks / hardlinks let a later entry write through a planted link
        // (e.g. `pkg/link -> /tmp/evil` then `pkg/link/x`). Refuse them. Zip
        // never creates symlinks today; keep that property if zip symlink
        // support is added later (refuse write-through-symlinked-parent).
        match entry.header().entry_type() {
            tar::EntryType::Symlink | tar::EntryType::Link => {
                let name = entry
                    .path()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|_| "<unknown>".into());
                return Err(Error::Extract(format!(
                    "archive entry `{name}` is a symlink/hardlink; puck refuses link entries in dist archives"
                )));
            }
            _ => {}
        }
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

    #[test]
    fn extracts_tar_stripping_root() {
        let mut buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut buf);
            let mut header = tar::Header::new_gnu();
            header.set_path("pkg-root/hello.txt").expect("path");
            header.set_size(2);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append(&header, b"hi".as_slice()).expect("append");
            builder.finish().expect("finish");
        }
        let dir = tempdir().expect("temp");
        extract_archive(&buf, ArchiveKind::Tar, dir.path()).expect("extract");
        assert_eq!(
            fs::read_to_string(dir.path().join("hello.txt")).expect("read"),
            "hi"
        );
    }

    /// PoC: symlink entry pointing outside dest, then a file under that link.
    /// Without the Symlink/Link refusal this writes outside `dest`.
    #[test]
    fn refuses_tar_symlink_escape() {
        let outside = tempdir().expect("outside");
        let escape_target = outside.path().join("pwned.txt");

        let mut buf = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut buf);

            let mut link = tar::Header::new_gnu();
            link.set_entry_type(tar::EntryType::Symlink);
            link.set_size(0);
            link.set_path("pkg/link").expect("link path");
            link.set_link_name(&escape_target).expect("link target");
            link.set_cksum();
            builder
                .append(&link, std::io::empty())
                .expect("append symlink");

            let mut file = tar::Header::new_gnu();
            file.set_path("pkg/link/hello.txt").expect("file path");
            file.set_size(5);
            file.set_mode(0o644);
            file.set_cksum();
            builder
                .append(&file, b"pwned".as_slice())
                .expect("append file");
            builder.finish().expect("finish");
        }

        let dest = tempdir().expect("dest");
        let err = extract_archive(&buf, ArchiveKind::Tar, dest.path()).expect_err("must refuse");
        let msg = err.to_string();
        assert!(
            msg.contains("symlink") || msg.contains("hardlink"),
            "unexpected error: {msg}"
        );
        assert!(
            !escape_target.exists(),
            "escape target must not be written"
        );
        assert!(
            !dest.path().join("link").exists(),
            "symlink must not be planted in dest"
        );
    }
}
