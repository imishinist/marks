use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::{env, error, fs};

use clap::Parser;
use once_cell::sync::Lazy;
use regex::Regex;
use sha2::digest::Digest;
use termcolor::{BufferWriter, Color, ColorChoice, ColorSpec, WriteColor};

fn get_spec_file_dir() -> PathBuf {
    let home = env::var("HOME").expect("failed to get $HOME env");
    let data_home = env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{}/.local/share", home));
    PathBuf::from(data_home).join("marks")
}

fn get_spec_file_path<P: AsRef<Path>>(file_path: P) -> PathBuf {
    let file_path = fs::canonicalize(file_path).expect("failed to get current directory");

    let mut hasher = sha2::Sha256::new();
    hasher.update(file_path.as_os_str().as_encoded_bytes());
    let result = hasher.finalize();

    get_spec_file_dir().join(PathBuf::from(format!("{:x}", result)))
}

#[derive(Debug)]
enum FileMarkSpec {
    All,
    Partial(Vec<SpecType>),
}

impl FileMarkSpec {
    fn match_line_no(&self, line_no: u16) -> bool {
        match self {
            FileMarkSpec::All => true,
            FileMarkSpec::Partial(specs) => {
                for spec in specs.iter() {
                    match *spec {
                        SpecType::Line(no) if no == line_no => return true,
                        SpecType::Range(l, r) if l <= line_no && line_no < r => return true,
                        _ => continue,
                    }
                }
                false
            }
        }
    }
}

#[derive(Debug)]
enum SpecType {
    Line(u16),
    Range(u16, u16),
}

static NUM_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*(\d+)\s*$").unwrap());
static RANGE_REGEX: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s*(\d+)\s*-\s*(\d+)\s*$").unwrap());

fn parse_spec_file<P: AsRef<Path>>(file_path: P) -> anyhow::Result<FileMarkSpec> {
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
        if line.contains("-*- all -*-") {
            return Ok(FileMarkSpec::All);
        }

        let spec;
        if let Some(cap) = RANGE_REGEX.captures(line) {
            let from_str = &cap[1];
            let to_str = &cap[2];
            spec = SpecType::Range(from_str.parse()?, to_str.parse()?);
        } else if let Some(cap) = NUM_REGEX.captures(line) {
            let num_str = &cap[1];
            spec = SpecType::Line(num_str.parse()?);
        } else {
            return Err(anyhow::anyhow!("invalid spec format"));
        }
        specs.push(spec);
        buf.clear();
    }

    Ok(FileMarkSpec::Partial(specs))
}

fn print_file(file: &File, spec: &FileMarkSpec) -> anyhow::Result<()> {
    let writer = BufferWriter::stdout(ColorChoice::Always);
    let mut buffer = writer.buffer();

    let mut line_no = 0u16;
    let mut read_buf = String::new();
    let mut reader = BufReader::new(file);
    while reader.read_line(&mut read_buf)? > 0 {
        let line = read_buf.trim_end_matches('\n');

        line_no += 1;
        // color print
        if spec.match_line_no(line_no) {
            buffer.set_color(ColorSpec::new().set_fg(Some(Color::Cyan)))?;
            write!(&mut buffer, "{:>4}", line_no)?;
            buffer.reset()?;
            write!(&mut buffer, "|")?;
            buffer.set_color(ColorSpec::new().set_fg(Some(Color::Green)))?;
            writeln!(&mut buffer, "{}", line)?;
            buffer.reset()?;
        } else {
            writeln!(&mut buffer, "{:>4}|{}", line_no, line)?;
        }

        read_buf.clear();
    }
    writer.print(&buffer)?;
    Ok(())
}

fn touch_file<P: AsRef<Path>>(file_path: P) -> anyhow::Result<()> {
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

#[derive(Parser)]
#[command(author, version, about, long_about=None)]
#[command(propagate_version = true)]
struct MarksCommands {
    source: String,
}

fn main() -> Result<(), Box<dyn error::Error>> {
    let marks = MarksCommands::parse();

    let source_path = marks.source;
    let spec_file_path = get_spec_file_path(&source_path);
    touch_file(&spec_file_path)?;

    // parse spec file
    let spec = parse_spec_file(&spec_file_path)?;

    // print source file with color
    let source_file = File::open(source_path)?;
    print_file(&source_file, &spec)?;
    Ok(())
}
