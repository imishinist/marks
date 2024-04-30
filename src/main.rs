use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::{env, error, fs, io};
use std::process::Command;
use anyhow::Context;

use clap::{Args, Parser, Subcommand};
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
    pub fn match_line_no(&self, line_no: u16) -> bool {
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

    pub fn optimize(&mut self) {
        match self {
            FileMarkSpec::All => {},
            FileMarkSpec::Partial(specs) => {
                let tmp = Self::rebuild_partial_specs(specs);
                *specs = tmp;
            }
        }
    }

    fn rebuild_partial_specs(specs: &Vec<SpecType>) -> Vec<SpecType> {
        let mut line_no_map = vec![false; u16::MAX as usize];
        for spec in specs {
            match *spec {
                SpecType::Line(line_no) => {
                    line_no_map[line_no as usize] = true;
                }
                SpecType::Range(l, r) => {
                    for line_no in l..r {
                        line_no_map[line_no as usize] = true;
                    }
                }
            }
        }

        let mut result = vec![];
        let mut left_value: Option<usize> = None;
        for (line_no, b) in line_no_map.iter().enumerate() {
            if left_value.is_none() && *b {
                left_value = Some(line_no);
            } else if left_value.is_some() && !*b {
                let left = left_value.take().unwrap();
                if line_no - left == 1 {
                    result.push(SpecType::Line(left as u16));
                } else {
                    result.push(SpecType::Range(left as u16, line_no as u16));
                }
            }
        }

        result
    }
}

#[derive(Debug)]
enum SpecType {
    Line(u16),
    Range(u16, u16),
}

const ALL_MAGIC: &str = "-*- all -*-";

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
        if line.contains(ALL_MAGIC) {
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

fn write_spec_file<P: AsRef<Path>>(file_path: P, spec: &FileMarkSpec) -> anyhow::Result<()> {
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
                    SpecType::Line(no) => {
                        buf.write_str(&format!("{}\n", no))?;
                    }
                    SpecType::Range(l, r) => {
                        buf.write_str(&format!("{}-{}\n", l, r))?;
                    }
                }
            }
        }
    }
    fs::write(file_path, buf)?;
    Ok(())
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
    #[command(subcommand)]
    commands: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Print file with color
    Print(PrintCommand),

    /// Edit spec file
    Edit(EditCommand),
}

#[derive(Args, Debug)]
struct PrintCommand {
    source: String,
}

impl PrintCommand {
    fn run(&self) -> anyhow::Result<()> {
        let source_path = &self.source;
        let spec_file_path = get_spec_file_path(source_path);
        touch_file(&spec_file_path)?;

        // parse spec file
        let spec = parse_spec_file(&spec_file_path)?;

        // print source file with color
        let source_file = File::open(source_path)?;
        print_file(&source_file, &spec)?;

        Ok(())
    }
}

fn edit_with_editor<P: AsRef<Path>>(file_path: P) -> anyhow::Result<()> {
    let file_path = file_path.as_ref();

    let editor = env::var("EDITOR").context("EDITOR variable not found")?;

    let mut command = Command::new(editor);
    command.arg(file_path);
    let child = command.spawn()?;
    child.wait_with_output()?;

    Ok(())
}

#[derive(Args, Debug)]
struct EditCommand {
    source: String,
}

impl EditCommand {
    fn run(&self) -> anyhow::Result<()> {
        let spec_file_dir = get_spec_file_dir();
        let spec_file_path = get_spec_file_path(&self.source);

        let mut tmp = tempfile::NamedTempFile::new_in(&spec_file_dir)?;
        let mut spec_file = File::open(&spec_file_path)?;
        io::copy(&mut spec_file, &mut tmp)?;

        edit_with_editor(tmp.path())?;

        let mut spec = parse_spec_file(tmp.path())?;
        spec.optimize();

        let tmp = tempfile::NamedTempFile::new_in(&spec_file_dir)?;
        write_spec_file(tmp.path(), &spec)?;

        fs::rename(tmp.path(), &spec_file_path)?;
        Ok(())
    }
}

fn main() -> Result<(), Box<dyn error::Error>> {
    let marks = MarksCommands::parse();
    match &marks.commands {
        Commands::Print(print) => print.run()?,
        Commands::Edit(edit) => edit.run()?,
    }
    Ok(())
}
