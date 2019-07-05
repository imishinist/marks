use crate::spec;
use crate::spec::{Marking, Type};
use regex::Regex;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::{error, fs};

impl From<&String> for Type {
    fn from(line: &String) -> Self {
        lazy_static! {
            static ref IGNORE: Regex = Regex::new(r"'ignore$").unwrap();
        }
        match IGNORE.captures(line) {
            Some(_cap) => Type::Ignore,
            None => Type::Mark,
        }
    }
}

#[derive(Default, Debug)]
pub struct Parser {
    spec_file: PathBuf,
    lines: Vec<String>,
}

impl Parser {
    pub fn new<P: AsRef<Path>>(spec_file: P) -> Self {
        Parser {
            spec_file: spec_file.as_ref().to_path_buf(),
            ..Default::default()
        }
    }

    pub fn parse(&mut self) -> Result<Vec<spec::Marking>, Box<error::Error>> {
        let mut markings = Vec::with_capacity(self.lines.len());

        for line in self.lines.iter() {
            match Parser::parse_line(line) {
                Ok(m) => markings.push(m),
                Err(_e) => continue,
            }
        }

        Ok(markings)
    }

    fn parse_line(line: &String) -> Result<spec::Marking, Box<error::Error>> {
        lazy_static! {
            static ref IGNORE: Regex = Regex::new(r"\s*('ignore)$").unwrap();
            static ref COMMENT: Regex = Regex::new(r"\s*(#.*)$").unwrap();
            static ref PATH_REGEX: Regex = Regex::new(r"([a-zA-Z0-9_\-/.]+)").unwrap();
            static ref NUM: Regex = Regex::new(r":\s*(\d+)\s*$").unwrap();
            static ref RANGE: Regex = Regex::new(r":\s*(\d+)\s*-\s*(\d+)\s*$").unwrap();
            static ref RE: Regex = Regex::new(r":\s*/(.*)/\s*$").unwrap();
        }

        let mark_type = From::from(line);

        // remove comment from line
        let line = line.clone();
        let line = &IGNORE.replace_all(&line, "").into_owned();
        let line = &COMMENT.replace_all(&line, "").into_owned();

        let spec;
        if let Some(cap) = NUM.captures(line) {
            let num_str = &cap[1];
            spec = Some(spec::Spec::Line(num_str.parse()?));
        } else if let Some(cap) = RANGE.captures(line) {
            let from_str = &cap[1];
            let to_str = &cap[2];
            spec = Some(spec::Spec::Range(from_str.parse()?, to_str.parse()?));
        } else if let Some(cap) = RE.captures(line) {
            let re = &cap[1];
            spec = Some(spec::Spec::Regex(re.to_string()));
        } else {
            spec = None;
        }

        let path;
        if let Some(cap) = PATH_REGEX.captures(line) {
            path = cap[0].to_string();
        } else {
            return Err(From::from("path required"));
        }

        let target;
        match spec {
            Some(s) => target = spec::Target::FileSpec(path, s),
            None => {
                use std::fs::metadata;
                let md = metadata(path.as_str())?;
                if md.is_dir() {
                    target = spec::Target::Dir(path);
                } else {
                    target = spec::Target::File(path);
                }
            }
        }

        Ok(Marking::new(target, mark_type))
    }

    pub fn read_file(&mut self) -> Result<(), Box<error::Error>> {
        let mut buf = String::new();
        let mut reader = BufReader::new(fs::File::open(&self.spec_file)?);
        while reader.read_line(&mut buf)? > 0 {
            let line = buf.trim_end_matches('\n');
            self.lines.push(line.to_string());
            buf.clear();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::spec::{Spec, Target, Type};

    #[test]
    fn parse_test() {
        let mut parser = Parser::new("");

        parser.lines.push("src/".to_string());
        parser.lines.push("src/main.rs".to_string());
        parser.lines.push("src/main.rs:10".to_string());
        parser.lines.push("src/main.rs:10-20".to_string());
        parser.lines.push("src/main.rs:/hoge/".to_string());
        parser.lines.push("src/main.rs:10 'ignore".to_string());
        parser.lines.push("src/main.rs:10-20 'ignore".to_string());
        parser.lines.push("src/ 'ignore".to_string());
        parser.lines.push("src/ # comment".to_string());

        let want = vec![
            spec::Marking::new(Target::Dir("src/".to_string()), Type::Mark),
            spec::Marking::new(Target::File("src/main.rs".to_string()), Type::Mark),
            spec::Marking::new(
                Target::FileSpec("src/main.rs".to_string(), Spec::Line(10)),
                Type::Mark,
            ),
            spec::Marking::new(
                Target::FileSpec("src/main.rs".to_string(), Spec::Range(10, 20)),
                Type::Mark,
            ),
            spec::Marking::new(
                Target::FileSpec("src/main.rs".to_string(), Spec::Regex("hoge".to_string())),
                Type::Mark,
            ),
            spec::Marking::new(
                Target::FileSpec("src/main.rs".to_string(), Spec::Line(10)),
                Type::Ignore,
            ),
            spec::Marking::new(
                Target::FileSpec("src/main.rs".to_string(), Spec::Range(10, 20)),
                Type::Ignore,
            ),
            spec::Marking::new(Target::Dir("src/".to_string()), Type::Ignore),
            spec::Marking::new(Target::Dir("src/".to_string()), Type::Mark),
        ];

        assert_eq!(parser.parse().unwrap(), want);
    }
}
