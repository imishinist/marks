use std::fs::File;
use std::io::{BufRead, BufReader, Stdout, Write};
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};
use std::{env, error, fs, io};

use anyhow::Context;
use clap::{Args, Parser, Subcommand};
use once_cell::sync::Lazy;

use crossterm::event::KeyModifiers;
use crossterm::{
    event::{self, Event, KeyCode},
    terminal::{self, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use ratatui::{prelude::*, text::Line, widgets::*};

use regex::Regex;
use sha2::digest::Digest;
use termcolor::{BufferWriter, ColorSpec, WriteColor};
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

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

#[derive(Copy, Clone, Debug)]
struct FileMarkStatus {
    pub marked: u16,
    pub line_no: u16,
}

fn directory_status<P: AsRef<Path>>(dir_path: P) -> anyhow::Result<FileMarkStatus> {
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

fn file_status<P: AsRef<Path>>(file_path: P) -> anyhow::Result<FileMarkStatus> {
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

#[derive(Debug)]
enum FileMarkSpec {
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
enum SpecType {
    // 0-index
    Line(u16),
    // 0-index, [l, r)
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

fn print_file(file: &File, spec: &FileMarkSpec) -> anyhow::Result<()> {
    use termcolor::{Color as tColor, ColorChoice};
    let writer = BufferWriter::stdout(ColorChoice::Always);
    let mut buffer = writer.buffer();

    let mut line_offset = 0u16;
    let mut read_buf = String::new();
    let mut reader = BufReader::new(file);
    while reader.read_line(&mut read_buf)? > 0 {
        let line = read_buf.trim_end_matches('\n');
        let line_no = line_offset + 1;

        // color print
        if spec.match_line_offset(line_offset) {
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

        line_offset += 1;
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

    /// Show status of all sources
    Status(StatusCommand),
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

    #[arg(long, default_value_t = false, conflicts_with = "all")]
    reset: bool,

    #[arg(long, default_value_t = false, conflicts_with = "reset")]
    all: bool,
}

impl EditCommand {
    fn run(&self) -> anyhow::Result<()> {
        let spec_file_dir = get_spec_file_dir();
        let spec_file_path = get_spec_file_path(&self.source);
        touch_file(&spec_file_path)?;

        if self.reset {
            fs::remove_file(&spec_file_path)?;
            return Ok(());
        }

        if self.all {
            if spec_file_path.exists() {
                fs::remove_file(&spec_file_path)?;
            }

            fs::write(&spec_file_path, format!("{}\n", ALL_MAGIC))?;
            return Ok(());
        }

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

#[derive(Debug)]
enum InputMode {
    Normal,
    Editing,
}

struct ViewApp {
    spec_file_path: PathBuf,
    spec: FileMarkSpec,

    source_lines: Vec<String>,
    source_line_len: u16,

    // top of the screen
    // 0-index
    offset: u16,
    // 0-index
    cursor_line_offset: u16,

    source_view_padding_height: u16,
    source_view_height: u16,

    input_mode: InputMode,
    input: Input,

    grep_text: Option<String>,
}

impl ViewApp {
    fn new(source_file_path: PathBuf) -> Self {
        let spec_file_path = get_spec_file_path(&source_file_path);
        touch_file(&spec_file_path).expect("failed to touch spec file");

        let source_lines =
            Self::read_source_by_line(&source_file_path).expect("failed to read source file");
        let spec = parse_spec_file(&spec_file_path).expect("failed to parse spec file");
        let source_line_len = source_lines.len() as u16;
        Self {
            spec_file_path,
            spec,
            source_lines,
            source_line_len,

            offset: 0,
            cursor_line_offset: 0,
            source_view_padding_height: 5,
            source_view_height: 80,

            input_mode: InputMode::Normal,
            input: Input::default(),

            grep_text: None,
        }
    }

    fn update_offset(&mut self) {
        if self.cursor_line_offset + 1
            >= self.offset + self.source_view_height - self.source_view_padding_height
        {
            self.offset = (self.cursor_line_offset + 1 + self.source_view_padding_height)
                .saturating_sub(self.source_view_height);
        }
        if self.cursor_line_offset < self.offset + self.source_view_padding_height {
            self.offset = self
                .cursor_line_offset
                .saturating_sub(self.source_view_padding_height);
        }
    }

    fn jump_cursor(&mut self, index: u16) {
        self.cursor_line_offset = index.min(self.source_line_len);
        self.update_offset();
    }

    fn inc_cursor(&mut self, count: u16) {
        self.jump_cursor(self.cursor_line_offset.saturating_add(count));
    }

    fn dec_cursor(&mut self, count: u16) {
        self.jump_cursor(self.cursor_line_offset.saturating_sub(count));
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
                let handle_result = match app.input_mode {
                    InputMode::Normal => app.normal_mode_handler()?,
                    InputMode::Editing => app.editing_mode_handler()?,
                };
                if handle_result.is_none() {
                    break;
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

    fn jump_prev_matched_line(&mut self, needle: &str) {
        if let Some(idx) = self.prev_matched_index(needle) {
            self.jump_cursor(idx);
        }
    }

    fn jump_next_matched_line(&mut self, needle: &str) {
        if let Some(idx) = self.next_matched_index(needle) {
            self.jump_cursor(idx);
        }
    }

    fn prev_matched_index(&self, needle: &str) -> Option<u16> {
        if self.cursor_line_offset == 0 {
            return None;
        }
        let offset = self.cursor_line_offset as usize;
        for (idx, line) in self.source_lines[..offset].iter().rev().enumerate() {
            if line.contains(needle) {
                return Some((offset - idx - 1) as u16);
            }
        }
        None
    }

    fn next_matched_index(&self, needle: &str) -> Option<u16> {
        if self.cursor_line_offset + 1 >= self.source_line_len {
            return None;
        }

        let start_offset = self.cursor_line_offset as usize + 1;
        for (idx, line) in self.source_lines[start_offset..].iter().enumerate() {
            if line.contains(needle) {
                return Some((start_offset + idx) as u16);
            }
        }
        None
    }

    fn current_line_contains(&self, needle: &str) -> bool {
        self.source_lines[self.cursor_line_offset as usize].contains(needle)
    }

    fn normal_mode_handler(&mut self) -> anyhow::Result<Option<()>> {
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Char('q') => return Ok(None),
                KeyCode::Char('n') => {
                    if let Some(grep_text) = self.grep_text.as_ref() {
                        let grep_text = grep_text.clone();
                        self.jump_next_matched_line(&grep_text);
                    }
                }
                KeyCode::Char('N') => {
                    if let Some(grep_text) = self.grep_text.as_ref() {
                        let grep_text = grep_text.clone();
                        self.jump_prev_matched_line(&grep_text);
                    }
                }
                KeyCode::Char('j') | KeyCode::Down => self.inc_cursor(1),
                KeyCode::Char('k') | KeyCode::Up => self.dec_cursor(1),
                KeyCode::Char('g') => self.jump_cursor(0),
                KeyCode::Char('G') => self.jump_cursor(self.source_line_len.saturating_sub(1)),
                KeyCode::Char('d') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.inc_cursor(10)
                }
                KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.dec_cursor(10)
                }
                KeyCode::Char('m') => {
                    self.spec.add(self.cursor_line_offset);
                    self.inc_cursor(1);
                }
                KeyCode::Char('u') => {
                    self.spec.remove(self.cursor_line_offset);
                    self.inc_cursor(1);
                }
                KeyCode::Char('/') => {
                    self.input_mode = InputMode::Editing;
                }
                _ => {}
            }
        }
        Ok(Some(()))
    }

    fn editing_mode_handler(&mut self) -> anyhow::Result<Option<()>> {
        if let Event::Key(key) = event::read()? {
            match key.code {
                KeyCode::Enter => {
                    // search by input value
                    let input = self.input.to_string();
                    if !self.current_line_contains(&input) {
                        self.jump_next_matched_line(&input);
                    }

                    self.grep_text = Some(input);

                    self.input.reset();
                    self.input_mode = InputMode::Normal;
                }
                KeyCode::Backspace if self.input.value().is_empty() => {
                    // cancel
                    self.input.reset();
                    self.input_mode = InputMode::Normal;
                }
                _ => {
                    self.input.handle_event(&Event::Key(key));
                }
            }
        }
        Ok(Some(()))
    }

    fn on_tick(&mut self) {}

    fn ui(&mut self, frame: &mut Frame) -> anyhow::Result<()> {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(1), Constraint::Max(1)].as_ref())
            .split(frame.size());

        let rect = chunks[0];
        self.source_view_height = rect.height;
        frame.render_widget(self.paragraph(rect)?, rect);

        let rect = chunks[1];
        frame.render_widget(self.command_palette(rect), rect);
        if matches!(self.input_mode, InputMode::Editing) {
            let scroll = self.input.visual_scroll(rect.width as usize);
            frame.set_cursor(
                rect.x + (self.input.visual_cursor().max(scroll) - scroll) as u16 + 1,
                rect.y,
            );
        }
        Ok(())
    }

    fn command_palette(&self, window_size: Rect) -> impl Widget + '_ {
        let width = window_size.width;
        let scroll = self.input.visual_scroll(width as usize);

        let palette = match self.input_mode {
            InputMode::Normal => format!(":{}", self.input.value()),
            InputMode::Editing => format!("/{}", self.input.value()),
        };

        Paragraph::new(palette).scroll((0, scroll as u16))
    }

    fn paragraph(&self, window_size: Rect) -> anyhow::Result<impl Widget + '_> {
        let offset = self.offset as usize;
        let (height, width) = (window_size.height as usize, window_size.width);

        let text = self.mark_lines_by_spec(offset..(offset + height), width);
        Ok(Paragraph::new(text))
    }

    fn mark_lines_by_spec(&self, idx_range: Range<usize>, window_width: u16) -> Vec<Line> {
        let mut lines = vec![];

        let start_offset = idx_range.start;
        let mut idx_range = idx_range;
        if idx_range.end > self.source_lines.len() {
            idx_range.end = self.source_lines.len();
        }

        for (i, line) in self.source_lines[idx_range].iter().enumerate() {
            lines.push(self.mark_line_by_spec(start_offset + i, line, window_width));
        }
        lines
    }

    fn mark_line_by_spec<'a>(
        &'a self,
        line_offset: usize,
        line: &'a str,
        window_width: u16,
    ) -> Line {
        let mut line_no_style = Style::default();
        let mut style = Style::default();
        if line_offset == self.cursor_line_offset as usize {
            style = style.underlined();
        }
        let line_matched = self.spec.match_line_offset(line_offset as u16);
        if line_matched {
            line_no_style = line_no_style.fg(Color::Cyan);
            style = style.fg(Color::Green);
        }

        // line_no length and padding length = 4 + 1
        let mut spans = Vec::new();

        spans.push(Span::styled(
            format!("{:>4}", line_offset + 1),
            line_no_style,
        ));
        spans.push(Span::styled("|", Style::default()));

        let mut cursor = 0;
        if let Some(grep_text) = self.grep_text.as_ref() {
            let grep_text_len = grep_text.len();
            while let Some(idx) = line[cursor..].find(grep_text) {
                // first character to highlight character
                spans.push(Span::styled(&line[cursor..(cursor + idx)], style));

                // highlight matched characters
                let mut style = style;
                style = style.bg(Color::Gray);
                if !line_matched {
                    style = style.fg(Color::Black);
                }
                spans.push(Span::styled(
                    &line[(cursor + idx)..(cursor + idx + grep_text_len)],
                    style,
                ));

                cursor += idx + grep_text_len;
            }

            if cursor != 0 {
                spans.push(Span::styled(&line[cursor..], style));

                let rest_size = window_width.saturating_sub(line.len() as u16 + 4 + 1);
                if rest_size > 0 {
                    spans.push(Span::styled(" ".repeat(rest_size as usize), style));
                }
                return Line::from(spans);
            }
        }

        let rest = window_width.saturating_sub(line.len() as u16 + 4 + 1);
        spans.push(Span::styled(
            format!("{}{}", line, " ".repeat(rest as usize)),
            style,
        ));
        Line::from(spans)
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

#[derive(Args, Debug)]
struct StatusCommand {
    sources: Vec<String>,
}

impl StatusCommand {
    fn run(&self) -> anyhow::Result<()> {
        for source in &self.sources {
            let file_path = PathBuf::from(source);

            let status = if file_path.is_dir() {
                directory_status(&file_path)?
            } else {
                file_status(&file_path)?
            };

            println!(
                "{}\t{}\t{:.1}%\t{}",
                source,
                status.marked,
                status.marked as f64 / status.line_no as f64 * 100.0,
                status.line_no
            );
        }
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
    simplelog::CombinedLogger::init(vec![simplelog::WriteLogger::new(
        simplelog::LevelFilter::Info,
        simplelog::Config::default(),
        File::create("/tmp/marks.log").unwrap(),
    )])
    .unwrap();
}

fn main() -> Result<(), Box<dyn error::Error>> {
    init_logger();
    let marks = MarksCommands::parse();
    match &marks.commands {
        Commands::Print(print) => print.run()?,
        Commands::Edit(edit) => edit.run()?,
        Commands::View(view) => view.run()?,
        Commands::Status(status) => status.run()?,
    }
    Ok(())
}
