use std::fmt::Arguments;

#[cfg(target_os = "macos")]
const MAX_LOG_BYTES: u64 = 1024 * 1024;

pub fn log(message: Arguments<'_>) {
    #[cfg(target_os = "macos")]
    if std::env::var_os("M4D_OFFLINE").is_none() {
        if let Some(home) = std::env::var_os("HOME") {
            let directory = std::path::PathBuf::from(home).join("Library/Logs");
            let path = directory.join("m4d.log");
            if std::fs::create_dir_all(&directory).is_ok() {
                if std::fs::metadata(&path)
                    .map(|metadata| metadata.len() > MAX_LOG_BYTES)
                    .unwrap_or(false)
                {
                    let _ = std::fs::rename(&path, directory.join("m4d.1.log"));
                }
                if let Ok(mut file) = std::fs::OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&path)
                {
                    use std::io::Write;
                    let _ = writeln!(file, "{message}");
                    return;
                }
            }
        }
    }
    eprintln!("{message}");
}

#[macro_export]
macro_rules! m4d_log {
    ($($arg:tt)*) => {
        $crate::logging::log(format_args!($($arg)*))
    };
}
