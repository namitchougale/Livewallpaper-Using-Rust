use crate::geometry::Fit;
use std::{
    fs, io,
    path::{Path, PathBuf},
};
pub const STOCK: [&str; 3] = ["Polar Aurora", "Ember Dunes", "Midnight Tide"];
pub fn mp4(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .is_some_and(|s| s.eq_ignore_ascii_case("mp4"))
}
pub fn same_file(a: &Path, b: &Path) -> bool {
    a == b || a.to_string_lossy().to_lowercase() == b.to_string_lossy().to_lowercase()
}
#[derive(Clone, Debug, PartialEq)]
pub enum Choice {
    Stock(usize),
    Video(PathBuf),
}
impl Choice {
    pub fn title(&self) -> String {
        match self {
            Self::Stock(i) => STOCK[*i].into(),
            Self::Video(p) => p
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
        }
    }
}
#[derive(Debug, Default)]
pub struct Catalog {
    pub files: Vec<PathBuf>,
    pub fit: Fit,
    pub selected: usize,
}
impl Catalog {
    pub fn add(&mut self, path: PathBuf) -> Option<usize> {
        if !mp4(&path) {
            return None;
        }
        if let Some(i) = self.files.iter().position(|p| same_file(p, &path)) {
            return Some(STOCK.len() + i);
        }
        self.files.push(path);
        Some(STOCK.len() + self.files.len() - 1)
    }
    pub fn choice(&self, row: usize) -> Option<Choice> {
        if row < STOCK.len() {
            Some(Choice::Stock(row))
        } else {
            self.files
                .get(row - STOCK.len())
                .cloned()
                .map(Choice::Video)
        }
    }
    pub fn index(&self, c: &Choice) -> Option<usize> {
        match c {
            Choice::Stock(i) => Some(*i),
            Choice::Video(p) => self
                .files
                .iter()
                .position(|f| same_file(p, f))
                .map(|i| STOCK.len() + i),
        }
    }
    pub fn load(dir: &Path, sample: Option<PathBuf>) -> Self {
        let mut c = Self::default();
        if let Ok(data) = fs::read_to_string(dir.join("library-v1.txt")) {
            let mut lines = data.lines();
            if lines.next() == Some("Flow library 1") {
                let row = lines
                    .next()
                    .and_then(|s| s.parse::<usize>().ok())
                    .unwrap_or(0);
                c.fit = Fit::parse(lines.next().unwrap_or("Fill"));
                for line in lines {
                    c.add(line.into());
                }
                if c.choice(row).is_some() {
                    c.selected = row;
                }
            }
        } else {
            c.selected = fs::read_to_string(dir.join("scene"))
                .ok()
                .and_then(|s| s.trim().parse::<usize>().ok())
                .filter(|&i| i < STOCK.len())
                .unwrap_or(0);
            if let Ok(path) = fs::read_to_string(dir.join("video.txt")) {
                if let Some(row) = c.add(path.trim_end_matches(['\r', '\n']).into()) {
                    c.selected = row;
                }
            }
            if let Some(p) = sample.filter(|p| p.is_file()) {
                c.add(p);
            }
        }
        c
    }
    pub fn save(&self, dir: &Path) -> io::Result<()> {
        fs::create_dir_all(dir)?;
        let mut data = format!("Flow library 1\n{}\n{}\n", self.selected, self.fit.name());
        for p in &self.files {
            data.push_str(&p.to_string_lossy());
            data.push('\n');
        }
        let tmp = dir.join(format!("library-{}.tmp", std::process::id()));
        fs::write(&tmp, data)?;
        fs::rename(tmp, dir.join("library-v1.txt"))
    }
}
pub fn dialog_paths(list: &[u16]) -> Vec<PathBuf> {
    let mut entries = list
        .split(|&c| c == 0)
        .take_while(|s| !s.is_empty())
        .map(|s| PathBuf::from(String::from_utf16_lossy(s)));
    let Some(first) = entries.next() else {
        return Vec::new();
    };
    let names: Vec<_> = entries.collect();
    if names.is_empty() {
        vec![first]
    } else {
        names.into_iter().map(|n| first.join(n)).collect()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_mp4_and_deduplicate() {
        let mut c = Catalog::default();
        assert_eq!(c.add("Test Folder/Lake.MP4".into()), Some(3));
        assert_eq!(c.add("Test Folder/lake.mp4".into()), Some(3));
        assert!(c.add("lake.mp4.txt".into()).is_none());
        assert!(c.add("a.mov".into()).is_none());
    }
    #[test]
    fn roundtrip_and_old_settings() {
        let dir = std::env::temp_dir().join(format!("flow-library-test-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("video.txt"), "wallpapers/ಕನ್ನಡ 夜.MP4").unwrap();
        let mut c = Catalog::load(&dir, None);
        assert_eq!(c.selected, 3);
        c.fit = Fit::Fit;
        c.add("C:/my clips/beach & snow.mp4".into());
        c.save(&dir).unwrap();
        let d = Catalog::load(&dir, None);
        assert_eq!(d.files, c.files);
        assert_eq!(d.selected, 3);
        assert_eq!(d.fit, Fit::Fit);
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn dialog_single_multi_and_cancel() {
        let parse = |s: &str| dialog_paths(&s.encode_utf16().collect::<Vec<_>>());
        assert_eq!(
            parse("C:/my clips/a.MP4\0\0"),
            vec![PathBuf::from("C:/my clips/a.MP4")]
        );
        assert_eq!(
            parse("C:/my clips\0夜.mp4\0a b.mp4\0\0"),
            vec![
                PathBuf::from("C:/my clips").join("夜.mp4"),
                PathBuf::from("C:/my clips").join("a b.mp4")
            ]
        );
        assert!(parse("\0\0").is_empty());
    }
}
