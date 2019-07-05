#[macro_use]
extern crate clap;

use clap::Arg;
use core::borrow::Borrow;
use marks::file::Marked;
use marks::parse::Parser;
use std::error;
use std::fmt;

struct StatSummary {
    all: usize,
    ignored: usize,
    marked: usize,
}

impl From<Marked> for StatSummary {
    fn from(m: Marked) -> Self {
        let mut count = 0;
        let mut ignore_count = 0;
        for line in m.iter() {
            if line.marked() {
                count += 1;
            }
            if line.ignored() {
                ignore_count += 1;
            }
        }
        StatSummary {
            all: m.len(),
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

fn main() -> Result<(), Box<error::Error>> {
    let app = app_from_crate!()
        .arg(Arg::from_usage("[stat] --stat 'print stat'"))
        .arg(Arg::from_usage("-s --source <OPT> 'target source file'"))
        .arg(Arg::from_usage("-c --spec <OPT> 'specification file"));

    let matches = app.get_matches();

    let source_path = matches.value_of("source").expect("specify source option");
    let mark_spec_path = matches.value_of("spec").expect("specify spec option");

    let mut parser = Parser::new(mark_spec_path);
    parser.read_file()?;
    let mut file = Marked::read_from(source_path)?;
    let specs = parser.parse()?;

    for (i, line) in file.iter_mut().enumerate() {
        for s in specs.iter() {
            use marks::spec::{Spec, Target};
            match s.target {
                Target::Dir(ref _path) => unimplemented!("dir is not implemented"),
                Target::File(ref _path) => unimplemented!("file is not implemented"),
                Target::FileSpec(ref _path, ref spec) => match spec {
                    Spec::Line(num) if *num == i + 1 => line.set(s.mark_type.borrow()),
                    Spec::Line(_num) => continue,
                    Spec::Range(from, to) if (*from..=*to).contains(&(i + 1)) => {
                        line.set(s.mark_type.borrow());
                    }
                    Spec::Range(_from, _to) => continue,
                    Spec::Regex(_re) => unimplemented!("regex is not implemented"),
                },
            }
        }
    }
    file.print()?;

    Ok(())
}
