use anyhow::{Context, Result};
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEvent},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use git2::{BranchType, Repository};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Debug, Serialize, Deserialize)]
struct GitsyConfig {
    worktree_path: String,
    #[serde(default)]
    default_base_branch: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Screen {
    MainMenu,
    CreateBranch,
    SelectBaseBranchOption,
    SelectRemote,
    SelectRemoteBranch,
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
    remote_branches: Vec<String>,
    remote_branch_state: ListState,
    base_branch: Option<String>,
    remotes: Vec<String>,
    selected_remote: usize,
    base_branch_options: Vec<String>,
    selected_base_option: usize,
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
            remote_branches: Vec::new(),
            remote_branch_state: ListState::default(),
            base_branch: None,
            remotes: Vec::new(),
            selected_remote: 0,
            base_branch_options: Vec::new(),
            selected_base_option: 0,
        }
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> Result<bool> {
        match self.screen {
            Screen::MainMenu => self.handle_main_menu_key(key),
            Screen::CreateBranch => self.handle_create_branch_key(key),
            Screen::SelectBaseBranchOption => self.handle_select_base_branch_option_key(key),
            Screen::SelectRemote => self.handle_select_remote_key(key),
            Screen::SelectRemoteBranch => self.handle_select_remote_branch_key(key),
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
                    self.input.clear();
                    self.cursor_position = 0;
                    self.message = None;
                    self.base_branch = None;

                    // Build the base branch options
                    self.base_branch_options = Vec::new();
                    if let Some(ref default_branch) = self.config.default_base_branch {
                        self.base_branch_options
                            .push(format!("Use default: {}", default_branch));
                    }
                    self.base_branch_options
                        .push("Fetch from a remote...".to_string());
                    self.base_branch_options
                        .push("Use current HEAD".to_string());
                    self.selected_base_option = 0;
                    self.screen = Screen::SelectBaseBranchOption;
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

    fn handle_select_base_branch_option_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.screen = Screen::MainMenu;
                self.message = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_base_option > 0 {
                    self.selected_base_option -= 1;
                } else {
                    self.selected_base_option = self.base_branch_options.len() - 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected_base_option =
                    (self.selected_base_option + 1) % self.base_branch_options.len();
            }
            KeyCode::Enter => {
                let has_default = self.config.default_base_branch.is_some();
                let fetch_index = if has_default { 1 } else { 0 };
                let head_index = if has_default { 2 } else { 1 };

                if has_default && self.selected_base_option == 0 {
                    // Use default branch
                    self.base_branch = self.config.default_base_branch.clone();
                    self.screen = Screen::CreateBranch;
                    self.message = None;
                } else if self.selected_base_option == fetch_index {
                    // Fetch from a remote
                    match self.load_remotes() {
                        Ok(_) => {
                            if self.remotes.is_empty() {
                                self.message = Some("No remotes configured".to_string());
                                self.screen = Screen::CreateBranch;
                            } else {
                                self.selected_remote = 0;
                                self.screen = Screen::SelectRemote;
                                self.message = None;
                            }
                        }
                        Err(e) => {
                            self.message = Some(format!("Error loading remotes: {}", e));
                            self.screen = Screen::CreateBranch;
                        }
                    }
                } else if self.selected_base_option == head_index {
                    // Use current HEAD
                    self.base_branch = None;
                    self.screen = Screen::CreateBranch;
                    self.message = None;
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_select_remote_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.screen = Screen::SelectBaseBranchOption;
                self.message = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                if self.selected_remote > 0 {
                    self.selected_remote -= 1;
                } else {
                    self.selected_remote = self.remotes.len() - 1;
                }
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected_remote = (self.selected_remote + 1) % self.remotes.len();
            }
            KeyCode::Enter => {
                let remote = self.remotes[self.selected_remote].clone();
                self.message = Some(format!("Fetching from {}...", remote));
                match self.fetch_and_load_remote_branches_from(&remote) {
                    Ok(_) => {
                        if self.remote_branches.is_empty() {
                            self.message = Some("No remote branches found".to_string());
                            self.screen = Screen::CreateBranch;
                        } else {
                            self.remote_branch_state.select(Some(0));
                            self.screen = Screen::SelectRemoteBranch;
                            self.message = None;
                        }
                    }
                    Err(e) => {
                        self.message = Some(format!("Error fetching: {}", e));
                        self.screen = Screen::CreateBranch;
                    }
                }
            }
            _ => {}
        }
        Ok(false)
    }

    fn handle_select_remote_branch_key(&mut self, key: KeyEvent) -> Result<bool> {
        match key.code {
            KeyCode::Esc => {
                self.screen = Screen::SelectRemote;
                self.message = None;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                let current = self.remote_branch_state.selected().unwrap_or(0);
                let new_index = if current > 0 {
                    current - 1
                } else {
                    self.remote_branches.len() - 1
                };
                self.remote_branch_state.select(Some(new_index));
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let current = self.remote_branch_state.selected().unwrap_or(0);
                let new_index = (current + 1) % self.remote_branches.len();
                self.remote_branch_state.select(Some(new_index));
            }
            KeyCode::Enter => {
                if let Some(selected) = self.remote_branch_state.selected() {
                    self.base_branch = Some(self.remote_branches[selected].clone());
                    self.screen = Screen::CreateBranch;
                    self.message = None;
                }
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

        let mut cmd = Command::new("git");
        cmd.arg("worktree").arg("add").arg("-b").arg(&self.input);

        // If we have a base branch (from remote), use it as the starting point
        if let Some(ref base) = self.base_branch {
            cmd.arg(&branch_path).arg(base);
        } else {
            cmd.arg(&branch_path);
        }

        let output = cmd
            .current_dir(&self.repo_root)
            .output()
            .context("Failed to execute git worktree add")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("git worktree add failed: {}", stderr));
        }

        Ok(())
    }

    fn load_remotes(&mut self) -> Result<()> {
        let output = Command::new("git")
            .arg("remote")
            .current_dir(&self.repo_root)
            .output()
            .context("Failed to execute git remote")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("git remote failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        self.remotes = stdout
            .lines()
            .filter(|line| !line.is_empty())
            .map(|s| s.to_string())
            .collect();

        Ok(())
    }

    fn fetch_and_load_remote_branches_from(&mut self, remote: &str) -> Result<()> {
        // First, fetch from the specified remote
        let fetch_output = Command::new("git")
            .arg("fetch")
            .arg(remote)
            .arg("--prune")
            .current_dir(&self.repo_root)
            .output()
            .context("Failed to execute git fetch")?;

        if !fetch_output.status.success() {
            let stderr = String::from_utf8_lossy(&fetch_output.stderr);
            return Err(anyhow::anyhow!("git fetch failed: {}", stderr));
        }

        // Now list remote branches for this specific remote
        let output = Command::new("git")
            .arg("branch")
            .arg("-r")
            .arg("--format=%(refname:short)")
            .current_dir(&self.repo_root)
            .output()
            .context("Failed to execute git branch -r")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(anyhow::anyhow!("git branch -r failed: {}", stderr));
        }

        let stdout = String::from_utf8_lossy(&output.stdout);
        let prefix = format!("{}/", remote);
        self.remote_branches = stdout
            .lines()
            .filter(|line| !line.contains("HEAD") && line.starts_with(&prefix))
            .map(|s| s.to_string())
            .collect();

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
    #[derive(Debug, PartialEq)]
    enum SetupStep {
        WorktreePath,
        DefaultBaseBranch,
    }

    #[derive(Debug)]
    struct SetupApp {
        step: SetupStep,
        input: String,
        cursor_position: usize,
        repo_root: PathBuf,
        worktree_path: String,
    }

    impl SetupApp {
        fn new(repo_root: PathBuf) -> Self {
            Self {
                step: SetupStep::WorktreePath,
                input: String::new(),
                cursor_position: 0,
                repo_root,
                worktree_path: String::new(),
            }
        }

        fn handle_key_event(&mut self, key: KeyEvent) -> bool {
            match key.code {
                KeyCode::Enter => match self.step {
                    SetupStep::WorktreePath => {
                        if !self.input.is_empty() {
                            self.worktree_path = self.input.clone();
                            self.input.clear();
                            self.cursor_position = 0;
                            self.step = SetupStep::DefaultBaseBranch;
                        }
                    }
                    SetupStep::DefaultBaseBranch => {
                        return true;
                    }
                },
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

            match app.step {
                SetupStep::WorktreePath => {
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
                    let info_widget =
                        Paragraph::new(info).block(Block::default().borders(Borders::NONE));
                    f.render_widget(info_widget, chunks[3]);
                }
                SetupStep::DefaultBaseBranch => {
                    let instructions = Paragraph::new(
                        "Enter the default base branch for new worktrees (e.g., origin/main). Leave empty to skip:",
                    )
                    .style(Style::default().fg(Color::White))
                    .block(Block::default().borders(Borders::NONE));
                    f.render_widget(instructions, chunks[1]);

                    let input = Paragraph::new(app.input.as_str())
                        .style(Style::default().fg(Color::Yellow))
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title("Default Base Branch (optional)"),
                        );
                    f.render_widget(input, chunks[2]);

                    let info = vec![
                        Line::from(vec![
                            Span::styled("Worktree Path: ", Style::default().fg(Color::Green)),
                            Span::raw(&app.worktree_path),
                        ]),
                        Line::from(""),
                        Line::from(Span::styled(
                            "This branch will be offered as the first option when creating new worktrees.",
                            Style::default().fg(Color::DarkGray),
                        )),
                    ];
                    let info_widget =
                        Paragraph::new(info).block(Block::default().borders(Borders::NONE));
                    f.render_widget(info_widget, chunks[3]);
                }
            }

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
                    return Ok(());
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

    let default_base_branch = if app.input.is_empty() {
        None
    } else {
        Some(app.input)
    };

    Ok(GitsyConfig {
        worktree_path: app.worktree_path,
        default_base_branch,
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

fn run_app<B: ratatui::backend::Backend>(terminal: &mut Terminal<B>, app: &mut App) -> Result<()> {
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

                    let base_info = if let Some(ref base) = app.base_branch {
                        format!("Base branch: {}", base)
                    } else {
                        "Base branch: (current HEAD)".to_string()
                    };

                    let instructions = Paragraph::new(format!(
                        "{}\nType branch name and press Enter to create, Esc to cancel",
                        base_info
                    ))
                    .style(Style::default().fg(Color::DarkGray));
                    f.render_widget(instructions, chunks[2]);
                }
                Screen::SelectBaseBranchOption => {
                    let items: Vec<ListItem> = app
                        .base_branch_options
                        .iter()
                        .enumerate()
                        .map(|(i, option)| {
                            let style = if i == app.selected_base_option {
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::White)
                            };
                            ListItem::new(option.as_str()).style(style)
                        })
                        .collect();

                    let list = List::new(items).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("Select base branch option"),
                    );
                    f.render_widget(list, chunks[1]);

                    let instructions = Paragraph::new(
                        "Use ↑/↓ or j/k to navigate, Enter to select, Esc to cancel",
                    )
                    .style(Style::default().fg(Color::DarkGray));
                    f.render_widget(instructions, chunks[2]);
                }
                Screen::SelectRemote => {
                    let items: Vec<ListItem> = app
                        .remotes
                        .iter()
                        .enumerate()
                        .map(|(i, remote)| {
                            let style = if i == app.selected_remote {
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::White)
                            };
                            ListItem::new(remote.as_str()).style(style)
                        })
                        .collect();

                    let list = List::new(items).block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("Select remote to fetch from"),
                    );
                    f.render_widget(list, chunks[1]);

                    let instructions = Paragraph::new(
                        "Use ↑/↓ or j/k to navigate, Enter to select, Esc to go back",
                    )
                    .style(Style::default().fg(Color::DarkGray));
                    f.render_widget(instructions, chunks[2]);
                }
                Screen::SelectRemoteBranch => {
                    let items: Vec<ListItem> = app
                        .remote_branches
                        .iter()
                        .enumerate()
                        .map(|(i, branch)| {
                            let selected = app.remote_branch_state.selected().unwrap_or(0);
                            let style = if i == selected {
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD)
                            } else {
                                Style::default().fg(Color::White)
                            };
                            ListItem::new(branch.as_str()).style(style)
                        })
                        .collect();

                    let list = List::new(items)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title("Select remote branch to base your new branch on"),
                        )
                        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
                    f.render_stateful_widget(list, chunks[1], &mut app.remote_branch_state.clone());

                    let instructions = Paragraph::new(
                        "Use ↑/↓ or j/k to navigate, Enter to select, Esc to go back",
                    )
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
