use std::process::Command;

#[derive(Debug, Clone)]
pub struct FontEntry {
    pub family: String,
    pub style: String,
    pub file: String,
    /// The face's index within `file` (non-zero only in a .ttc collection).
    pub index: u32,
}

pub fn fetch_fonts() -> Vec<FontEntry> {
    let output = match Command::new("fc-list")
        .arg("--format=%{family}\\t%{style}\\t%{index}\\t%{file}\\n")
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => return Vec::new(),
    };

    let mut fonts: Vec<FontEntry> = output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(4, '\t').collect();
            if parts.len() == 4 {
                let family = parts[0].split(',').next().unwrap_or(parts[0]).trim().to_string();
                let style = parts[1].split(',').next().unwrap_or(parts[1]).trim().to_string();
                Some(FontEntry {
                    family,
                    style,
                    index: parts[2].trim().parse().unwrap_or(0),
                    file: parts[3].trim().to_string(),
                })
            } else {
                None
            }
        })
        .collect();

    // Deduped by the caller, which first drops the faces it cannot render.
    fonts.sort_by_key(|a| a.family.to_lowercase());
    fonts
}

pub fn is_user_font(file: &str) -> bool {
    file.starts_with("/home/") || file.contains(".local/share/fonts") || file.contains(".fonts")
}

pub fn count_chars(file: &str) -> usize {
    let output = Command::new("fc-query")
        .arg("--format=%{charset}")
        .arg(file)
        .output()
        .ok();

    match output {
        Some(o) => {
            let s = String::from_utf8_lossy(&o.stdout);
            let mut count = 0usize;
            for range in s.split_whitespace() {
                if let Some((start, end)) = range.split_once('-') {
                    if let (Ok(s_val), Ok(e_val)) = (u32::from_str_radix(start, 16), u32::from_str_radix(end, 16)) {
                        // Guard against malformed/reversed ranges (end < start) — an unsigned
                        // subtraction there panics and crashes the whole app.
                        if e_val >= s_val {
                            count += (e_val - s_val + 1) as usize;
                        }
                    }
                } else if u32::from_str_radix(range, 16).is_ok() {
                    count += 1;
                }
            }
            count
        }
        None => 0,
    }
}
