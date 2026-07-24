//! unpkg-style edge serving of package contents: extract one file from a
//! stored artifact so the web can consume packages without installing them.

use std::io::{Cursor, Read};

use axum::http::header::{self, HeaderName, HeaderValue};
use flate2::read::GzDecoder;
use zed_interfaces::artifact::ArtifactFormat;

/// Cache policy for served package files: content is sha-addressed and
/// immutable, so it can be cached indefinitely.
const IMMUTABLE_CACHE: &str = "public, max-age=31536000, immutable";

/// Largest single file served out of an artifact. Also the allocation cap:
/// archive headers declare sizes and are attacker-controlled, so they are
/// never trusted for buffer sizing.
pub const MAX_SERVED_FILE_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Debug)]
pub enum ExtractError {
    /// The entry is larger than [`MAX_SERVED_FILE_BYTES`] (declared or actual).
    TooLarge,
    /// The archive could not be read.
    Archive(anyhow::Error),
}

impl From<std::io::Error> for ExtractError {
    fn from(err: std::io::Error) -> Self {
        Self::Archive(err.into())
    }
}

impl From<zip::result::ZipError> for ExtractError {
    fn from(err: zip::result::ZipError) -> Self {
        Self::Archive(err.into())
    }
}

/// Find `pkg/<rel_path>` inside a stored artifact of the given format.
pub fn extract_file(
    archive: &[u8],
    format: ArtifactFormat,
    rel_path: &str,
) -> Result<Option<Vec<u8>>, ExtractError> {
    let want = format!("{}/{rel_path}", zed_interfaces::paths::ARCHIVE_ROOT);
    match format {
        ArtifactFormat::TarGz => {
            let mut tar = tar::Archive::new(GzDecoder::new(archive));
            for entry in tar.entries()? {
                let entry = entry?;
                if entry.path()?.to_string_lossy() == want {
                    if entry.size() > MAX_SERVED_FILE_BYTES {
                        return Err(ExtractError::TooLarge);
                    }
                    return Ok(Some(read_capped(entry)?));
                }
            }
            Ok(None)
        }
        ArtifactFormat::Zip => {
            let mut zip = zip::ZipArchive::new(Cursor::new(archive))?;
            let entry = match zip.by_name(&want) {
                Ok(entry) => entry,
                Err(zip::result::ZipError::FileNotFound) => return Ok(None),
                Err(err) => return Err(err.into()),
            };
            if entry.size() > MAX_SERVED_FILE_BYTES {
                return Err(ExtractError::TooLarge);
            }
            Ok(Some(read_capped(entry)?))
        }
    }
}

/// Read a whole entry without trusting its declared size: never allocate up
/// front, stop at the cap + 1, and reject if the actual bytes exceed the cap.
fn read_capped<R: Read>(reader: R) -> Result<Vec<u8>, ExtractError> {
    let mut buf = Vec::new();
    std::io::copy(&mut reader.take(MAX_SERVED_FILE_BYTES + 1), &mut buf)?;
    if buf.len() as u64 > MAX_SERVED_FILE_BYTES {
        return Err(ExtractError::TooLarge);
    }
    Ok(buf)
}

/// Best-effort content-type guess from a file extension. This is the *raw*
/// guess; user-published files are served through [`served_mime`], which
/// neutralizes active-content types before they reach a browser.
pub fn mime_for(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or_default() {
        "css" => "text/css; charset=utf-8",
        "js" | "mjs" | "cjs" => "text/javascript; charset=utf-8",
        "json" | "map" => "application/json",
        "html" | "htm" => "text/html; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "wasm" => "application/wasm",
        "toml" => "application/toml",
        "md" | "txt" => "text/plain; charset=utf-8",
        "woff2" => "font/woff2",
        _ => "application/octet-stream",
    }
}

/// Content types a browser executes in the page's origin. Package contents are
/// author-controlled, so serving any of these as-is from the trusted registry
/// origin is a stored-XSS vector (H2).
fn is_active_content(mime: &str) -> bool {
    let base = mime.split(';').next().unwrap_or(mime).trim();
    matches!(
        base,
        "text/html" | "image/svg+xml" | "application/xhtml+xml"
    ) || base.contains("javascript")
}

/// The content-type a single package file is *served* with. HTML/SVG/XHTML/JS
/// are downgraded to `text/plain` so author-published markup or scripts cannot
/// execute from the registry origin (H2); everything else keeps its guess.
pub fn served_mime(path: &str) -> &'static str {
    let guessed = mime_for(path);
    if is_active_content(guessed) {
        "text/plain; charset=utf-8"
    } else {
        guessed
    }
}

/// Response headers for every `/v1/files` (unpkg-style) response. Beyond the
/// neutralized content-type, user content is served `inline` under a `sandbox`
/// CSP so it is inert as active content even if a browser guesses otherwise
/// (H2). `X-Content-Type-Options: nosniff` is applied globally by the router.
pub fn served_file_headers(path: &str) -> [(HeaderName, HeaderValue); 4] {
    [
        (
            header::CONTENT_TYPE,
            HeaderValue::from_static(served_mime(path)),
        ),
        (
            header::CACHE_CONTROL,
            HeaderValue::from_static(IMMUTABLE_CACHE),
        ),
        (
            header::CONTENT_DISPOSITION,
            HeaderValue::from_static("inline"),
        ),
        (
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static("sandbox"),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;
    use flate2::Compression;
    use flate2::write::GzEncoder;

    fn tiny_archive() -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let data = b"body { color: orange }";
        let mut header = tar::Header::new_gnu();
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "pkg/dist/style.css", data.as_slice())
            .unwrap();
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn tiny_zip() -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                "pkg/dist/style.css",
                zip::write::SimpleFileOptions::default(),
            )
            .unwrap();
        writer.write_all(b"body { color: orange }").unwrap();
        writer.finish().unwrap().into_inner()
    }

    /// A tar.gz whose only entry declares `declared` bytes but carries none:
    /// the size cap must trip on the header alone, before any allocation.
    fn lying_archive(declared: u64) -> Vec<u8> {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut header = tar::Header::new_gnu();
        header.set_path("pkg/huge.bin").unwrap();
        header.set_size(declared);
        header.set_mode(0o644);
        header.set_cksum();
        encoder.write_all(header.as_bytes()).unwrap();
        encoder.finish().unwrap()
    }

    #[test]
    fn extracts_by_relative_path() {
        let archive = tiny_archive();
        let found = extract_file(&archive, ArtifactFormat::TarGz, "dist/style.css").unwrap();
        assert_eq!(found.unwrap(), b"body { color: orange }");
        assert!(
            extract_file(&archive, ArtifactFormat::TarGz, "missing.css")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn zip_extracts_by_relative_path() {
        let archive = tiny_zip();
        let found = extract_file(&archive, ArtifactFormat::Zip, "dist/style.css").unwrap();
        assert_eq!(found.unwrap(), b"body { color: orange }");
        assert!(
            extract_file(&archive, ArtifactFormat::Zip, "missing.css")
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn rejects_oversized_declared_entry() {
        let archive = lying_archive(MAX_SERVED_FILE_BYTES + 1);
        let err = extract_file(&archive, ArtifactFormat::TarGz, "huge.bin").unwrap_err();
        assert!(matches!(err, ExtractError::TooLarge));
    }

    #[test]
    fn mime_guessing() {
        assert_eq!(mime_for("dist/style.css"), "text/css; charset=utf-8");
        assert_eq!(mime_for("mod.wasm"), "application/wasm");
        assert_eq!(mime_for("weird.bin"), "application/octet-stream");
    }
}
