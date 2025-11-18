# Gitsy

A terminal-based Git worktree manager with an intuitive TUI interface.

## Overview

Gitsy is a command-line tool that simplifies the management of Git worktrees. It provides a user-friendly terminal interface for creating and deleting worktrees, making it easy to work on multiple branches simultaneously without the hassle of stashing or switching contexts.

## Features

- **Interactive TUI**: Clean, keyboard-driven interface built with Ratatui
- **Create Worktrees**: Quickly create new branches with dedicated worktrees
- **Delete Worktrees**: Safely remove worktrees with sync status checks
- **Sync Detection**: Warns you when deleting branches that aren't in sync with their remote
- **First-Run Setup**: Interactive configuration wizard on first launch
- **Workspace Organization**: Keeps all worktrees in a configurable directory

## Requirements

- Rust 1.70 or later
- Git 2.5 or later (for worktree support)
- A Git repository to work with

## Installation

### From Source

```bash
git clone https://github.com/yourusername/gitsy.git
cd gitsy
cargo build --release
```

The binary will be available at `target/release/gitsy`.

### Using Cargo

```bash
cargo install --path .
```

## Usage

Navigate to any Git repository and run:

```bash
gitsy
```

### First-Time Setup

On first run, Gitsy will prompt you to configure the worktree storage path. This can be:
- An absolute path (e.g., `/Users/username/worktrees`)
- A relative path from the repository root (e.g., `../worktrees`)

This configuration is saved in `.gitsy.toml` in your repository root.

### Main Menu

The main menu provides three options:

1. **Create new branch**: Creates a new branch and worktree
2. **Delete a branch**: Lists and removes worktrees (with safety checks)
3. **Exit**: Quit the application

### Keyboard Navigation

- `↑/↓` or `j/k`: Navigate menu items
- `Enter`: Select/confirm
- `Esc`: Go back/cancel
- `Ctrl+C`: Exit application
- `y/n`: Confirm/cancel deletion

### Creating a Branch

1. Select "Create new branch" from the main menu
2. Enter the branch name
3. Press `Enter` to create the branch and worktree
4. The worktree will be created in your configured worktree directory

### Deleting a Branch

1. Select "Delete a branch" from the main menu
2. Navigate to the branch you want to delete
3. Press `Enter` to review
4. Gitsy will check if the branch is in sync with its remote
5. Confirm the deletion with `y` or cancel with `n`

## Configuration

Configuration is stored in `.gitsy.toml` at the root of your Git repository:

```toml
worktree_path = "../worktrees"
```

### Configuration Options

- `worktree_path`: Directory where worktrees will be created (required)

## How It Works

Gitsy uses Git's native worktree functionality to create isolated working directories for each branch. This allows you to:

- Work on multiple branches simultaneously
- Avoid stashing or committing incomplete work
- Run different versions of your code side-by-side
- Keep your main working directory clean

When you create a branch through Gitsy, it:
1. Creates a new Git branch
2. Creates a worktree directory in your configured location
3. Checks out the new branch in that worktree

When you delete a branch, Gitsy:
1. Checks if the branch is in sync with its remote
2. Warns you if there are unpushed changes
3. Removes the worktree and cleans up Git metadata

## Project Structure

```
gitsy/
├── Cargo.toml          # Project dependencies and metadata
├── src/
│   └── main.rs         # Main application logic
└── target/             # Build artifacts (gitignored)
```

## Development

### Building

```bash
cargo build
```

### Running in Development

```bash
cargo run
```

### Running Tests

```bash
cargo test
```

## Dependencies

- [ratatui](https://github.com/ratatui/ratatui) - Terminal UI framework
- [crossterm](https://github.com/crossterm-rs/crossterm) - Cross-platform terminal manipulation
- [git2](https://github.com/rust-lang/git2-rs) - libgit2 Rust bindings
- [serde](https://github.com/serde-rs/serde) - Serialization framework
- [toml](https://github.com/toml-rs/toml) - TOML parser
- [anyhow](https://github.com/dtolnay/anyhow) - Error handling
- [dirs](https://github.com/dirs-dev/dirs-rs) - Platform-specific directory paths

## License

This project is open source and available under the MIT License.

## Contributing

Contributions are welcome! Please feel free to submit issues or pull requests.

## Troubleshooting

### "Not a git repository" error
Make sure you're running Gitsy from within a Git repository or one of its subdirectories.

### Worktree creation fails
Check that:
- The branch name is valid
- The worktree path is accessible
- You have sufficient permissions
- The worktree directory doesn't already exist

### Can't delete a worktree
Ensure the worktree isn't currently in use (e.g., another terminal is in that directory).
