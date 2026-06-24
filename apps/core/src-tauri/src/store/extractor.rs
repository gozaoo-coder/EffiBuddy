//! Extractor: unpack a downloaded `.tar.gz` or `.zip` archive into the
//! packages directory.

use anyhow::{anyhow, Context, Result};
use std::io::Read;
use std::path::PathBuf;

pub struct Extractor;

impl Extractor {
    pub fn extract(archive_bytes: &[u8], dest: &PathBuf, filename: &str) -> Result<()> {
        std::fs::create_dir_all(dest)?;
        if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
            Self::extract_targz(archive_bytes, dest)
        } else if filename.ends_with(".zip") {
            Self::extract_zip(archive_bytes, dest)
        } else {
            Err(anyhow!("unsupported archive type: {filename}"))
        }
    }

    fn extract_targz(bytes: &[u8], dest: &PathBuf) -> Result<()> {
        let decoder = flate2::read::GzDecoder::new(bytes);
        let mut archive = tar::Archive::new(decoder);
        archive.unpack(dest).context("unpack tar.gz")?;
        Ok(())
    }

    fn extract_zip(bytes: &[u8], dest: &PathBuf) -> Result<()> {
        let cursor = std::io::Cursor::new(bytes);
        let mut archive = zip::ZipArchive::new(cursor).context("open zip")?;
        for i in 0..archive.len() {
            let mut file = archive.by_index(i).context("read zip entry")?;
            let outpath = match file.enclosed_name() {
                Some(p) => dest.join(p),
                None => continue,
            };
            if file.is_dir() {
                std::fs::create_dir_all(&outpath)?;
            } else {
                if let Some(parent) = outpath.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                let mut outfile = std::fs::File::create(&outpath)?;
                std::io::copy(&mut file, &mut outfile)?;
            }
        }
        Ok(())
    }
}

#[allow(dead_code)]
fn _silence_unused_read_warning<R: Read>(_r: R) {}
