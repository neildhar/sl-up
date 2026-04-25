use std::collections::HashMap;
use std::env;
use std::io::{self, Read, Write};
use std::process::{exit, Command, Stdio};

const HELP: &str = "\
hg-sl-up [OPTIONS] [SMARTLOG_OPTIONS] -- [UP_OR_REBASE_OPTIONS]

select commit with keyboard from smartlog and update/rebase to it

    Use up and down arrow keys to select previous and next commit
    respectively. Use left and right arrow keys to select previous and
    next bookmark respectively on a selected commit. Hit Enter to
    update to the selected commit or bookmark. Hit P to update to the
    parent of the selected commit instead. Hit Q, CTRL-C or Esc
    to exit without updating.

    Hit R to select a commit to rebase. Move to a different commit and
    hit Enter to rebase onto that commit. Hit P to rebase onto its
    parent instead.

    SMARTLOG_OPTIONS are options that are passed to sl smartlog.
    UP_OR_REBASE_OPTIONS are options that are passed to sl up or rebase.

    For example:

        hg-sl-up --stat -- --clean

    shows the stats for each commit (sl smartlog --stat) and performs
    a clean update (sl up --clean).

OPTIONS can be any of:
 --help     shows this help listing";

const PREFIX_CHARS: [char; 9] = [' ', '│', '╭', '╮', '╯', '╰', '╷', '─', '~'];
const SMARTLOG_METADATA_TEMPLATE: &str = "\
{node}\
{if(github_pull_request_number, ' {label(\"sl.branch\", \"#{github_pull_request_number}\")}')}\
{remotenames % ' {label(\"sl.remote\", remotename)}'}\
{bookmarks % ' {label(\"sl.book\", bookmark)}'}\\n";

fn main() {
    if let Err(err) = run() {
        eprintln!("{err}");
        exit(1);
    }
}

fn run() -> Result<(), String> {
    let argv: Vec<String> = env::args().skip(1).collect();
    if matches!(argv.first().map(String::as_str), Some("--help" | "help")) {
        println!("{HELP}");
        return Ok(());
    }

    let (smartlog_args, command_args) = split_argv(&argv);
    let term = TerminalGuard::enter()?;
    let output = run_sl_smartlog(&smartlog_args)?;
    let mut state = AppState::new(output)?;
    state.render()?;

    let stdin = io::stdin();
    let mut input = stdin.lock();
    let mut buf = [0_u8; 64];

    loop {
        let n = input.read(&mut buf).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }

        if let Some(action) = state.handle_input(&buf[..n])? {
            drop(term);
            return run_action(action, &command_args);
        }
    }

    Ok(())
}

fn split_argv(args: &[String]) -> (Vec<String>, Vec<String>) {
    match args.iter().position(|arg| arg == "--") {
        Some(i) => (args[..i].to_vec(), args[i + 1..].to_vec()),
        None => (args.to_vec(), Vec::new()),
    }
}

fn run_sl_smartlog(args: &[String]) -> Result<Vec<String>, String> {
    let output = Command::new("sl")
        .arg("--color")
        .arg("always")
        .arg("smartlog")
        .args(args)
        .output()
        .map_err(|e| e.to_string())?;

    let mut lines = smartlog_lines(&output.stdout);
    if !has_template_arg(args) {
        let metadata = run_sl_smartlog_metadata(args)?;
        append_metadata_to_changesets(&mut lines, &metadata);
    }

    Ok(lines)
}

fn smartlog_lines(stdout: &[u8]) -> Vec<String> {
    let mut lines: Vec<String> = String::from_utf8_lossy(stdout)
        .replace("\r\n", "\n")
        .split('\n')
        .map(str::to_owned)
        .collect();

    if let Some(i) = lines
        .iter()
        .position(|line| strip_ansi(line).contains("hint["))
    {
        lines.truncate(i);
    }

    while matches!(lines.last(), Some(line) if line.is_empty()) {
        lines.pop();
    }

    lines
}

fn run_sl_smartlog_metadata(args: &[String]) -> Result<HashMap<String, String>, String> {
    let output = Command::new("sl")
        .args(smartlog_metadata_command_args(args))
        .output()
        .map_err(|e| e.to_string())?;
    Ok(metadata_by_node(&smartlog_lines(&output.stdout)))
}

fn smartlog_metadata_command_args(args: &[String]) -> Vec<String> {
    let mut command_args = vec![
        "--color".to_owned(),
        "always".to_owned(),
        "smartlog".to_owned(),
    ];
    command_args.extend(metadata_passthrough_args(args));
    command_args.extend(["-T".to_owned(), SMARTLOG_METADATA_TEMPLATE.to_owned()]);
    command_args
}

fn metadata_passthrough_args(args: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        if matches!(
            arg.as_str(),
            "--stat" | "--commit-info" | "-p" | "--patch" | "-g" | "--git" | "-G" | "--graph"
        ) {
            i += 1;
        } else if arg == "-T" || arg == "--template" {
            i += 2;
        } else if arg.starts_with("-T") || arg.starts_with("--template=") {
            i += 1;
        } else {
            out.push(arg.clone());
            i += 1;
        }
    }
    out
}

fn has_template_arg(args: &[String]) -> bool {
    args.iter().any(|arg| {
        arg == "-T"
            || arg == "--template"
            || arg.starts_with("-T")
            || arg.starts_with("--template=")
    })
}

fn metadata_by_node(lines: &[String]) -> HashMap<String, String> {
    lines
        .iter()
        .filter_map(|line| metadata_for_line(line))
        .collect()
}

fn metadata_for_line(line: &str) -> Option<(String, String)> {
    let stripped = strip_ansi(line);
    let node = find_full_hex_commit(&stripped)?;
    let tail = line.get(line.find(&node)? + node.len()..)?;
    if strip_ansi(tail).trim().is_empty() {
        return None;
    }
    Some((node, format!("  {}", tail.trim_start())))
}

fn append_metadata_to_changesets(lines: &mut [String], metadata: &HashMap<String, String>) {
    for line in lines {
        let stripped = strip_ansi(line);
        if !stripped.contains("changeset:") {
            continue;
        }
        let Some(node) = find_full_hex_commit(&stripped) else {
            continue;
        };
        if let Some(suffix) = metadata.get(&node) {
            line.push_str(suffix);
        }
    }
}

fn run_action(action: Action, command_args: &[String]) -> Result<(), String> {
    let mut cmd = Command::new("sl");
    match action {
        Action::Quit(code) => exit(code),
        Action::Up(target) => {
            cmd.arg("up").args(command_args).arg(target);
        }
        Action::Rebase { source, dest } => {
            cmd.arg("rebase")
                .args(command_args)
                .arg("-s")
                .arg(source)
                .arg("-d")
                .arg(dest);
        }
        Action::Hide(target) => {
            cmd.arg("hide").args(command_args).arg(target);
        }
    }

    let status = cmd
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .map_err(|e| e.to_string())?;
    exit(status.code().unwrap_or(1));
}

struct AppState {
    output: Vec<String>,
    commit_pos: Pos,
    bookmark_index: Option<usize>,
    rebasing: Option<String>,
    rebasing_pos: Option<Pos>,
}

impl AppState {
    fn new(output: Vec<String>) -> Result<Self, String> {
        let commit_pos = search(1, Pos { line: -1, col: 0 }, is_current_commit_line, &output)
            .ok_or_else(|| "could not find current commit in smartlog output".to_owned())?;
        Ok(Self {
            output,
            commit_pos,
            bookmark_index: None,
            rebasing: None,
            rebasing_pos: None,
        })
    }

    fn handle_input(&mut self, input: &[u8]) -> Result<Option<Action>, String> {
        let mut i = 0;
        while i < input.len() {
            match input[i] {
                b'\x1b' if i + 2 < input.len() && input[i + 1] == b'[' => {
                    match input[i + 2] {
                        b'A' => self.update_commit(-1)?,
                        b'B' => self.update_commit(1)?,
                        b'D' => self.update_bookmark(-1)?,
                        b'C' => self.update_bookmark(1)?,
                        _ => return Ok(Some(Action::Quit(0))),
                    }
                    i += 3;
                }
                b'k' => {
                    self.update_commit(-1)?;
                    i += 1;
                }
                b'j' => {
                    self.update_commit(1)?;
                    i += 1;
                }
                b'\r' | b'\n' => return self.finish(|to| to),
                b'p' => return self.finish(|to| format!("{to}^")),
                b'r' => {
                    self.rebase_from_current()?;
                    i += 1;
                }
                b'x' => return Ok(Some(Action::Hide(self.current_target()?))),
                b'q' | 3 => return Ok(Some(Action::Quit(0))),
                b'\x1b' => return Ok(Some(Action::Quit(0))),
                _ => i += 1,
            }
        }

        Ok(None)
    }

    fn render(&self) -> Result<(), String> {
        let (rows, _) = terminal_size()?;
        let line_after = self.line_after_commit();
        let to = rows.max(line_after + 1);
        let from = to.saturating_sub(rows);

        let mut render_buffer: Vec<String> = self
            .output
            .iter()
            .skip(from)
            .take(to.saturating_sub(from + 1))
            .cloned()
            .collect();

        if let Some(bookmark_index) = self.bookmark_index {
            insert(
                "\u{1b}[0;33m",
                &[Pos {
                    line: self.commit_pos.line - from as isize,
                    col: bookmark_index + 7,
                }],
                &mut render_buffer,
            );
        }

        insert(
            "\u{1b}[35m",
            &self.color_markers_for_commit(line_after, from),
            &mut render_buffer,
        );

        self.mark_rebase_pos(&mut render_buffer, from);

        let mut stdout = io::stdout().lock();
        stdout
            .write_all(b"\x1b[2J\x1b[0f\x1b[0m")
            .map_err(|e| e.to_string())?;
        stdout
            .write_all(render_buffer.join("\u{1b}[0m\r\n").as_bytes())
            .map_err(|e| e.to_string())?;
        stdout.write_all(b"\r\n").map_err(|e| e.to_string())?;
        stdout.flush().map_err(|e| e.to_string())
    }

    fn color_markers_for_commit(&self, line_after: usize, line_offset: usize) -> Vec<Pos> {
        let mut markers = Vec::new();
        let to = line_after as isize - self.commit_pos.line;
        for i in 0..to {
            markers.push(Pos {
                line: self.commit_pos.line + i - line_offset as isize,
                col: self.commit_pos.col + 2,
            });
        }
        markers
    }

    fn mark_rebase_pos(&self, lines: &mut [String], line_offset: usize) {
        let Some(rebasing_pos) = self.rebasing_pos else {
            return;
        };
        let i = rebasing_pos.line - line_offset as isize;
        if i < 0 {
            return;
        }
        let Some(line) = lines.get_mut(i as usize) else {
            return;
        };
        *line = replace_at_char(line, rebasing_pos.col, "\u{1b}[0;1m←\u{1b}[0m");
    }

    fn update_commit(&mut self, direction: isize) -> Result<(), String> {
        self.commit_pos = search(direction, self.commit_pos, is_commit_line, &self.output)
            .unwrap_or(self.commit_pos);
        self.bookmark_index = None;
        self.render()
    }

    fn line_after_commit(&self) -> usize {
        search(1, self.commit_pos, is_commit_line, &self.output)
            .map(|pos| pos.line as usize)
            .unwrap_or(self.output.len())
    }

    fn update_bookmark(&mut self, direction: isize) -> Result<(), String> {
        let line = &self.output[self.commit_pos.line as usize];
        let from_index = match (self.bookmark_index, direction) {
            (None, -1) => line.chars().count().saturating_sub(1),
            (Some(index), _) => index.saturating_add_signed(direction),
            (None, _) => 0,
        };
        self.bookmark_index = index_of(direction, from_index, "\u{1b}[0;32m", line);
        self.render()
    }

    fn finish<F>(&self, to_modifier: F) -> Result<Option<Action>, String>
    where
        F: FnOnce(String) -> String,
    {
        let target = to_modifier(self.current_target()?);
        Ok(Some(match &self.rebasing {
            Some(source) => Action::Rebase {
                source: source.clone(),
                dest: target,
            },
            None => Action::Up(target),
        }))
    }

    fn rebase_from_current(&mut self) -> Result<(), String> {
        let current = self.current_target()?;
        if self.rebasing.as_ref() == Some(&current) {
            self.rebasing = None;
            self.rebasing_pos = None;
        } else {
            self.rebasing = Some(current);
            self.rebasing_pos = Some(self.commit_pos);
        }
        self.render()
    }

    fn current_target(&self) -> Result<String, String> {
        if let Some(bookmark_index) = self.bookmark_index {
            let line = &self.output[self.commit_pos.line as usize];
            let tail = slice_chars(line, bookmark_index);
            let stripped = tail.strip_prefix("\u{1b}[0;32m").unwrap_or(&tail);
            let bookmark: String = stripped
                .chars()
                .skip_while(|c| c.is_whitespace())
                .take_while(|c| !c.is_whitespace() && *c != '*')
                .collect();
            if !bookmark.is_empty() {
                return Ok(bookmark);
            }
        }

        let line = &self.output[self.commit_pos.line as usize];
        find_hex_commit(&strip_ansi(line)).ok_or_else(|| "could not find commit hash".to_owned())
    }
}

#[derive(Clone, Copy, Debug)]
struct Pos {
    line: isize,
    col: usize,
}

enum Action {
    Quit(i32),
    Up(String),
    Rebase { source: String, dest: String },
    Hide(String),
}

struct TerminalGuard {
    stty_state: String,
}

impl TerminalGuard {
    fn enter() -> Result<Self, String> {
        let stty_state = stty_output(&["-g"])?;
        run_stty(&["raw", "-echo"])?;
        print!("\u{1b}[?1049h");
        io::stdout().flush().map_err(|e| e.to_string())?;
        Ok(Self { stty_state })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = run_stty(&[self.stty_state.trim()]);
        let _ = io::stdout().write_all(b"\x1b[?1049l");
        let _ = io::stdout().flush();
    }
}

fn stty_output(args: &[&str]) -> Result<String, String> {
    let output = Command::new("stty")
        .args(args)
        .stdin(Stdio::inherit())
        .output()
        .map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        Err(String::from_utf8_lossy(&output.stderr).into_owned())
    }
}

fn run_stty(args: &[&str]) -> Result<(), String> {
    let status = Command::new("stty")
        .args(args)
        .stdin(Stdio::inherit())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("stty {:?} failed", args))
    }
}

fn terminal_size() -> Result<(usize, usize), String> {
    let output = stty_output(&["size"])?;
    let mut parts = output.split_whitespace();
    let rows = parts
        .next()
        .ok_or_else(|| "missing terminal rows".to_owned())?
        .parse::<usize>()
        .map_err(|e| e.to_string())?;
    let cols = parts
        .next()
        .ok_or_else(|| "missing terminal cols".to_owned())?
        .parse::<usize>()
        .map_err(|e| e.to_string())?;
    Ok((rows, cols))
}

fn is_current_commit_line(line: &str) -> Option<usize> {
    commit_marker_pos(line, |c| c == '@')
}

fn is_commit_line(line: &str) -> Option<usize> {
    commit_marker_pos(line, |c| c == '@' || c == 'o' || c == 'x')
}

fn commit_marker_pos<F>(line: &str, marker: F) -> Option<usize>
where
    F: Fn(char) -> bool,
{
    let mut prefix_len = 0;
    for ch in line.chars() {
        if PREFIX_CHARS.contains(&ch) {
            prefix_len += 1;
            continue;
        }
        if marker(ch) {
            return Some(prefix_len);
        }
        return None;
    }
    None
}

fn search<F>(direction: isize, from_pos: Pos, pattern: F, where_lines: &[String]) -> Option<Pos>
where
    F: Fn(&str) -> Option<usize>,
{
    let mut line = from_pos.line + direction;
    while line >= 0 && (line as usize) < where_lines.len() {
        let current = &where_lines[line as usize];
        if let Some(col) = pattern(current) {
            return Some(Pos { line, col });
        }
        line += direction;
    }
    None
}

fn index_of(direction: isize, from_index: usize, what: &str, where_str: &str) -> Option<usize> {
    if direction == 1 {
        where_str
            .match_indices(what)
            .find(|(idx, _)| char_pos(where_str, *idx) >= from_index)
            .map(|(idx, _)| char_pos(where_str, idx))
    } else {
        where_str
            .match_indices(what)
            .filter_map(|(idx, _)| {
                let pos = char_pos(where_str, idx);
                (pos <= from_index).then_some(pos)
            })
            .last()
    }
}

fn char_pos(s: &str, byte_idx: usize) -> usize {
    s[..byte_idx].chars().count()
}

fn strip_ansi(s: &str) -> String {
    let mut out = String::new();
    let mut chars = s.chars().peekable();

    while let Some(ch) = chars.next() {
        if ch == '\u{1b}' {
            if matches!(chars.peek(), Some('[')) {
                chars.next();
                for next in chars.by_ref() {
                    if next == 'm' {
                        break;
                    }
                }
            }
            continue;
        }
        out.push(ch);
    }

    out
}

fn slice_chars(s: &str, from: usize) -> String {
    s.chars().skip(from).collect()
}

fn replace_at_char(s: &str, at: usize, replacement: &str) -> String {
    let mut out = String::new();
    let mut replaced = false;
    for (i, ch) in s.chars().enumerate() {
        if i == at {
            out.push_str(replacement);
            replaced = true;
        } else if !(replaced && i == at + 1) {
            out.push(ch);
        }
    }
    if !replaced {
        return s.to_owned();
    }
    out
}

fn insert(what: &str, positions: &[Pos], inserted: &mut [String]) {
    for pos in positions {
        if pos.line < 0 {
            continue;
        }
        let Some(line) = inserted.get_mut(pos.line as usize) else {
            continue;
        };
        *line = insert_at_char(line, pos.col, what);
    }
}

fn insert_at_char(s: &str, at: usize, insert: &str) -> String {
    let mut out = String::new();
    let mut inserted = false;
    for (i, ch) in s.chars().enumerate() {
        if i == at {
            out.push_str(insert);
            inserted = true;
        }
        out.push(ch);
    }
    if !inserted {
        out.push_str(insert);
    }
    out
}

fn find_hex_commit(line: &str) -> Option<String> {
    find_hex_commit_with_min_len(line, 12)
}

fn find_full_hex_commit(line: &str) -> Option<String> {
    find_hex_commit_with_min_len(line, 40)
}

fn find_hex_commit_with_min_len(line: &str, min_len: usize) -> Option<String> {
    let mut current = String::new();
    for ch in line.chars() {
        if ch.is_ascii_hexdigit() {
            current.push(ch);
        } else {
            if current.len() >= min_len {
                return Some(current);
            }
            current.clear();
        }
    }
    (current.len() >= min_len).then_some(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_command_preserves_rev_args_and_drops_display_args() {
        let args = smartlog_metadata_command_args(&[
            "--stat".to_owned(),
            "-r".to_owned(),
            "draft()".to_owned(),
            "--patch".to_owned(),
        ]);
        assert_eq!(
            args,
            vec![
                "--color".to_owned(),
                "always".to_owned(),
                "smartlog".to_owned(),
                "-r".to_owned(),
                "draft()".to_owned(),
                "-T".to_owned(),
                SMARTLOG_METADATA_TEMPLATE.to_owned(),
            ]
        );
    }

    #[test]
    fn template_args_disable_metadata_injection() {
        assert!(has_template_arg(&["-T".to_owned(), "{sl}".to_owned()]));
        assert!(has_template_arg(&["--template={sl}".to_owned()]));
        assert!(has_template_arg(&["-T{sl}".to_owned()]));
    }

    #[test]
    fn metadata_for_line_keeps_colored_labels() {
        let line = "o  7d46efca1623c2947fb39c63272c4dd488419d93  \u{1b}[0;1m#10129\u{1b}[0m \u{1b}[0;32morigin/main\u{1b}[0m";
        assert_eq!(
            metadata_for_line(line),
            Some((
                "7d46efca1623c2947fb39c63272c4dd488419d93".to_owned(),
                "  \u{1b}[0;1m#10129\u{1b}[0m \u{1b}[0;32morigin/main\u{1b}[0m".to_owned(),
            ))
        );
    }

    #[test]
    fn append_metadata_only_changes_changeset_lines() {
        let node = "7d46efca1623c2947fb39c63272c4dd488419d93";
        let mut lines = vec![
            format!("╷ @  \u{1b}[0;1;34;93mchangeset:   {node}\u{1b}[0m  (@)"),
            "╷ │  summary:     Account for duplicated elementwise ops".to_owned(),
        ];
        let metadata =
            HashMap::from([(node.to_owned(), "  \u{1b}[0;1m#10129\u{1b}[0m".to_owned())]);

        append_metadata_to_changesets(&mut lines, &metadata);

        assert_eq!(
            lines[0],
            format!("╷ @  \u{1b}[0;1;34;93mchangeset:   {node}\u{1b}[0m  (@)  \u{1b}[0;1m#10129\u{1b}[0m")
        );
        assert_eq!(
            lines[1],
            "╷ │  summary:     Account for duplicated elementwise ops"
        );
    }
}
