use std::{fs, io::Read, path::Path, time::UNIX_EPOCH};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExeRecord {
    pub sha256: String,
    pub size: u64,
    pub mtime_ms: u128,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ExeCheck {
    NoRecord,
    Ok,
    Refreshed(ExeRecord),
    Changed { expected: String, actual: String },
}

/// The config key is constructed from the canonical exe path (case-insensitive for Windows).
pub fn record_key(canonical: &Path) -> String {
    format!("exe_record:{}", canonical.to_string_lossy().to_lowercase())
}

fn stat(path: &Path) -> Result<(u64, u128), String> {
    let md = fs::metadata(path).map_err(|e| e.to_string())?;
    let mtime = md
        .modified()
        .map_err(|e| e.to_string())?
        .duration_since(UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_millis();
    Ok((md.len(), mtime))
}

fn hash_file(path: &Path) -> Result<String, String> {
    let mut file = fs::File::open(path).map_err(|e| e.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = file.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}

pub fn make_record(path: &Path) -> Result<ExeRecord, String> {
    let (size, mtime_ms) = stat(path)?;
    Ok(ExeRecord {
        sha256: hash_file(path)?,
        size,
        mtime_ms,
    })
}

pub fn check(path: &Path, stored: Option<&str>) -> Result<ExeCheck, String> {
    let Some(raw) = stored else {
        return Ok(ExeCheck::NoRecord);
    };
    let rec: ExeRecord =
        serde_json::from_str(raw).map_err(|e| format!("exe_record_corrupt:{e}"))?;
    let (size, mtime_ms) = stat(path)?;
    let actual = hash_file(path)?;
    if actual == rec.sha256 {
        if size == rec.size && mtime_ms == rec.mtime_ms {
            Ok(ExeCheck::Ok)
        } else {
            Ok(ExeCheck::Refreshed(ExeRecord {
                sha256: actual,
                size,
                mtime_ms,
            }))
        }
    } else {
        Ok(ExeCheck::Changed {
            expected: rec.sha256,
            actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, thread::sleep, time::Duration};

    fn tmp(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("fusion-exe-{}-{name}", std::process::id()))
    }

    #[test]
    fn no_record_means_no_check() {
        let p = tmp("none");
        fs::write(&p, b"x").unwrap();
        assert_eq!(check(&p, None).unwrap(), ExeCheck::NoRecord);
        fs::remove_file(p).ok();
    }

    #[test]
    fn unchanged_passes() {
        let p = tmp("ok");
        fs::write(&p, b"aaaa").unwrap();
        let json = serde_json::to_string(&make_record(&p).unwrap()).unwrap();
        assert_eq!(check(&p, Some(&json)).unwrap(), ExeCheck::Ok);
        fs::remove_file(p).ok();
    }

    #[test]
    fn replaced_file_is_rejected() {
        let p = tmp("chg");
        fs::write(&p, b"aaaa").unwrap();
        let json = serde_json::to_string(&make_record(&p).unwrap()).unwrap();
        fs::write(&p, b"bbbbbbbb").unwrap();
        assert!(matches!(
            check(&p, Some(&json)).unwrap(),
            ExeCheck::Changed { .. }
        ));
        fs::remove_file(p).ok();
    }

    #[test]
    fn same_content_new_mtime_is_refreshed() {
        let p = tmp("touch");
        fs::write(&p, b"aaaa").unwrap();
        let json = serde_json::to_string(&make_record(&p).unwrap()).unwrap();
        sleep(Duration::from_millis(50));
        fs::write(&p, b"aaaa").unwrap();
        assert!(matches!(
            check(&p, Some(&json)).unwrap(),
            ExeCheck::Refreshed(_)
        ));
        fs::remove_file(p).ok();
    }

    #[test]
    fn corrupt_record_fails_closed() {
        let p = tmp("bad");
        fs::write(&p, b"x").unwrap();
        assert!(check(&p, Some("garbage")).is_err());
        fs::remove_file(p).ok();
    }

    #[test]
    fn forged_mtime_and_size_is_still_rejected() {
        let p = tmp("forge");
        fs::write(&p, b"original-content").unwrap();
        let record = make_record(&p).unwrap();
        let json = serde_json::to_string(&record).unwrap();

        // Write different content with the exact same length
        fs::write(&p, b"modified-content").unwrap();
        let res = check(&p, Some(&json)).unwrap();
        assert!(matches!(res, ExeCheck::Changed { .. }));
        fs::remove_file(p).ok();
    }
}
