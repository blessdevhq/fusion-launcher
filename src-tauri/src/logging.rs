use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::Serialize;

#[derive(Serialize)]
struct LogLine<'a> {
    timestamp: String,
    event: &'a str,
    fields: Vec<(&'a str, &'a str)>,
}

pub fn log_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("logs")
}

pub fn log_file_path(data_dir: &Path) -> PathBuf {
    log_dir(data_dir).join("fusion-launcher.log")
}

pub fn initialize(data_dir: &Path) {
    let _ = fs::create_dir_all(log_dir(data_dir));
    log_event(data_dir, "app_start", &[]);
}

pub fn log_event(data_dir: &Path, event: &str, fields: &[(&str, &str)]) {
    let path = log_file_path(data_dir);
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let line = LogLine {
        timestamp: Utc::now().to_rfc3339(),
        event,
        fields: fields.to_vec(),
    };
    let Ok(json) = serde_json::to_string(&line) else {
        return;
    };
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{json}");
    }
}

pub fn tail_log(data_dir: &Path, limit: usize) -> Vec<String> {
    let path = log_file_path(data_dir);
    let Ok(file) = OpenOptions::new().read(true).open(path) else {
        return Vec::new();
    };
    let reader = BufReader::new(file);
    let mut lines = reader.lines().map_while(Result::ok).collect::<Vec<_>>();
    if lines.len() > limit {
        lines.drain(0..lines.len() - limit);
    }
    lines
}
