#[macro_use]
extern crate clap;
#[macro_use]
extern crate lazy_static;

use clap::Arg;
use regex::Regex;
use std::default::Default;
use std::error;
use std::fmt;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use termcolor::{BufferWriter, Color, ColorChoice, ColorSpec, WriteColor};

struct StatSummary {
    all: usize,
    ignored: usize,
    marked: usize,
}

impl StatSummary {
    fn calc(lines: &MarkedFile) -> Self {
        let mut count = 0;
        let mut ignore_count = 0;
        for line in lines.iter() {
            if line.mark {
                count += 1;
            }
            if line.ignore {
                ignore_count += 1;
            }
        }
        StatSummary {
            all: lines.len(),
            ignored: ignore_count,
            marked: count,
        }
    }
}

impl fmt::Display for StatSummary {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let ratio = 100u64 * (self.marked as u64) / (self.all as u64);
        write!(
            f,
            "lines: {}, marked: {}, ignored: {}, ratio: {}%\n",
            self.all, self.marked, self.ignored, ratio,
        )?;
        let ratio = 100u64 * (self.marked as u64) / (self.all - self.ignored) as u64;
        write!(
            f,
            "lines: {}, marked: {}, ratio: {}%",
            self.all - self.ignored,
            self.marked,
            ratio
        )
    }
}

enum MarkStat {
    Number(usize),
    Range(usize, usize),
    IgnoreNumber(usize),
    IgnoreRange(usize, usize),
    RegexpRange(Regex, Regex),
}

impl MarkStat {
    fn parse_line(line: &String) -> Result<Option<Self>, Box<error::Error>> {
        lazy_static! {
            static ref comment: Regex = Regex::new(r"^(#.*)$").unwrap();
            static ref num: Regex = Regex::new(r"^(\d+)$").unwrap();
            static ref range: Regex = Regex::new(r"^(\d+)\s(\d+)$").unwrap();
            static ref ignore_num: Regex = Regex::new(r"^ignore:\s*(\d+)$").unwrap();
            static ref ignore_range: Regex = Regex::new(r"^ignore:\s*(\d+)\s(\d+)$").unwrap();
        }

        if let Some(_cap) = comment.captures(line) {
            return Ok(None);
        }

        if let Some(cap) = num.captures(line) {
            let num_str = &cap[1];
            return Ok(Some(MarkStat::Number(num_str.parse()?)));
        }

        if let Some(cap) = range.captures(line) {
            let from_str = &cap[1];
            let to_str = &cap[2];
            return Ok(Some(MarkStat::Range(from_str.parse()?, to_str.parse()?)));
        }

        if let Some(cap) = ignore_num.captures(line) {
            let num_str = &cap[1];
            return Ok(Some(MarkStat::IgnoreNumber(num_str.parse()?)));
        }

        if let Some(cap) = ignore_range.captures(line) {
            let from_str = &cap[1];
            let to_str = &cap[2];
            return Ok(Some(MarkStat::IgnoreRange(
                from_str.parse()?,
                to_str.parse()?,
            )));
        }

        Err(From::from("spec invalid"))
    }
}

#[derive(Default)]
struct Line {
    str: String,
    mark: bool,
    ignore: bool,
}

#[derive(Default)]
struct MarkedFile {
    filename: PathBuf,
    lines: Vec<Line>,
}

impl MarkedFile {
    fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<error::Error>> {
        let mut lines = Vec::with_capacity(100);

        let mut buf = String::new();
        let mut reader = BufReader::new(fs::File::open(&path)?);
        while reader.read_line(&mut buf)? > 0 {
            let line = buf.trim_end_matches('\n');
            lines.push(Line {
                str: line.to_string(),
                ..Default::default()
            });
            buf.clear();
        }
        Ok(MarkedFile {
            filename: path.as_ref().to_path_buf(),
            lines,
            ..Default::default()
        })
    }

    fn len(&self) -> usize {
        self.lines.len()
    }

    fn mark(&mut self, line_number: usize) {
        if line_number >= self.lines.len() {
            return;
        }
        self.lines[line_number].mark = true;
    }

    fn unmark(&mut self, line_number: usize) {
        if line_number >= self.lines.len() {
            return;
        }
        self.lines[line_number].mark = false;
    }

    fn ignore(&mut self, line_number: usize) {
        if line_number >= self.lines.len() {
            return;
        }
        self.lines[line_number].ignore = true;
    }

    fn iter(&self) -> MarkedFileIterator {
        MarkedFileIterator {
            lines: &self,
            pos: 0,
        }
    }

    fn print(&self) -> Result<(), Box<error::Error>> {
        let bufwriter = BufferWriter::stdout(ColorChoice::Always);
        let mut buffer = bufwriter.buffer();

        let filename = self.filename.to_str().unwrap_or("");
        buffer.set_color(ColorSpec::new().set_fg(Some(Color::Yellow)))?;
        writeln!(&mut buffer, "{}", filename)?;
        buffer.reset()?;
        for (i, line) in self.lines.iter().enumerate() {
            let mark = line.mark;
            let ignored = line.ignore;
            let line = &line.str;
            if mark {
                buffer.set_color(ColorSpec::new().set_fg(Some(Color::Cyan)))?;
                write!(&mut buffer, "{:>4}", i + 1)?;
                buffer.reset()?;
                write!(&mut buffer, "|")?;
                buffer.set_color(ColorSpec::new().set_fg(Some(Color::Green)))?;
                writeln!(&mut buffer, "{}", line)?;
                buffer.reset()?;
            } else if ignored {
                buffer.set_color(ColorSpec::new().set_fg(Some(Color::Cyan)))?;
                write!(&mut buffer, "{:>4}", i + 1)?;
                buffer.reset()?;
                write!(&mut buffer, "|")?;
                buffer.set_color(ColorSpec::new().set_fg(Some(Color::Rgb(128, 128, 128))))?;
                writeln!(&mut buffer, "{}", line)?;
                buffer.reset()?;
            } else {
                writeln!(&mut buffer, "{:>4}|{}", i + 1, line)?;
            }
        }
        bufwriter.print(&buffer)?;
        Ok(())
    }
}

struct MarkedFileIterator<'a> {
    lines: &'a MarkedFile,
    pos: usize,
}

impl<'a> Iterator for MarkedFileIterator<'a> {
    type Item = &'a Line;

    fn next(&mut self) -> Option<Self::Item> {
        self.pos += 1;
        if self.pos - 1 < self.lines.lines.len() {
            Some(&self.lines.lines[self.pos - 1])
        } else {
            None
        }
    }
}

fn parse_mark<P: AsRef<Path>>(mark_spec_path: P) -> Result<Vec<MarkStat>, Box<error::Error>> {
    let mut mark_spec = Vec::new();
    let mut reader = BufReader::new(fs::File::open(mark_spec_path)?);
    let mut buf = String::new();
    while reader.read_line(&mut buf)? > 0 {
        let line = buf.trim_end_matches('\n').to_string();
        buf.clear();
        if line.is_empty() {
            continue;
        }
        let spec = MarkStat::parse_line(&line)?;
        match spec {
            Some(spec) => mark_spec.push(spec),
            None => continue,
        }
    }
    Ok(mark_spec)
}

fn main() -> Result<(), Box<error::Error>> {
    let app = app_from_crate!()
        .arg(Arg::from_usage("[stat] --stat 'print stat'"))
        .arg(Arg::from_usage("-s --source <OPT> 'target source file'"))
        .arg(Arg::from_usage("-c --spec <OPT> 'specification file"));

    let matches = app.get_matches();

    let source_path = matches.value_of("source").expect("specify source option");
    let mark_spec_path = matches.value_of("spec").expect("specify spec option");

    let mut lines = MarkedFile::from_file(source_path)?;
    let mark_spec = parse_mark(mark_spec_path)?;

    let mut ignores = Vec::new();

    for mark in mark_spec {
        match mark {
            MarkStat::Number(num) => {
                lines.mark(num - 1);
            }
            MarkStat::Range(from, to) => {
                for l in from..=to {
                    lines.mark(l - 1);
                }
            }
            MarkStat::IgnoreNumber(num) => {
                ignores.push(num - 1);
            }
            MarkStat::IgnoreRange(from, to) => {
                for l in from..=to {
                    ignores.push(l - 1);
                }
            }
            MarkStat::RegexpRange(from, to) => {
                unimplemented!("regex");
            }
        }
    }
    for ignore_line in ignores.iter() {
        lines.unmark(*ignore_line);
        lines.ignore(*ignore_line);
    }

    if matches.is_present("stat") {
        println!("{}", StatSummary::calc(&lines));
        return Ok(());
    }

    lines.print()?;

    Ok(())
}
