#[macro_use]
extern crate clap;

use clap::Arg;
use regex::Regex;
use std::default::Default;
use std::error;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use termcolor::{BufferWriter, Color, ColorChoice, ColorSpec, WriteColor};

enum MarkStat {
    Number(usize),
    Range(usize, usize),
    RegexpRange(Regex, Regex),
}

impl MarkStat {
    fn parse_line(line: &String) -> Result<Self, Box<error::Error>> {
        let nums = line.split_whitespace().collect::<Vec<_>>();

        if nums.len() == 1 {
            let num = nums[0].parse()?;
            return Ok(MarkStat::Number(num));
        } else if nums.len() == 2 {
            let from = nums[0].parse()?;
            let to = nums[1].parse()?;
            return Ok(MarkStat::Range(from, to));
        }
        Err(From::from("spec invalid"))
    }
}

#[derive(Default)]
struct Line {
    str: String,
    mark: bool,
}

#[derive(Default)]
struct MarkedFile {
    lines: Vec<Line>,
}

impl MarkedFile {
    fn from_file<P: AsRef<Path>>(path: P) -> Result<Self, Box<error::Error>> {
        let mut lines = Vec::with_capacity(100);

        let mut buf = String::new();
        let mut reader = BufReader::new(fs::File::open(path)?);
        while reader.read_line(&mut buf)? > 0 {
            let line = buf.trim_end_matches('\n');
            lines.push(Line {
                str: line.to_string(),
                ..Default::default()
            });
            buf.clear();
        }
        Ok(MarkedFile {
            lines,
            ..Default::default()
        })
    }

    fn mark(&mut self, line_number: usize) {
        if line_number >= self.lines.len() {
            return;
        }
        self.lines[line_number].mark = true;
    }

    fn iter(&self) -> MarkedFileIterator {
        MarkedFileIterator {
            lines: &self,
            pos: 0,
        }
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
        mark_spec.push(MarkStat::parse_line(&line)?);
    }
    Ok(mark_spec)
}

fn main() -> Result<(), Box<error::Error>> {
    let app = app_from_crate!()
        .arg(Arg::from_usage("-s --source <OPT> 'target source file'"))
        .arg(Arg::from_usage("-c --spec <OPT> 'specification file"));

    let matches = app.get_matches();

    let source_path = matches.value_of("source").expect("specify source option");
    let mark_spec_path = matches.value_of("spec").expect("specify spec option");

    let mut lines = MarkedFile::from_file(source_path)?;
    let mark_spec = parse_mark(mark_spec_path)?;

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
            MarkStat::RegexpRange(from, to) => {
                unimplemented!("regex");
            }
        }
    }

    let bufwriter = BufferWriter::stdout(ColorChoice::Always);
    let mut buffer = bufwriter.buffer();
    for (i, line) in lines.iter().enumerate() {
        let mark = line.mark;
        let line = &line.str;
        if mark {
            buffer.set_color(ColorSpec::new().set_fg(Some(Color::Green)))?;
            writeln!(&mut buffer, "{:>4}|{}", i + 1, line)?;
            buffer.reset()?;
        } else {
            writeln!(&mut buffer, "{:>4}|{}", i + 1, line)?;
        }
    }
    bufwriter.print(&buffer)?;

    Ok(())
}
