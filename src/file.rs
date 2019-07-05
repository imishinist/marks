use crate::spec;
use std::io::{BufRead, BufReader, Write};
use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::{error, fs};
use termcolor::{BufferWriter, Color, ColorChoice, ColorSpec, WriteColor};

#[derive(Debug, Default)]
pub struct Line {
    str: String,
    mark: bool,
    ignore: bool,
}

impl Line {
    pub fn set(&mut self, mark_type: &spec::Type) {
        use spec::Type;
        match mark_type {
            Type::Mark => self.mark = true,
            Type::Ignore => self.ignore = true,
        }
    }

    pub fn marked(&self) -> bool {
        self.mark
    }

    pub fn ignored(&self) -> bool {
        self.ignore
    }
}

impl From<String> for Line {
    fn from(s: String) -> Self {
        Line {
            str: s,
            ..Default::default()
        }
    }
}

impl From<&str> for Line {
    fn from(s: &str) -> Self {
        Line {
            str: s.to_string(),
            ..Default::default()
        }
    }
}

impl Deref for Line {
    type Target = String;

    fn deref(&self) -> &Self::Target {
        &self.str
    }
}

impl DerefMut for Line {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.str
    }
}

#[derive(Default)]
pub struct Marked {
    filename: PathBuf,
    lines: Vec<Line>,

    pub marked: bool,
    pub ignore: bool,
}

impl Marked {
    pub fn read_from<P: AsRef<Path>>(path: P) -> Result<Self, Box<error::Error>> {
        let mut lines = Vec::with_capacity(100);

        let mut buf = String::new();
        let mut reader = BufReader::new(fs::File::open(&path)?);
        while reader.read_line(&mut buf)? > 0 {
            let line = buf.trim_end_matches('\n');
            lines.push(From::from(line));
            buf.clear();
        }
        Ok(Self {
            filename: path.as_ref().to_path_buf(),
            lines,
            ..Default::default()
        })
    }

    pub fn len(&self) -> usize {
        self.lines.len()
    }

    pub fn print(&self) -> Result<(), Box<error::Error>> {
        let bufwriter = BufferWriter::stdout(ColorChoice::Always);
        let mut buffer = bufwriter.buffer();

        let filename = self.filename.to_str().unwrap_or("");
        buffer.set_color(ColorSpec::new().set_fg(Some(Color::Yellow)))?;
        writeln!(&mut buffer, "{}", filename)?;
        buffer.reset()?;

        for (i, line) in self.lines.iter().enumerate() {
            let mark = line.mark;
            let ignored = line.ignore;
            if self.ignore || ignored {
                buffer.set_color(ColorSpec::new().set_fg(Some(Color::Cyan)))?;
                write!(&mut buffer, "{:>4}", i + 1)?;
                buffer.reset()?;
                write!(&mut buffer, "|")?;
                buffer.set_color(ColorSpec::new().set_fg(Some(Color::Rgb(128, 128, 128))))?;
                writeln!(&mut buffer, "{}", line.deref())?;
                buffer.reset()?;
            } else if self.marked || mark {
                buffer.set_color(ColorSpec::new().set_fg(Some(Color::Cyan)))?;
                write!(&mut buffer, "{:>4}", i + 1)?;
                buffer.reset()?;
                write!(&mut buffer, "|")?;
                buffer.set_color(ColorSpec::new().set_fg(Some(Color::Green)))?;
                writeln!(&mut buffer, "{}", line.deref())?;
                buffer.reset()?;
            } else {
                writeln!(&mut buffer, "{:>4}|{}", i + 1, line.deref())?;
            }
        }
        bufwriter.print(&buffer)?;
        Ok(())
    }
}

impl Deref for Marked {
    type Target = Vec<Line>;

    fn deref(&self) -> &Self::Target {
        &self.lines
    }
}

impl DerefMut for Marked {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.lines.as_mut()
    }
}

#[cfg(test)]
mod tests {
    use crate::file::Marked;
    use crate::spec;
    use core::borrow::Borrow;
    use std::path::PathBuf;

    #[test]
    fn marked_iteration() {
        let mut lines = Vec::with_capacity(100);
        lines.push(From::from("hogehoge"));
        lines.push(From::from("fugafuga"));
        lines.push(From::from("piyopiyo"));

        let mut file = Marked {
            filename: PathBuf::new(),
            lines,
            ..Default::default()
        };
        for line in file.iter_mut() {
            if line.eq(&"hogehoge".to_string()) {
                line.set(spec::Type::Ignore.borrow())
            }
            line.set(spec::Type::Mark.borrow())
        }

        for line in file.iter() {
            assert_eq!(line.mark, true);
        }
        assert_eq!(file[0].ignore, true);
        assert_eq!(file[1].ignore, false);
        assert_eq!(file[2].ignore, false);
    }
}
