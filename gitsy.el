;;; gitsy.el --- Git worktree manager with magit-style interface -*- lexical-binding: t -*-

;; Copyright (C) 2024

;; Author: Pete
;; Version: 0.1.0
;; Package-Requires: ((emacs "28.1") (transient "0.4.0") (magit-section "3.0.0"))
;; Keywords: git tools vc
;; URL: https://github.com/yourname/gitsy

;;; Commentary:

;; Gitsy provides a magit-style interface for managing git worktrees.
;;
;; Main entry point: M-x gitsy-status
;;
;; Features:
;; - Create worktrees with new branches (from HEAD, default base, or remote)
;; - Delete worktrees with sync-status warnings
;; - Configuration via .gitsy.toml or customize
;; - First-run setup wizard
;; - Projectile integration: opens worktrees as projects
;;
;; Keybindings in gitsy buffer:
;;   c   - Create worktree (transient menu) and open as project
;;   k   - Delete worktree at point
;;   RET - Visit worktree as projectile project
;;   g   - Refresh buffer
;;   n/p - Navigate sections
;;   TAB - Toggle section
;;   q   - Quit
;;   ?   - Help
;;
;; Customize `gitsy-use-projectile' and `gitsy-open-worktree-after-create'
;; to control projectile integration behavior.

;;; Code:

(require 'cl-lib)
(require 'transient)
(require 'magit-section)

;;; Customization

(defgroup gitsy nil
  "Git worktree manager."
  :group 'tools
  :prefix "gitsy-")

(defcustom gitsy-worktree-path nil
  "Default path for worktrees.
Can be overridden per-repository via .gitsy.toml."
  :type '(choice (const :tag "Use .gitsy.toml" nil)
                 (directory :tag "Directory"))
  :group 'gitsy)

(defcustom gitsy-default-base-branch nil
  "Default base branch for new worktrees.
Can be overridden per-repository via .gitsy.toml."
  :type '(choice (const :tag "Use .gitsy.toml" nil)
                 (string :tag "Branch name"))
  :group 'gitsy)

(defcustom gitsy-use-projectile t
  "Whether to use projectile when opening worktrees.
When non-nil, opening a worktree will switch to it as a projectile project."
  :type 'boolean
  :group 'gitsy)

(defcustom gitsy-open-worktree-after-create t
  "Whether to open worktree after creating it.
When non-nil, newly created worktrees are opened as projectile projects."
  :type 'boolean
  :group 'gitsy)

;;; Faces

(defgroup gitsy-faces nil
  "Faces for gitsy."
  :group 'gitsy
  :group 'faces)

(defface gitsy-header-face
  '((t :inherit bold :height 1.2))
  "Face for the gitsy buffer header."
  :group 'gitsy-faces)

(defface gitsy-section-heading-face
  '((t :inherit bold))
  "Face for section headings."
  :group 'gitsy-faces)

(defface gitsy-branch-face
  '((((class color) (background light)) :foreground "DarkGoldenrod4" :weight bold)
    (((class color) (background dark)) :foreground "LightGoldenrod2" :weight bold))
  "Face for branch names."
  :group 'gitsy-faces)

(defface gitsy-directory-face
  '((t :inherit font-lock-comment-face))
  "Face for directory paths."
  :group 'gitsy-faces)

(defface gitsy-sync-ok-face
  '((t :inherit success))
  "Face for synced status."
  :group 'gitsy-faces)

(defface gitsy-sync-warning-face
  '((t :inherit warning))
  "Face for ahead/behind status."
  :group 'gitsy-faces)

(defface gitsy-sync-error-face
  '((t :inherit error))
  "Face for diverged/no-upstream status."
  :group 'gitsy-faces)

;;; Data Structures

(cl-defstruct (gitsy-worktree (:constructor gitsy-worktree-create))
  "Represents a git worktree."
  path          ; Absolute path to worktree directory
  branch        ; Branch name (without refs/heads/)
  head          ; Current HEAD commit (short sha)
  bare-p        ; Non-nil if this is the bare/main repo
  locked-p      ; Non-nil if worktree is locked
  prunable-p    ; Non-nil if worktree can be pruned
  sync-status)  ; 'synced, 'ahead, 'behind, 'diverged, or 'no-upstream

(cl-defstruct (gitsy-config (:constructor gitsy-config-create))
  "Configuration for gitsy."
  worktree-path        ; Directory where worktrees are created
  default-base-branch) ; Default branch for new worktrees (e.g., "origin/main")

;;; Buffer-local Variables

(defvar-local gitsy--repo-root nil
  "Root directory of the current git repository.")

(defvar-local gitsy--config nil
  "Current gitsy configuration.")

(defvar-local gitsy--worktrees nil
  "List of gitsy-worktree structs for current repository.")

;;; Git Interaction Functions

(defun gitsy--call-git (&rest args)
  "Call git with ARGS and return output as string.
Returns nil if git command fails."
  (with-temp-buffer
    (let ((exit-code (apply #'call-process "git" nil t nil args)))
      (if (zerop exit-code)
          (string-trim (buffer-string))
        nil))))

(defun gitsy--call-git-lines (&rest args)
  "Call git with ARGS and return output as list of lines."
  (when-let ((output (apply #'gitsy--call-git args)))
    (split-string output "\n" t)))

(defun gitsy--find-repo-root ()
  "Find the root of the git repository.
Returns the path to the working directory."
  (let ((default-directory (or default-directory ".")))
    (gitsy--call-git "rev-parse" "--show-toplevel")))

(defun gitsy--list-worktrees ()
  "Return list of gitsy-worktree structs.
Parses output of: git worktree list --porcelain"
  (let ((output (gitsy--call-git "worktree" "list" "--porcelain"))
        (worktrees nil)
        (current-wt nil))
    (when output
      (dolist (line (split-string output "\n"))
        (cond
         ((string-prefix-p "worktree " line)
          (when current-wt
            (push current-wt worktrees))
          (setq current-wt (gitsy-worktree-create
                            :path (substring line 9))))
         ((string-prefix-p "HEAD " line)
          (when current-wt
            (setf (gitsy-worktree-head current-wt)
                  (substring line 5 12)))) ; short sha
         ((string-prefix-p "branch " line)
          (when current-wt
            (let ((branch (substring line 7)))
              (setf (gitsy-worktree-branch current-wt)
                    (if (string-prefix-p "refs/heads/" branch)
                        (substring branch 11)
                      branch)))))
         ((string= "bare" line)
          (when current-wt
            (setf (gitsy-worktree-bare-p current-wt) t)))
         ((string= "locked" line)
          (when current-wt
            (setf (gitsy-worktree-locked-p current-wt) t)))
         ((string= "prunable" line)
          (when current-wt
            (setf (gitsy-worktree-prunable-p current-wt) t)))))
      (when current-wt
        (push current-wt worktrees)))
    ;; Add sync status to each worktree
    (dolist (wt worktrees)
      (when (gitsy-worktree-branch wt)
        (setf (gitsy-worktree-sync-status wt)
              (gitsy--branch-sync-status (gitsy-worktree-branch wt)))))
    (nreverse worktrees)))

(defun gitsy--branch-sync-status (branch)
  "Check if BRANCH is in sync with its upstream.
Returns: \\='synced, \\='ahead, \\='behind, \\='diverged, or \\='no-upstream"
  (let ((output (gitsy--call-git "rev-list" "--left-right" "--count"
                                  (format "%s...%s@{upstream}" branch branch))))
    (if (null output)
        'no-upstream
      (let ((counts (split-string output)))
        (if (= (length counts) 2)
            (let ((ahead (string-to-number (nth 0 counts)))
                  (behind (string-to-number (nth 1 counts))))
              (cond
               ((and (zerop ahead) (zerop behind)) 'synced)
               ((and (> ahead 0) (> behind 0)) 'diverged)
               ((> ahead 0) 'ahead)
               ((> behind 0) 'behind)
               (t 'synced)))
          'no-upstream)))))

(defun gitsy--sync-status-string (status)
  "Return display string for sync STATUS."
  (pcase status
    ('synced "synced")
    ('ahead "ahead")
    ('behind "behind")
    ('diverged "diverged")
    ('no-upstream "no upstream")
    (_ "unknown")))

(defun gitsy--sync-status-face (status)
  "Return face for sync STATUS."
  (pcase status
    ('synced 'gitsy-sync-ok-face)
    ('ahead 'gitsy-sync-warning-face)
    ('behind 'gitsy-sync-warning-face)
    ('diverged 'gitsy-sync-error-face)
    ('no-upstream 'gitsy-sync-error-face)
    (_ 'default)))

(defun gitsy--list-remotes ()
  "Return list of remote names."
  (gitsy--call-git-lines "remote"))

(defun gitsy--fetch-remote (remote)
  "Fetch from REMOTE with prune."
  (message "Fetching from %s..." remote)
  (let ((result (gitsy--call-git "fetch" remote "--prune")))
    (if result
        (message "Fetched from %s" remote)
      (message "Fetch from %s completed" remote))
    t))

(defun gitsy--list-remote-branches (remote)
  "Return list of branches for REMOTE."
  (let ((branches (gitsy--call-git-lines "branch" "-r" "--format=%(refname:short)")))
    (seq-filter (lambda (b)
                  (and (string-prefix-p (concat remote "/") b)
                       (not (string-match-p "HEAD" b))))
                branches)))

(defun gitsy--list-local-branches ()
  "Return list of local branch names."
  (gitsy--call-git-lines "branch" "--format=%(refname:short)"))

(defun gitsy--create-worktree (branch-name &optional base-branch)
  "Create a new worktree for BRANCH-NAME.
If BASE-BRANCH is provided, use it as the starting point."
  (let* ((worktree-dir (gitsy--resolve-worktree-path))
         (branch-path (expand-file-name branch-name worktree-dir)))
    ;; Ensure worktree directory exists
    (unless (file-directory-p worktree-dir)
      (make-directory worktree-dir t))
    (let ((result (if base-branch
                      (gitsy--call-git "worktree" "add" "-b" branch-name
                                        branch-path base-branch)
                    (gitsy--call-git "worktree" "add" "-b" branch-name
                                      branch-path))))
      (if result
          (message "Created worktree for branch '%s'" branch-name)
        ;; Try to get error message
        (with-temp-buffer
          (call-process "git" nil t nil "worktree" "add" "-b" branch-name
                        branch-path (or base-branch "HEAD"))
          (error "Failed to create worktree: %s" (buffer-string)))))))

(defun gitsy--delete-worktree (worktree &optional force)
  "Delete WORKTREE and its associated branch.
If FORCE is non-nil, use --force flag for worktree removal
and -D for branch deletion."
  (let ((path (gitsy-worktree-path worktree))
        (branch (gitsy-worktree-branch worktree)))
    ;; First remove the worktree
    (with-temp-buffer
      (let ((exit-code (if force
                           (call-process "git" nil t nil "worktree" "remove" "--force" path)
                         (call-process "git" nil t nil "worktree" "remove" path))))
        (unless (zerop exit-code)
          (error "Failed to remove worktree: %s" (buffer-string)))))
    (message "Removed worktree at '%s'" path)
    ;; Then delete the branch if it exists
    (when branch
      (with-temp-buffer
        (let ((exit-code (if force
                             (call-process "git" nil t nil "branch" "-D" branch)
                           (call-process "git" nil t nil "branch" "-d" branch))))
          (if (zerop exit-code)
              (message "Deleted branch '%s'" branch)
            ;; Branch deletion failed, but worktree is gone - just warn
            (message "Warning: Could not delete branch '%s': %s"
                     branch (string-trim (buffer-string)))))))))

(defun gitsy--resolve-worktree-path ()
  "Resolve the worktree path from config."
  (let ((path (gitsy-config-worktree-path gitsy--config)))
    (if (file-name-absolute-p path)
        path
      (expand-file-name path gitsy--repo-root))))

;;; Configuration

(defun gitsy--parse-toml-simple (content)
  "Parse simple TOML CONTENT.
Only supports top-level key = \"value\" pairs."
  (let ((result nil))
    (dolist (line (split-string content "\n" t))
      (when (string-match "^\\([a-z_]+\\)\\s-*=\\s-*\"\\([^\"]*\\)\"" line)
        (let ((key (intern (match-string 1 line)))
              (value (match-string 2 line)))
          (push (cons key value) result))))
    (nreverse result)))

(defun gitsy--load-config (repo-root)
  "Load configuration for REPO-ROOT.
Checks .gitsy.toml in repo, falls back to customize variables."
  (let ((config-file (expand-file-name ".gitsy.toml" repo-root)))
    (if (file-exists-p config-file)
        (let* ((content (with-temp-buffer
                          (insert-file-contents config-file)
                          (buffer-string)))
               (parsed (gitsy--parse-toml-simple content)))
          (gitsy-config-create
           :worktree-path (or (cdr (assq 'worktree_path parsed))
                              gitsy-worktree-path
                              (error "worktree_path not configured"))
           :default-base-branch (or (cdr (assq 'default_base_branch parsed))
                                    gitsy-default-base-branch)))
      ;; No config file
      (if gitsy-worktree-path
          (gitsy-config-create
           :worktree-path gitsy-worktree-path
           :default-base-branch gitsy-default-base-branch)
        ;; Run setup wizard
        (if (gitsy--run-setup-wizard repo-root)
            (gitsy--load-config repo-root)
          (error "Gitsy configuration required"))))))

(defun gitsy--run-setup-wizard (repo-root)
  "Run the first-time setup wizard for REPO-ROOT.
Returns non-nil if config was created."
  (when (yes-or-no-p "No .gitsy.toml found. Run setup wizard? ")
    (let* ((worktree-path
            (read-string
             "Worktree directory (relative to repo or absolute): "
             "../worktrees"))
           (default-base
            (let ((input (read-string
                          "Default base branch (e.g., origin/main, empty to skip): ")))
              (unless (string-empty-p input) input)))
           (config-content
            (concat
             (format "worktree_path = \"%s\"\n" worktree-path)
             (when default-base
               (format "default_base_branch = \"%s\"\n" default-base)))))
      (with-temp-file (expand-file-name ".gitsy.toml" repo-root)
        (insert config-content))
      (message "Created .gitsy.toml")
      t)))

;;; Major Mode and Keymap

(defvar gitsy-mode-map
  (let ((map (make-sparse-keymap)))
    ;; Navigation (inherit from magit-section)
    (define-key map (kbd "n") #'magit-section-forward)
    (define-key map (kbd "p") #'magit-section-backward)
    (define-key map (kbd "TAB") #'magit-section-toggle)
    (define-key map (kbd "<backtab>") #'magit-section-cycle-global)
    (define-key map (kbd "^") #'magit-section-up)
    ;; Actions
    (define-key map (kbd "c") #'gitsy-create)
    (define-key map (kbd "k") #'gitsy-delete)
    (define-key map (kbd "RET") #'gitsy-visit)
    ;; Buffer operations
    (define-key map (kbd "g") #'gitsy-refresh)
    (define-key map (kbd "q") #'quit-window)
    (define-key map (kbd "?") #'gitsy-dispatch)
    map)
  "Keymap for `gitsy-mode'.")

(define-derived-mode gitsy-mode magit-section-mode "Gitsy"
  "Major mode for gitsy worktree management.

\\{gitsy-mode-map}"
  :group 'gitsy
  (setq-local revert-buffer-function #'gitsy-refresh))

;;; Buffer Rendering

(defun gitsy-refresh (&optional _ignore-auto _noconfirm)
  "Refresh the gitsy buffer."
  (interactive)
  (when gitsy--repo-root
    (let ((default-directory gitsy--repo-root))
      (setq gitsy--worktrees (gitsy--list-worktrees)))
    (let ((inhibit-read-only t)
          (pos (point)))
      (erase-buffer)
      (magit-insert-section (gitsy-root)
        (gitsy--insert-header)
        (gitsy--insert-config-section)
        (gitsy--insert-worktrees-section))
      (goto-char (min pos (point-max))))))

(defun gitsy--insert-header ()
  "Insert the buffer header."
  (insert (propertize "Gitsy" 'face 'gitsy-header-face))
  (insert " - Git Worktree Manager\n")
  (insert (propertize (format "Repository: %s" gitsy--repo-root)
                      'face 'gitsy-directory-face))
  (insert "\n\n"))

(defun gitsy--insert-config-section ()
  "Insert configuration section."
  (magit-insert-section (gitsy-config nil t)  ; collapsed by default
    (magit-insert-heading
      (propertize "Configuration" 'face 'gitsy-section-heading-face))
    (insert (format "  Worktree path: %s\n"
                    (propertize (gitsy-config-worktree-path gitsy--config)
                                'face 'gitsy-directory-face)))
    (if-let ((base (gitsy-config-default-base-branch gitsy--config)))
        (insert (format "  Default base:  %s\n"
                        (propertize base 'face 'gitsy-branch-face)))
      (insert "  Default base:  (none)\n"))
    (insert "\n")))

(defun gitsy--insert-worktrees-section ()
  "Insert worktrees section."
  (magit-insert-section (gitsy-worktrees nil)
    (magit-insert-heading
      (propertize (format "Worktrees (%d)"
                          (length (gitsy--managed-worktrees)))
                  'face 'gitsy-section-heading-face))
    (let ((managed (gitsy--managed-worktrees)))
      (if managed
          (dolist (wt managed)
            (gitsy--insert-worktree wt))
        (insert "  No managed worktrees\n")))
    (insert "\n")))

(defun gitsy--managed-worktrees ()
  "Return worktrees that are in the gitsy worktree directory."
  (let ((worktree-dir (gitsy--resolve-worktree-path)))
    (seq-filter (lambda (wt)
                  (and (not (gitsy-worktree-bare-p wt))
                       (string-prefix-p worktree-dir
                                        (gitsy-worktree-path wt))))
                gitsy--worktrees)))

(defun gitsy--insert-worktree (worktree)
  "Insert a single WORKTREE section."
  (magit-insert-section (gitsy-worktree worktree)
    (let* ((branch (or (gitsy-worktree-branch worktree) "(detached)"))
           (path (gitsy-worktree-path worktree))
           (sync (gitsy-worktree-sync-status worktree))
           (sync-str (gitsy--sync-status-string sync))
           (sync-face (gitsy--sync-status-face sync)))
      (insert "  ")
      (insert (propertize branch 'face 'gitsy-branch-face))
      (insert " ")
      (insert (propertize (format "[%s]" sync-str) 'face sync-face))
      (insert "\n")
      (insert (format "    %s\n"
                      (propertize path 'face 'gitsy-directory-face))))))

;;; Transient Menus

;;;###autoload (autoload 'gitsy-dispatch "gitsy" nil t)
(transient-define-prefix gitsy-dispatch ()
  "Show gitsy help."
  ["Gitsy Commands"
   ("c" "Create worktree" gitsy-create)
   ("k" "Delete worktree" gitsy-delete)
   ("RET" "Visit worktree" gitsy-visit)
   ("g" "Refresh" gitsy-refresh)]
  ["Navigation"
   ("n" "Next section" magit-section-forward)
   ("p" "Previous section" magit-section-backward)
   ("TAB" "Toggle section" magit-section-toggle)])

;;;###autoload (autoload 'gitsy-create "gitsy" nil t)
(transient-define-prefix gitsy-create ()
  "Create a new worktree."
  ["Create Worktree"
   ("c" "With default base" gitsy-create-default
    :if gitsy--has-default-base-p)
   ("l" "From local branch" gitsy-create-from-local)
   ("r" "From remote branch" gitsy-create-from-remote)
   ("h" "From HEAD" gitsy-create-from-head)])

(defun gitsy--has-default-base-p ()
  "Return non-nil if a default base branch is configured."
  (and gitsy--config
       (gitsy-config-default-base-branch gitsy--config)))

(transient-define-suffix gitsy-create-default ()
  "Create worktree using default base branch."
  :transient nil
  (interactive)
  (let* ((branch-name (read-string "New branch name: "))
         (base-branch (gitsy-config-default-base-branch gitsy--config))
         (worktree-path (expand-file-name branch-name (gitsy--resolve-worktree-path))))
    (when (string-empty-p branch-name)
      (user-error "Branch name cannot be empty"))
    (gitsy--create-worktree branch-name base-branch)
    (if gitsy-open-worktree-after-create
        (gitsy--open-worktree-project worktree-path)
      (gitsy-refresh))))

(transient-define-suffix gitsy-create-from-local ()
  "Create worktree from a local branch."
  :transient nil
  (interactive)
  (let ((local-branches (gitsy--list-local-branches)))
    (unless local-branches
      (user-error "No local branches found"))
    (let* ((base-branch (completing-read "Base branch: " local-branches nil t))
           (branch-name (read-string "New branch name: "))
           (worktree-path (expand-file-name branch-name (gitsy--resolve-worktree-path))))
      (when (string-empty-p branch-name)
        (user-error "Branch name cannot be empty"))
      (gitsy--create-worktree branch-name base-branch)
      (if gitsy-open-worktree-after-create
          (gitsy--open-worktree-project worktree-path)
        (gitsy-refresh)))))

(transient-define-suffix gitsy-create-from-remote ()
  "Create worktree from a remote branch."
  :transient nil
  (interactive)
  (let* ((remotes (gitsy--list-remotes)))
    (unless remotes
      (user-error "No remotes configured"))
    (let* ((remote (if (= (length remotes) 1)
                       (car remotes)
                     (completing-read "Remote: " remotes nil t)))
           (_ (gitsy--fetch-remote remote))
           (remote-branches (gitsy--list-remote-branches remote)))
      (unless remote-branches
        (user-error "No remote branches found"))
      ;; Present branch names without the remote prefix, defaulting to "main".
      (let* ((prefix (concat remote "/"))
             (branch-names (mapcar (lambda (b)
                                     (if (string-prefix-p prefix b)
                                         (substring b (length prefix))
                                       b))
                                   remote-branches))
             (default-base (car (member "main" branch-names)))
             (remote-branch (completing-read
                             (if default-base
                                 (format "Remote branch (default %s): " default-base)
                               "Remote branch: ")
                             branch-names nil t nil nil default-base))
             (base-branch (concat prefix remote-branch))
             (branch-name (read-string "New branch name: " remote-branch))
             (worktree-path (expand-file-name branch-name (gitsy--resolve-worktree-path))))
        (when (string-empty-p branch-name)
          (user-error "Branch name cannot be empty"))
        (gitsy--create-worktree branch-name base-branch)
        (if gitsy-open-worktree-after-create
            (gitsy--open-worktree-project worktree-path)
          (gitsy-refresh))))))

(transient-define-suffix gitsy-create-from-head ()
  "Create worktree from current HEAD."
  :transient nil
  (interactive)
  (let* ((branch-name (read-string "New branch name: "))
         (worktree-path (expand-file-name branch-name (gitsy--resolve-worktree-path))))
    (when (string-empty-p branch-name)
      (user-error "Branch name cannot be empty"))
    (gitsy--create-worktree branch-name nil)
    (if gitsy-open-worktree-after-create
        (gitsy--open-worktree-project worktree-path)
      (gitsy-refresh))))

;;; Commands

(defun gitsy--open-worktree-project (path)
  "Open worktree at PATH in dired in a new frame."
  (let ((frame (make-frame)))
    (select-frame-set-input-focus frame)
    (dired path)))

(defun gitsy-delete ()
  "Delete the worktree at point."
  (interactive)
  (let ((section (magit-current-section)))
    (if (and section (eq (oref section type) 'gitsy-worktree))
        (let* ((worktree (oref section value))
               (branch (or (gitsy-worktree-branch worktree) "(detached)"))
               (sync-status (gitsy-worktree-sync-status worktree))
               (warning (unless (eq sync-status 'synced)
                          (format "WARNING: Branch '%s' is NOT in sync with upstream!\n\n"
                                  branch)))
               (prompt (format "%sDelete worktree for '%s'? "
                               (or warning "") branch)))
          (when (yes-or-no-p prompt)
            (gitsy--delete-worktree worktree (not (null warning)))
            (gitsy-refresh)))
      (user-error "No worktree at point"))))

(defun gitsy-visit ()
  "Visit the worktree at point as a projectile project."
  (interactive)
  (let ((section (magit-current-section)))
    (if (and section (eq (oref section type) 'gitsy-worktree))
        (let* ((worktree (oref section value))
               (path (gitsy-worktree-path worktree)))
          (if (file-directory-p path)
              (gitsy--open-worktree-project path)
            (user-error "Worktree directory does not exist: %s" path)))
      (user-error "No worktree at point"))))

;;; Entry Point

;;;###autoload
(defun gitsy-status ()
  "Open the gitsy status buffer for the current repository."
  (interactive)
  (let* ((repo-root (or (gitsy--find-repo-root)
                        (user-error "Not in a git repository")))
         (buffer-name (format "*gitsy: %s*"
                              (file-name-nondirectory
                               (directory-file-name repo-root))))
         (buffer (get-buffer-create buffer-name)))
    (with-current-buffer buffer
      (unless (eq major-mode 'gitsy-mode)
        (gitsy-mode))
      (setq default-directory repo-root)
      (setq gitsy--repo-root repo-root)
      (setq gitsy--config (gitsy--load-config repo-root))
      (gitsy-refresh))
    (pop-to-buffer buffer)))

(provide 'gitsy)
;;; gitsy.el ends here
