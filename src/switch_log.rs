use crate::config::AppConfig;
use std::fs;
use std::path::PathBuf;
use std::time::UNIX_EPOCH;
use windows::Win32::System::SystemInformation::GetLocalTime;

const MAX_LOG_ENTRIES: usize = 500;

#[derive(Debug, Clone)]
pub struct SwitchLogEntry {
    pub timestamp: String,
    pub process_name: String,
    pub from_device: String,
    pub to_device: String,
    pub monitor_name: String,
    pub volume_percent: Option<u8>,
    pub exe_path: String,
}

pub fn log_path() -> PathBuf {
    AppConfig::config_path().with_file_name("switch.log")
}

pub fn now_local_timestamp() -> String {
    unsafe {
        let st = GetLocalTime();
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond
        )
    }
}

fn sanitize_field(s: &str) -> String {
    s.replace(['\r', '\n', '\t'], " ").replace('|', "/")
}

fn format_log_line(entry: &SwitchLogEntry) -> String {
    let vol_col = entry
        .volume_percent
        .map(|v| format!("{v}%"))
        .unwrap_or_default();
    format!(
        "{} | {} | {} | {} | {} | {} | {}",
        sanitize_field(&entry.timestamp),
        sanitize_field(&entry.process_name),
        sanitize_field(&entry.from_device),
        sanitize_field(&entry.to_device),
        sanitize_field(&entry.monitor_name),
        vol_col,
        sanitize_field(&entry.exe_path)
    )
}

fn parse_log_line(line: &str) -> Option<SwitchLogEntry> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let parts: Vec<&str> = trimmed.split('|').map(str::trim).collect();
    if parts.len() < 5 {
        return None;
    }
    let volume_percent = parts
        .get(5)
        .and_then(|v| v.strip_suffix('%'))
        .and_then(|n| n.parse::<u8>().ok());
    let exe_path = parts.get(6).copied().unwrap_or_default().to_string();

    Some(SwitchLogEntry {
        timestamp: parts[0].to_string(),
        process_name: parts[1].to_string(),
        from_device: parts[2].to_string(),
        to_device: parts[3].to_string(),
        monitor_name: parts[4].to_string(),
        volume_percent,
        exe_path,
    })
}

pub fn append_switch_log(entry: &SwitchLogEntry) {
    let path = log_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let line = format_log_line(entry);

    let mut lines: Vec<String> = fs::read_to_string(&path)
        .map(|s| {
            s.lines()
                .filter(|l| !l.trim().is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    lines.push(line);

    if lines.len() > MAX_LOG_ENTRIES {
        let drop_count = lines.len() - MAX_LOG_ENTRIES;
        lines.drain(0..drop_count);
    }

    let mut out = lines.join("\n");
    out.push('\n');
    let _ = fs::write(&path, out);
}

pub fn read_switch_logs(max_entries: usize) -> Vec<SwitchLogEntry> {
    let path = log_path();
    let Ok(content) = fs::read_to_string(&path) else {
        return Vec::new();
    };

    let mut entries = Vec::new();
    for line in content.lines().rev() {
        let Some(entry) = parse_log_line(line) else {
            continue;
        };
        entries.push(entry);
        if entries.len() >= max_entries {
            break;
        }
    }
    entries
}

pub fn clear_switch_logs() -> Result<(), String> {
    let path = log_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(&path, "").map_err(|e| e.to_string())
}

pub fn open_log_in_editor() {
    let path = log_path();
    if !path.exists() {
        if let Some(parent) = path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&path, "");
    }
    let _ = std::process::Command::new("notepad.exe")
        .arg(&path)
        .spawn();
}

/// 返回日志文件当前的轻量签名（长度 + 修改时间戳纳秒），供 UI 轮询检测变更
pub fn log_signature() -> (u64, u128) {
    let Ok(meta) = fs::metadata(log_path()) else {
        return (0, 0);
    };
    let len = meta.len();
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    (len, mtime)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_log_line_with_volume_and_sanitization() {
        let entry = SwitchLogEntry {
            timestamp: "2026-11-14 00:35:00".to_string(),
            process_name: "chrome.exe".to_string(),
            from_device: "扬声器 | Realtek".to_string(),
            to_device: "显示器音频 (NVIDIA)".to_string(),
            monitor_name: "DISPLAY2".to_string(),
            volume_percent: Some(65),
            exe_path: r"C:\Program Files\Google\Chrome\Application\chrome.exe".to_string(),
        };
        let line = format_log_line(&entry);
        let parsed = parse_log_line(&line).expect("should parse formatted log line");
        assert_eq!(parsed.timestamp, "2026-11-14 00:35:00");
        assert_eq!(parsed.process_name, "chrome.exe");
        assert_eq!(parsed.from_device, "扬声器 / Realtek");
        assert_eq!(parsed.to_device, "显示器音频 (NVIDIA)");
        assert_eq!(parsed.monitor_name, "DISPLAY2");
        assert_eq!(parsed.volume_percent, Some(65));
        assert_eq!(
            parsed.exe_path,
            r"C:\Program Files\Google\Chrome\Application\chrome.exe"
        );
    }

    #[test]
    fn parse_log_line_without_volume() {
        let entry = SwitchLogEntry {
            timestamp: "2026-11-14 00:36:00".to_string(),
            process_name: "potplayer.exe".to_string(),
            from_device: "系统默认".to_string(),
            to_device: "耳机".to_string(),
            monitor_name: "DISPLAY1".to_string(),
            volume_percent: None,
            exe_path: String::new(),
        };
        let parsed = parse_log_line(&format_log_line(&entry)).expect("should parse line");
        assert_eq!(parsed.volume_percent, None);
        assert_eq!(parsed.exe_path, "");
    }
}
