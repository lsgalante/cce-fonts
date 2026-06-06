use std::process::Command;

#[derive(Debug, Clone)]
pub struct FontEntry {
    pub family: String,
    pub style: String,
    pub file: String,
}

pub fn fetch_fonts() -> Vec<FontEntry> {
    let output = match Command::new("fc-list")
        .arg("--format=%{family}\\t%{style}\\t%{file}\\n")
        .output()
    {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => return Vec::new(),
    };

    let mut fonts: Vec<FontEntry> = output
        .lines()
        .filter_map(|line| {
            let parts: Vec<&str> = line.splitn(3, '\t').collect();
            if parts.len() == 3 {
                let family = parts[0].split(',').next().unwrap_or(parts[0]).trim().to_string();
                let style = parts[1].split(',').next().unwrap_or(parts[1]).trim().to_string();
                Some(FontEntry {
                    family,
                    style,
                    file: parts[2].trim().to_string(),
                })
            } else {
                None
            }
        })
        .collect();

    fonts.sort_by(|a, b| a.family.to_lowercase().cmp(&b.family.to_lowercase()));
    fonts.dedup_by(|a, b| a.family == b.family && a.style == b.style);
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
                        count += (e_val - s_val + 1) as usize;
                    }
                } else if let Ok(_) = u32::from_str_radix(range, 16) {
                    count += 1;
                }
            }
            count
        }
        None => 0,
    }
}
