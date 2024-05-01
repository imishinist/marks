use std::fs::File;
use std::io::{BufRead, BufReader, Stdout, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use std::{env, error, fs, io};
use std::ops::Range;

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use once_cell::sync::Lazy;

use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use crossterm::event::KeyModifiers;
use ratatui::{prelude::*, text::Line, widgets::*};

use regex::Regex;
use sha2::digest::Digest;
use termcolor::{BufferWriter, ColorSpec, WriteColor};

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

    pub fn add(&mut self, line_no: u16) {
        match self {
            FileMarkSpec::All => {}
            FileMarkSpec::Partial(specs) => {
                specs.push(SpecType::Line(line_no));
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
    use termcolor::{Color as tColor, ColorChoice};
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
            buffer.set_color(ColorSpec::new().set_fg(Some(tColor::Cyan)))?;
            write!(&mut buffer, "{:>4}", line_no)?;
            buffer.reset()?;
            write!(&mut buffer, "|")?;
            buffer.set_color(ColorSpec::new().set_fg(Some(tColor::Green)))?;
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

    /// View file with special window
    View(ViewCommand),
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
        touch_file(&spec_file_path)?;

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

struct ViewApp {
    spec_file_path: PathBuf,

    source_lines: Vec<String>,
    spec: FileMarkSpec,

    source_line_no: u16,

    // top of the screen
    offset: u16,
    cursor_line_no: u16,
    line_padding: u16,
    height: u16,
}

impl ViewApp {
    fn new(source_file_path: PathBuf) -> Self {
        let spec_file_path = get_spec_file_path(&source_file_path);
        touch_file(&spec_file_path).expect("failed to touch spec file");

        let source_lines =
            Self::read_source_by_line(&source_file_path).expect("failed to read source file");
        let spec = parse_spec_file(&spec_file_path).expect("failed to parse spec file");
        let source_line_no = source_lines.len() as u16;
        Self {
            spec_file_path,
            source_lines,
            spec,
            source_line_no,
            offset: 0,
            cursor_line_no: 1,
            line_padding: 5,
            height: 80,
        }
    }

    fn inc_cursor(&mut self, count: u16) {
        self.cursor_line_no = self.cursor_line_no.saturating_add(count);
        if self.cursor_line_no > self.source_line_no {
            self.cursor_line_no = self.source_line_no;
        }

        if self.cursor_line_no >= self.offset + self.height - self.line_padding {
            let tmp = self.cursor_line_no + self.line_padding;
            self.offset = tmp.saturating_sub(self.height);
        }
    }

    fn dec_cursor(&mut self, count: u16) {
        self.cursor_line_no = self.cursor_line_no.saturating_sub(count);
        if self.cursor_line_no == 0 {
            self.cursor_line_no = 1;
        }

        if self.cursor_line_no < self.offset + self.line_padding + 1 {
            self.offset = (self.cursor_line_no - 1).saturating_sub(self.line_padding);
        }
    }

    fn run(source_file_path: PathBuf) -> anyhow::Result<()> {
        let mut terminal = init_terminal()?;
        let mut last_tick = Instant::now();
        let mut app = Self::new(source_file_path);
        let tick_rate = Duration::from_millis(16);
        loop {
            let _ = terminal.draw(|frame| app.ui(frame).unwrap());

            let timeout = tick_rate.saturating_sub(last_tick.elapsed());
            if event::poll(timeout)? {
                if let Event::Key(key) = event::read()? {
                    match key.code {
                        KeyCode::Char('q') => break,
                        KeyCode::Char('j') | KeyCode::Down => app.inc_cursor(1),
                        KeyCode::Char('k') | KeyCode::Up => app.dec_cursor(1),
                        KeyCode::Char('g') => {
                            app.cursor_line_no = 1;
                            app.dec_cursor(0);
                        },
                        KeyCode::Char('G') => {
                            app.cursor_line_no = app.source_line_no;
                            app.inc_cursor(0);
                        },
                        KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => app.inc_cursor(10),
                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => app.dec_cursor(10),
                        KeyCode::Char('m') => app.spec.add(app.cursor_line_no),
                        _ => {}
                    }
                }
            }

            if last_tick.elapsed() >= tick_rate {
                app.on_tick();
                last_tick = Instant::now();
            }
        }
        app.spec.optimize();
        write_spec_file(app.spec_file_path, &app.spec)?;

        restore_terminal()?;
        Ok(())
    }

    fn on_tick(&mut self) {}

    fn ui(&mut self, frame: &mut Frame) -> anyhow::Result<()> {
        let rect = frame.size();
        self.height = rect.height;
        frame.render_widget(self.paragraph(rect)?, rect);
        Ok(())
    }

    fn paragraph(&self, window_size: Rect) -> anyhow::Result<impl Widget + '_> {
        let offset = self.offset as usize;
        let line_range = (offset+1)..(offset + window_size.height as usize + 1);
        let text = self.mark_lines_by_spec(line_range, window_size.width);
        Ok(Paragraph::new(text))
    }

    fn mark_lines_by_spec(&self, line_range: Range<usize>, window_width: u16) -> Vec<Line> {
        let mut lines = vec![];

        let line_no_offset = line_range.start;
        let mut idx_range = line_range;
        idx_range.start = idx_range.start.saturating_sub(1);
        idx_range.end = idx_range.end.saturating_sub(1);
        if idx_range.end > self.source_lines.len() {
            idx_range.end = self.source_lines.len();
        }

        for (i, line) in self.source_lines[idx_range].iter().enumerate() {
            let line_no = line_no_offset + i;
            let mut line_no_style = Style::default();
            let mut style = Style::default();
            if line_no == self.cursor_line_no as usize {
                style = style.underlined();
            }
            if self.spec.match_line_no(line_no as u16) {
                line_no_style = line_no_style.fg(Color::Cyan);
                style = style.fg(Color::Green);
            }

            let rest = window_width - line.len() as u16 - 4 - 1;
            let line_no = Span::styled(format!("{:>4}", line_no), line_no_style);
            let padding = Span::styled("|", Style::default());
            let source = Span::styled(format!("{}{}", line, " ".repeat(rest as usize)), style);
            lines.push(Line::from(vec![line_no, padding, source]));
        }
        lines
    }

    fn read_source_by_line<P: AsRef<Path>>(source_file_path: P) -> anyhow::Result<Vec<String>> {
        let source_file = File::open(source_file_path)?;
        let mut reader = BufReader::new(source_file);
        let mut buf = String::new();

        let mut source_lines = vec![];
        while reader.read_line(&mut buf)? > 0 {
            let line = buf.trim_end_matches('\n').to_string();
            source_lines.push(line);
            buf.clear();
        }

        Ok(source_lines)
    }
}

#[derive(Args, Debug)]
struct ViewCommand {
    source: String,
}

impl ViewCommand {
    fn run(&self) -> anyhow::Result<()> {
        let source_file_path = PathBuf::from(&self.source);
        ViewApp::run(source_file_path)?;
        Ok(())
    }
}

fn init_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    terminal::enable_raw_mode()?;
    io::stdout().execute(EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(io::stdout()))
}

fn restore_terminal() -> io::Result<()> {
    terminal::disable_raw_mode()?;
    io::stdout().execute(LeaveAlternateScreen)?;
    Ok(())
}

fn init_logger() {
    simplelog::CombinedLogger::init(vec![
        simplelog::WriteLogger::new(
            simplelog::LevelFilter::Info,
            simplelog::Config::default(),
            File::create("/tmp/marks.log").unwrap(),
        ),
    ])
    .unwrap();
}

fn main() -> Result<(), Box<dyn error::Error>> {
    init_logger();
    let marks = MarksCommands::parse();
    match &marks.commands {
        Commands::Print(print) => print.run()?,
        Commands::Edit(edit) => edit.run()?,
        Commands::View(view) => view.run()?,
    }
    Ok(())
}
