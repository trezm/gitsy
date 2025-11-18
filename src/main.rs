use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use git2::{BranchType, Repository};
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, Paragraph},
    Terminal,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Serialize, Deserialize)]
struct GitsyConfig {
    worktree_path: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Screen {
    MainMenu,
    CreateBranch,
    DeleteBranch,
    ConfirmDelete,
}

struct MainMenu {
    selected: usize,
    items: Vec<&'static str>,
}

impl MainMenu {
    fn new() -> Self {
        Self {
            selected: 0,
            items: vec!["Create new branch", "Delete a branch", "Exit"],
        }
    }

    fn next(&mut self) {
        self.selected = (self.selected + 1) % self.items.len();
    }

    fn previous(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        } else {
            self.selected = self.items.len() - 1;
        }
    }
}

struct App {
    screen: Screen,
    main_menu: MainMenu,
    input: String,
    cursor_position: usize,
    repo_root: PathBuf,
    config: GitsyConfig,
    branches: Vec<String>,
    selected_branch: usize,
    message: Option<String>,
    confirm_delete: bool,
    branch_out_of_sync: bool,
}

impl App {
    fn new(repo_root: PathBuf, config: GitsyConfig) -> Self {
        Self {
            screen: Screen::MainMenu,
            main_menu: MainMenu::new(),
            input: String::new(),
            cursor_position: 0,
            repo_root,
            config,
            branches: Vec::new(),
            selected_branch: 0,
            message: None,
            confirm_delete: false,
            branch_out_of_sync: false,
        }
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Result<bool> {
        match self.screen {
            Screen::MainMenu => self.handle_main_menu_key(key),
            Screen::CreateBranch => self.handle_create_branch_key(key),
            Screen::DeleteBranch => self.handle_delete_branch_key(key),
            Screen::ConfirmDelete => self.handle_confirm_delete_key(key),
        }
    }

    fn handle_main_menu_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.main_menu.previous();
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.main_menu.next();
            }
            KeyCode::Enter => match self.main_menu.selected {
                0 => {
                    self.screen = Screen::CreateBranch;
                    self.input.clear();
                    self.cursor_position = 0;
                    self.message = None;
                }
                1 => {
                    self.load_branches()?;
                    if self.branches.is_empty() {
                        self.message = Some("No branches with worktrees found".to_string());
                    } else {
                        self.screen = Screen::DeleteBranch;
                        self.selected_branch = 0;
                        self.message = None;
                    }
                }
                2 => return Ok(true), // Exit
                _ => {}
            },
            _ => {}
        }
        Ok(false)
    }

    fn handle_create_branch_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.screen = Screen::MainMenu;
                self.message = None;
            }
            KeyCode::Enter => {
                if !self.input.is_empty() {
                    match self.create_worktree() {
                        Ok(_) => {
                            self.message = Some(format!(
                                "Successfully created worktree for branch '{}'",
                                self.input
                            ));
                            self.input.clear();
                            self.cursor_position = 0;
                        }
                        Err(e) => {
                            self.message = Some(format!("Error: {}", e));
                        }
                    }
                }
            }
            KeyCode::Char(c) => {
                self.input.insert(self.cursor_position, c);
                self.cursor_position += 1;
            }
            KeyCode::Backspace => {
                if self.cursor_position > 0 {
                    self.input.remove(self.cursor_position - 1);
                    self.cursor_position -= 1;
                }
            }
            KeyCode::Delete => {
                if self.cursor_position < self.input.len() {
                    self.input.remove(self.cursor_position);
                }
            }
            KeyCode::Left => {
                if self.cursor_position > 0 {
                    self.cursor_position -= 1;
                }
            }
            KeyCode::Right => {
                if self.cursor_position < self.input.len() {
                    self.cursor_position += 1;
                }
            }
            KeyCode::Home => {
                self.cursor_position = 0;
            }
            KeyCode::End => {
                self.cursor_position = self.input.len();
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_delete_branch_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.screen = Screen::MainMenu;
                self.message = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_branch > 0 {
                    self.selected_branch -= 1;
                } else {
                    self.selected_branch = self.branches.len() - 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected_branch = (self.selected_branch + 1) % self.branches.len();
            }
            KeyCode::Enter => {
                let branch_name = &self.branches[self.selected_branch];
                self.branch_out_of_sync = !self.is_branch_in_sync(branch_name)?;
                self.screen = Screen::ConfirmDelete;
                self.confirm_delete = false;
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_confirm_delete_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.screen = Screen::DeleteBranch;
            }
            KeyCode::Char('y') | KeyCode::Char('Y') => {
                let branch_name = self.branches[self.selected_branch].clone();
                match self.delete_worktree(&branch_name) {
                    Ok(_) => {
                        self.message = Some(format!(
                            "Successfully deleted worktree for branch '{}'",
                            branch_name
                        ));
                        self.screen = Screen::MainMenu;
                    }
                    Err(e) => {
                        self.message = Some(format!("Error: {}", e));
                        self.screen = Screen::MainMenu;
                    }
                }
            }
            KeyCode::Char('n') | KeyCode::Char('N') => {
                self.screen = Screen::DeleteBranch;
            }
            _ => {}
        }
        Ok(false)
    }

    fn create_worktree(&self) -> Result<()> {
        let worktree_path = if Path::new(&self.config.worktree_path).is_absolute() {
            PathBuf::from(&self.config.worktree_path)
        } else {
            self.repo_root.join(&self.config.worktree_path)
        };

        let branch_path = worktree_path.join(&self.input);

        let output = Command::new("git")
            .arg("worktree")
            .arg("add")
            .arg("-b")
            .arg(&self.input)
            .arg(&branch_path)
            .current_dir(&self.repo_root)
            .output()
            .context("Failed to execute git worktree add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("git worktree add failed: {}", stderr));
        }

        Ok(())
    }

    fn load_branches(&mut self) -> Result<()> {
        let output = Command::new("git")
            .arg("worktree")
            .arg("list")
            .arg("--porcelain")
            .current_dir(&self.repo_root)
            .output()
            .context("Failed to execute git worktree list")?;

        if !output.status.success() {
            return Err(anyhow::anyhow!("Failed to list worktrees"));
        }

        let worktree_path = if Path::new(&self.config.worktree_path).is_absolute() {
            PathBuf::from(&self.config.worktree_path)
        } else {
            self.repo_root.join(&self.config.worktree_path)
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut branches = Vec::new();
        let mut current_worktree_path: Option<PathBuf> = None;

        for line in stdout.lines() {
            if line.starts_with("worktree ") {
                current_worktree_path = Some(PathBuf::from(line.trim_start_matches("worktree ")));
            } else if line.starts_with("branch ") {
                if let Some(ref wt_path) = current_worktree_path {
                    // Only include branches whose worktree is in the gitsy workspace
                    if wt_path.starts_with(&worktree_path) {
                        let branch = line.trim_start_matches("branch refs/heads/").to_string();
                        branches.push(branch);
                    }
                }
                current_worktree_path = None;
            }
        }

        self.branches = branches;
        Ok(())
    }

    fn is_branch_in_sync(&self, branch_name: &str) -> Result<bool> {
        let repo = Repository::open(&self.repo_root)?;

        let local_branch = repo.find_branch(branch_name, BranchType::Local)?;
        let local_oid = local_branch
            .get()
            .target()
            .context("Failed to get local branch target")?;

        let upstream = match local_branch.upstream() {
            Ok(upstream) => upstream,
            Err(_) => return Ok(true), // No upstream, consider it in sync
        };

        let upstream_oid = upstream
            .get()
            .target()
            .context("Failed to get upstream branch target")?;

        Ok(local_oid == upstream_oid)
    }

    fn delete_worktree(&self, branch_name: &str) -> Result<()> {
        let worktree_path = if Path::new(&self.config.worktree_path).is_absolute() {
            PathBuf::from(&self.config.worktree_path)
        } else {
            self.repo_root.join(&self.config.worktree_path)
        };

        let branch_path = worktree_path.join(branch_name);

        let output = Command::new("git")
            .arg("worktree")
            .arg("remove")
            .arg(&branch_path)
            .current_dir(&self.repo_root)
            .output()
            .context("Failed to execute git worktree remove")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("git worktree remove failed: {}", stderr));
        }

        Ok(())
    }
}

fn find_git_root() -> Result<PathBuf> {
    let current_dir = std::env::current_dir().context("Failed to get current directory")?;
    let repo = Repository::discover(&current_dir)
        .context("Not a git repository (or any parent up to mount point)")?;

    let workdir = repo
        .workdir()
        .context("Repository doesn't have a working directory")?;

    Ok(workdir.to_path_buf())
}

fn load_or_create_config(repo_root: &Path) -> Result<GitsyConfig> {
    let config_path = repo_root.join(".gitsy.toml");

    if config_path.exists() {
        let content = fs::read_to_string(&config_path).context("Failed to read .gitsy.toml")?;
        let config: GitsyConfig =
            toml::from_str(&content).context("Failed to parse .gitsy.toml")?;
        Ok(config)
    } else {
        let config = run_tui_setup(repo_root)?;
        save_config(repo_root, &config)?;
        Ok(config)
    }
}

fn save_config(repo_root: &Path, config: &GitsyConfig) -> Result<()> {
    let config_path = repo_root.join(".gitsy.toml");
    let toml_string = toml::to_string_pretty(config).context("Failed to serialize config")?;
    fs::write(&config_path, toml_string).context("Failed to write .gitsy.toml")?;
    Ok(())
}

fn run_tui_setup(repo_root: &Path) -> Result<GitsyConfig> {
    #[derive(Debug)]
    struct SetupApp {
        input: String,
        cursor_position: usize,
        repo_root: PathBuf,
    }

    impl SetupApp {
        fn new(repo_root: PathBuf) -> Self {
            Self {
                input: String::new(),
                cursor_position: 0,
                repo_root,
            }
        }

        fn handle_key_event(&mut self, key: KeyEvent) -> bool {
            match key.code {
                KeyCode::Enter => return true,
                KeyCode::Char(c) => {
                    self.input.insert(self.cursor_position, c);
                    self.cursor_position += 1;
                }
                KeyCode::Backspace => {
                    if self.cursor_position > 0 {
                        self.input.remove(self.cursor_position - 1);
                        self.cursor_position -= 1;
                    }
                }
                KeyCode::Delete => {
                    if self.cursor_position < self.input.len() {
                        self.input.remove(self.cursor_position);
                    }
                }
                KeyCode::Left => {
                    if self.cursor_position > 0 {
                        self.cursor_position -= 1;
                    }
                }
                KeyCode::Right => {
                    if self.cursor_position < self.input.len() {
                        self.cursor_position += 1;
                    }
                }
                KeyCode::Home => {
                    self.cursor_position = 0;
                }
                KeyCode::End => {
                    self.cursor_position = self.input.len();
                }
                _ => {}
            }
            false
        }
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = SetupApp::new(repo_root.to_path_buf());

    let result: Result<()> = (|| loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(2)
                .constraints(
                    [
                        Constraint::Length(3),
                        Constraint::Length(3),
                        Constraint::Length(3),
                        Constraint::Min(0),
                    ]
                    .as_ref(),
                )
                .split(f.area());

            let title = Paragraph::new("Gitsy - Git Worktree Manager")
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(title, chunks[0]);

            let instructions = Paragraph::new(
                "Enter the path where gitsy worktrees will be stored (Press Enter to confirm, Ctrl+C to cancel):",
            )
            .style(Style::default().fg(Color::White))
            .block(Block::default().borders(Borders::NONE));
            f.render_widget(instructions, chunks[1]);

            let input = Paragraph::new(app.input.as_str())
                .style(Style::default().fg(Color::Yellow))
                .block(
                    Block::default()
                        .borders(Borders::ALL)
                        .title("Worktree Path"),
                );
            f.render_widget(input, chunks[2]);

            let info = vec![
                Line::from(vec![
                    Span::styled("Git Repository: ", Style::default().fg(Color::Green)),
                    Span::raw(app.repo_root.display().to_string()),
                ]),
                Line::from(""),
                Line::from(Span::styled(
                    "The path can be absolute or relative to the repository root.",
                    Style::default().fg(Color::DarkGray),
                )),
            ];
            let info_widget = Paragraph::new(info).block(Block::default().borders(Borders::NONE));
            f.render_widget(info_widget, chunks[3]);

            f.set_cursor_position((
                chunks[2].x + app.cursor_position as u16 + 1,
                chunks[2].y + 1,
            ));
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Esc
                    || (key.code == KeyCode::Char('c')
                        && key.modifiers.contains(event::KeyModifiers::CONTROL))
                {
                    return Err(anyhow::anyhow!("Setup cancelled by user"));
                }

                if app.handle_key_event(key) {
                    if !app.input.is_empty() {
                        return Ok(());
                    }
                }
            }
        }
    })();

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result?;

    Ok(GitsyConfig {
        worktree_path: app.input,
    })
}

fn run_main_app(repo_root: PathBuf, config: GitsyConfig) -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(repo_root, config);
    let result = run_app(&mut terminal, &mut app);

    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    result
}

fn run_app<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
) -> Result<()> {
    loop {
        terminal.draw(|f| {
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .margin(2)
                .constraints(
                    [
                        Constraint::Length(3),
                        Constraint::Min(10),
                        Constraint::Length(3),
                    ]
                    .as_ref(),
                )
                .split(f.area());

            let title = Paragraph::new("Gitsy - Git Worktree Manager")
                .style(Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                .block(Block::default().borders(Borders::ALL));
            f.render_widget(title, chunks[0]);

            match app.screen {
                Screen::MainMenu => {
                    let items: Vec<ListItem> = app
                        .main_menu
                        .items
                        .iter()
                        .enumerate()
                        .map(|(i, item)| {
                            let style = if i == app.main_menu.selected {
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::White)
                            };
                            ListItem::new(*item).style(style)
                        })
                        .collect();

                    let list = List::new(items)
                        .block(Block::default().borders(Borders::ALL).title("Main Menu"))
                        .highlight_style(Style::default().fg(Color::Yellow));
                    f.render_widget(list, chunks[1]);

                    let instructions =
                        Paragraph::new("Use ↑/↓ or j/k to navigate, Enter to select, Esc to go back")
                            .style(Style::default().fg(Color::DarkGray));
                    f.render_widget(instructions, chunks[2]);
                }
                Screen::CreateBranch => {
                    let content_chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(3), Constraint::Min(0)].as_ref())
                        .split(chunks[1]);

                    let input = Paragraph::new(app.input.as_str())
                        .style(Style::default().fg(Color::Yellow))
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title("Enter new branch name"),
                        );
                    f.render_widget(input, content_chunks[0]);

                    if let Some(ref msg) = app.message {
                        let msg_style = if msg.starts_with("Error") {
                            Style::default().fg(Color::Red)
                        } else {
                            Style::default().fg(Color::Green)
                        };
                        let message = Paragraph::new(msg.as_str())
                            .style(msg_style)
                            .block(Block::default().borders(Borders::ALL).title("Status"));
                        f.render_widget(message, content_chunks[1]);
                    }

                    f.set_cursor_position((
                        content_chunks[0].x + app.cursor_position as u16 + 1,
                        content_chunks[0].y + 1,
                    ));

                    let instructions =
                        Paragraph::new("Type branch name and press Enter to create, Esc to cancel")
                            .style(Style::default().fg(Color::DarkGray));
                    f.render_widget(instructions, chunks[2]);
                }
                Screen::DeleteBranch => {
                    let items: Vec<ListItem> = app
                        .branches
                        .iter()
                        .enumerate()
                        .map(|(i, branch)| {
                            let style = if i == app.selected_branch {
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::White)
                            };
                            ListItem::new(branch.as_str()).style(style)
                        })
                        .collect();

                    let list = List::new(items).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("Select branch to delete"),
                    );
                    f.render_widget(list, chunks[1]);

                    let instructions =
                        Paragraph::new("Use ↑/↓ or j/k to navigate, Enter to delete, Esc to cancel")
                            .style(Style::default().fg(Color::DarkGray));
                    f.render_widget(instructions, chunks[2]);
                }
                Screen::ConfirmDelete => {
                    let branch_name = &app.branches[app.selected_branch];
                    let warning_text = if app.branch_out_of_sync {
                        format!(
                            "WARNING: Branch '{}' is NOT in sync with origin!\n\nAre you sure you want to delete this worktree? (y/N)",
                            branch_name
                        )
                    } else {
                        format!(
                            "Branch '{}' is in sync with origin.\n\nAre you sure you want to delete this worktree? (y/N)",
                            branch_name
                        )
                    };

                    let style = if app.branch_out_of_sync {
                        Style::default().fg(Color::Red)
                    } else {
                        Style::default().fg(Color::Yellow)
                    };

                    let confirm = Paragraph::new(warning_text)
                        .style(style)
                        .block(Block::default().borders(Borders::ALL).title("Confirm Delete"));
                    f.render_widget(confirm, chunks[1]);

                    let instructions = Paragraph::new("Press Y to confirm, N or Esc to cancel")
                        .style(Style::default().fg(Color::DarkGray));
                    f.render_widget(instructions, chunks[2]);
                }
            }

            if let Some(ref msg) = app.message {
                if app.screen == Screen::MainMenu {
                    let msg_style = if msg.starts_with("Error") || msg.starts_with("No branches") {
                        Style::default().fg(Color::Red)
                    } else {
                        Style::default().fg(Color::Green)
                    };
                    let message = Paragraph::new(msg.as_str())
                        .style(msg_style)
                        .block(Block::default().borders(Borders::ALL).title("Status"));

                    let popup_area = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Percentage(70), Constraint::Length(5), Constraint::Percentage(25)].as_ref())
                        .split(f.area())[1];

                    f.render_widget(message, popup_area);
                }
            }
        })?;

        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.code == KeyCode::Char('c')
                    && key.modifiers.contains(event::KeyModifiers::CONTROL)
                {
                    break;
                }

                if app.handle_key_event(key)? {
                    break;
                }
            }
        }
    }

    Ok(())
}

fn main() -> Result<()> {
    let repo_root = find_git_root()?;
    let config = load_or_create_config(&repo_root)?;
    run_main_app(repo_root, config)?;
    Ok(())
}
