//! unpkg-style edge serving of package contents: extract one file from a
//! stored artifact so the web can consume packages without installing them.

use std::io::Read;

use anyhow::Result;
use flate2::read::GzDecoder;

/// Find `pkg/<rel_path>` inside a tar.gz artifact.
pub fn extract_file(archive: &[u8], rel_path: &str) -> Result<Option<Vec<u8>>> {
    let mut tar = tar::Archive::new(GzDecoder::new(archive));
    let want = format!("{}/{rel_path}", zed_interfaces::paths::ARCHIVE_ROOT);
    for entry in tar.entries()? {
        let mut entry = entry?;
        if entry.path()?.to_string_lossy() == want {
            let mut buf = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut buf)?;
            return Ok(Some(buf));
        }
    }
    Ok(None)
}

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

#[cfg(test)]
mod tests {
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

    #[test]
    fn extracts_by_relative_path() {
        let archive = tiny_archive();
        let found = extract_file(&archive, "dist/style.css").unwrap();
        assert_eq!(found.unwrap(), b"body { color: orange }");
        assert!(extract_file(&archive, "missing.css").unwrap().is_none());
    }

    #[test]
    fn mime_guessing() {
        assert_eq!(mime_for("dist/style.css"), "text/css; charset=utf-8");
        assert_eq!(mime_for("mod.wasm"), "application/wasm");
        assert_eq!(mime_for("weird.bin"), "application/octet-stream");
    }
}
