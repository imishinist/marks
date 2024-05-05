use once_cell::sync::Lazy;
use regex::Regex;
use sha2::Digest;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::{env, fs};

pub fn get_spec_file_dir() -> PathBuf {
    let home = env::var("HOME").expect("failed to get $HOME env");
    let data_home = env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{}/.local/share", home));
    PathBuf::from(data_home).join("marks")
}

pub fn get_spec_file_path<P: AsRef<Path>>(file_path: P) -> PathBuf {
    let file_path = fs::canonicalize(file_path).expect("failed to get current directory");

    let mut hasher = sha2::Sha256::new();
    hasher.update(file_path.as_os_str().as_encoded_bytes());
    let result = hasher.finalize();

    get_spec_file_dir().join(PathBuf::from(format!("{:x}", result)))
}

pub fn touch_file<P: AsRef<Path>>(file_path: P) -> anyhow::Result<()> {
    let file_path = file_path.as_ref();
    if file_path.exists() {
        return Ok(());
    }
    fs::create_dir_all(
        file_path
            .parent()
            .ok_or(anyhow::anyhow!("failed to get parent directory"))?,
    )?;
    File::create(file_path)?;
    Ok(())
}

#[derive(Copy, Clone, Debug)]
pub struct FileMarkStatus {
    pub marked: u16,
    pub line_no: u16,
}

#[derive(Debug)]
pub enum FileMarkSpec {
    All,
    Partial(Vec<SpecType>),
}

impl FileMarkSpec {
    pub fn match_line_offset(&self, line_offset: u16) -> bool {
        match self {
            FileMarkSpec::All => true,
            FileMarkSpec::Partial(specs) => {
                for spec in specs.iter() {
                    match *spec {
                        SpecType::Line(offset) if offset == line_offset => return true,
                        SpecType::Range(l, r) if l <= line_offset && line_offset < r => {
                            return true
                        }
                        _ => continue,
                    }
                }
                false
            }
        }
    }

    pub fn add(&mut self, line_offset: u16) {
        match self {
            FileMarkSpec::All => {}
            FileMarkSpec::Partial(specs) => {
                specs.push(SpecType::Line(line_offset));
            }
        }
    }

    pub fn remove(&mut self, line_offset: u16) {
        match self {
            FileMarkSpec::All => {
                let before = SpecType::Range(0, line_offset);
                let after = SpecType::Range(line_offset + 1, u16::MAX);
                *self = FileMarkSpec::Partial(vec![before, after]);
            }
            FileMarkSpec::Partial(specs) => {
                let idx = specs.iter().enumerate().find_map(|(idx, spec)| match spec {
                    SpecType::Line(offset) if *offset == line_offset => Some(idx),
                    SpecType::Range(l, r) if *l <= line_offset && line_offset < *r => Some(idx),
                    _ => None,
                });
                if let Some(idx) = idx {
                    let spec = specs.get(idx).unwrap();
                    match spec {
                        SpecType::Line(_) => {
                            specs.remove(idx);
                        }
                        SpecType::Range(l, r) => {
                            let l = *l;
                            let r = *r;

                            if r - l == 1 {
                                specs[idx] = SpecType::Line(l);
                                return;
                            }

                            if l == line_offset {
                                specs[idx] = SpecType::Range(l + 1, r);
                            } else if r == line_offset + 1 {
                                specs[idx] = SpecType::Range(l, r - 1);
                            } else {
                                specs[idx] = SpecType::Range(l, line_offset);
                                specs.insert(idx + 1, SpecType::Range(line_offset + 1, r));
                            }
                        }
                    }
                }
            }
        }
    }

    pub fn optimize(&mut self) {
        match self {
            FileMarkSpec::All => {}
            FileMarkSpec::Partial(specs) => {
                let tmp = Self::rebuild_partial_specs(specs);
                *specs = tmp;
            }
        }
    }

    fn rebuild_partial_specs(specs: &Vec<SpecType>) -> Vec<SpecType> {
        let mut line_offset_map = vec![false; u16::MAX as usize];
        for spec in specs {
            match *spec {
                SpecType::Line(line_offset) => {
                    line_offset_map[line_offset as usize] = true;
                }
                SpecType::Range(l, r) => {
                    for line_offset in l..r {
                        line_offset_map[line_offset as usize] = true;
                    }
                }
            }
        }

        let mut result = vec![];
        let mut left_value: Option<usize> = None;
        for (line_offset, b) in line_offset_map.iter().enumerate() {
            if left_value.is_none() && *b {
                left_value = Some(line_offset);
            } else if left_value.is_some() && !*b {
                let left = left_value.take().unwrap();
                if line_offset - left == 1 {
                    result.push(SpecType::Line(left as u16));
                } else {
                    result.push(SpecType::Range(left as u16, line_offset as u16));
                }
            }
        }
        if let Some(left) = left_value {
            result.push(SpecType::Range(left as u16, u16::MAX));
        }

        result
    }
}

#[derive(Debug)]
pub enum SpecType {
    // 0-index
    Line(u16),
    // 0-index, [l, r)
    Range(u16, u16),
}

pub fn directory_status<P: AsRef<Path>>(dir_path: P) -> anyhow::Result<FileMarkStatus> {
    let dir_path = dir_path.as_ref();
    let mut marked = 0u16;
    let mut line_no = 0u16;
    for entry in fs::read_dir(dir_path)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let status = directory_status(&path)?;
            marked += status.marked;
            line_no += status.line_no;
        } else {
            let status = file_status(&path)?;
            marked += status.marked;
            line_no += status.line_no;
        }
    }
    Ok(FileMarkStatus { marked, line_no })
}

pub fn file_status<P: AsRef<Path>>(file_path: P) -> anyhow::Result<FileMarkStatus> {
    let file_path = file_path.as_ref();
    let spec_file_path = get_spec_file_path(file_path);
    touch_file(&spec_file_path)?;

    let spec = parse_spec_file(&spec_file_path)?;

    let mut line_no = 0u16;
    let mut reader = BufReader::new(File::open(file_path)?);
    let mut buf = String::new();
    let mut marked = 0u16;
    while reader.read_line(&mut buf)? > 0 {
        line_no += 1;
        if spec.match_line_offset(line_no) {
            marked += 1;
        }
        buf.clear();
    }

    Ok(FileMarkStatus { marked, line_no })
}

pub const ALL_MAGIC: &str = "-*- all -*-";

static NUM_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*(\d+)\s*$").unwrap());
static RANGE_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*(\d+)\s*-\s*(\d+)\s*$").unwrap());

pub fn parse_spec_file<P: AsRef<Path>>(file_path: P) -> anyhow::Result<FileMarkSpec> {
    let mut specs = Vec::new();

    let mut buf = String::new();
    let mut reader = BufReader::new(fs::File::open(file_path)?);
    while reader.read_line(&mut buf)? > 0 {
        let line = buf.trim_end_matches('\n');

        if line.is_empty() {
            continue;
        }

        // comment
        if line.starts_with('#') {
            continue;
        }

        // all magic comment
        if line.contains(ALL_MAGIC) {
            return Ok(FileMarkSpec::All);
        }

        let spec;
        if let Some(cap) = RANGE_REGEX.captures(line) {
            let from_str = &cap[1];
            let to_str = &cap[2];
            let from: u16 = from_str.parse()?;
            let to: u16 = to_str.parse()?;

            spec = SpecType::Range(from.saturating_sub(1), to.saturating_sub(1));
        } else if let Some(cap) = NUM_REGEX.captures(line) {
            let num_str = &cap[1];
            let num: u16 = num_str.parse()?;
            spec = SpecType::Line(num.saturating_sub(1));
        } else {
            return Err(anyhow::anyhow!("invalid spec format"));
        }
        specs.push(spec);
        buf.clear();
    }

    Ok(FileMarkSpec::Partial(specs))
}

pub fn write_spec_file<P: AsRef<Path>>(file_path: P, spec: &FileMarkSpec) -> anyhow::Result<()> {
    use std::fmt::Write as fmtWrite;
    let mut buf = String::new();
    match spec {
        FileMarkSpec::All => {
            buf.write_str(ALL_MAGIC)?;
            buf.write_char('\n')?;
        }
        FileMarkSpec::Partial(specs) => {
            for spec in specs {
                match spec {
                    SpecType::Line(offset) => {
                        buf.write_str(&format!("{}\n", offset.saturating_add(1)))?;
                    }
                    SpecType::Range(l, r) => {
                        buf.write_str(&format!(
                            "{}-{}\n",
                            l.saturating_add(1),
                            r.saturating_add(1)
                        ))?;
                    }
                }
            }
        }
    }
    fs::write(file_path, buf)?;
    Ok(())
}
