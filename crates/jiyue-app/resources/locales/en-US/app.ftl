# Jiyue English Localization
-app-name = Jiyue

# Sidebar
sidebar-note-count = { $count ->
    [one] 1 note
   *[other] { $count } notes
}
sidebar-new-note = New Note
sidebar-search = Search Notes
sidebar-journal = Daily Journal
sidebar-graph = Knowledge Graph
sidebar-calendar = Calendar
sidebar-bookmarks = Bookmarks
sidebar-bookmarks-empty = Star a note or right click to bookmark
sidebar-files = Files
sidebar-tags = Tags
sidebar-settings = Settings
sidebar-backlinks = Backlinks
sidebar-outline = Outline

# Tab Bar & Common
tab-untitled = Untitled
tab-close = Close Tab
tab-close-all = Close All Tabs

# Welcome Page
welcome-title = { -app-name }
welcome-subtitle = Minimalist · Local-First · Pure Writing
welcome-start = Start
welcome-new-file = New Note
welcome-open-file = Open Note
welcome-open-vault = Switch Vault
welcome-recent = Recent Notes
welcome-no-recent = No recent notes
welcome-shortcuts = Keyboard Shortcuts
welcome-shortcut-palette = Command Palette
welcome-shortcut-quick-open = Quick Open
welcome-shortcut-search = Global Search
welcome-shortcut-settings = Preferences
welcome-tips = Features & Tips
welcome-tip-wikilink = Use [[Note Name]] to quickly link and form bidirectional graphs
welcome-tip-markdown = Native CommonMark & GFM support with instant live preview
welcome-tip-local = 100% stored in local Markdown files, true data ownership
welcome-show-on-startup = Show welcome page on startup

# Settings Nav Categories
settings-title = Settings
settings-cat-editor = Editor
settings-cat-appearance = Appearance
settings-cat-daily = Daily Notes
settings-cat-sync = Sync
settings-cat-ai = AI
settings-cat-keymap = Shortcuts
settings-cat-vaults = Vaults
settings-cat-about = About

# Settings Appearance
settings-appearance = Appearance
settings-theme = Theme
settings-theme-desc = Choose interface colour scheme
theme-paper = Paper
theme-ink = Ink
theme-moss = Moss
theme-graphite = Graphite
theme-fog = Fog
settings-language = Display Language
settings-language-desc = Change the interface display language with instant live reload
settings-lang-auto = System Default
settings-lang-zh = 简体中文 (zh-CN)
settings-lang-en = English (en-US)
settings-show-left-sidebar = Show Left Sidebar
settings-show-left-sidebar-desc = Sidebar for file tree, search, and tags
settings-show-right-panel = Show Right Panel
settings-show-right-panel-desc = Panel for outline and backlinks

# Settings Fonts
settings-fonts-title = Fonts & Sizing
settings-fonts-desc = Applies across all vaults, immediately reflected in editor and UI
settings-ui-font = UI Font
settings-ui-font-desc = Font family for menus, sidebars, tags, and buttons
settings-editor-font = Editor Font
settings-editor-font-desc = Font family used for editing and reading notes
settings-code-font = Code Font
settings-code-font-desc = Font family for code blocks, inline code, and formulas
settings-editor-font-size = Editor Font Size
settings-editor-font-size-desc = Font size for note editor and reading view (headings scale proportionally)
settings-code-font-size = Code Font Size
settings-code-font-size-desc = Font size for code blocks; follows editor at 90% when linked
settings-line-height = Line Height
settings-line-height-desc = Multiplier for spacing between lines
settings-font-default = Default ({ $label })
settings-font-default-ui = Default UI Font
settings-font-same-as-ui = Same as UI Font
settings-font-default-code = Default Monospace Font
settings-font-system-default = Default (System)
settings-font-preset-follow = Follow Body { $size } px
settings-font-search-placeholder = Search fonts…
settings-font-search-footer = Press Enter to apply, Esc to cancel
settings-font-searching = Loading font list…
settings-font-no-matches = No matching fonts
settings-font-count = { $count } fonts total
settings-font-more = { $count } more, type to filter
settings-font-tag-cjk = CJK
settings-font-tag-mono = Mono
settings-font-preview-title = Preview
settings-font-preview-heading = Heading Example
settings-font-preview-body = Jiyue is a local-first Markdown note-taking app. The quick brown fox 1234567890
settings-font-reset-family = Reset Fonts
settings-font-reset-size = Reset Font Sizes

# Settings Editor
settings-editor = Editor
settings-live-preview = Live Preview
settings-live-preview-desc = Render Markdown elements directly in the editor
settings-autoname = Name new note using first heading
settings-autoname-desc = Automatically rename new notes based on the first # heading
settings-editor-more-note = More editor preferences will be available soon

# Settings Daily
settings-daily-folder = Daily Notes Folder
settings-daily-folder-desc = Folder where new daily notes are created
settings-daily-format = Date Format
settings-daily-format-desc = File naming pattern for daily notes
settings-daily-format-preview = Filename Preview
settings-daily-template = Daily Note Template
settings-daily-template-desc = Template used when creating a daily note; "Change" cycles through note templates and returns to built-in

# Settings About & Updates
settings-about = About & Updates
update-card-title = Software Version & Updates
update-current-version = Current Version
update-channel = Release Channel
update-check-now = Check for Updates
update-checking = Checking for updates...
update-up-to-date = Jiyue is up to date
update-available = New version available: v{ $version }
update-download = Download & Update
update-downloading = Downloading update ({ $percent }%)...
update-install = Restart & Install
update-error = Update check failed: { $error }
update-ready = New version v{ $version } ready
update-ready-desc = Installation runs silently in background and restarts Jiyue
update-retry = Retry

# Status Bar
status-source = Source
status-live-preview = Live Preview
status-reading = Reading
status-cursor = Line { $line }, Col { $col }
status-history = History
status-sync-off = Sync Disabled
status-sync-synced = Synced · { $time }
status-sync-failed = Sync Failed
status-sync-conflicts = { $count ->
    [one] 1 conflict
   *[other] { $count } conflicts
}
status-sync-never = Not Synced Yet
status-ai-off = AI Off
status-ai-on = AI
status-saved = Saved
status-saving = Saving…
status-unsaved = Unsaved
status-save-failed = Save Failed

# Right Panel
panel-properties = Properties
panel-properties-empty = No properties
panel-outline = Outline
panel-outline-empty = No outline available
panel-backlinks = Backlinks
panel-backlinks-empty = No backlinks found
panel-unlinked = Unlinked Mentions
panel-unlinked-empty = No unlinked mentions found

# Find & Replace
find-placeholder = Find
find-replace-placeholder = Replace with
find-no-results = No results
find-replace = Replace
find-replace-all = Replace All
find-case-sensitive = Match Case
find-whole-word = Match Whole Word
find-shortcut-hint = Enter next · Esc close
find-no-match-to-replace = No match to replace
find-replaced-count = Replaced { $count } occurrences

# Search Panel
search-placeholder = Search notes… tag: path: file: "phrase" -exclude
search-results-count = { $count } results
search-add-bookmark = Bookmark Search
search-recent = Recent Searches
search-syntax = Search Syntax
search-no-results = No matching notes found
search-no-results-hint = Try different keywords, or remove filters
search-searching = Searching…
search-indexing = Preparing index…
search-footer-select = Select
search-footer-open = Open
search-footer-tab = New Tab
search-footer-close = Close

# Command Palette
palette-section-recent = Recent Notes
palette-section-commands = Commands
palette-no-matches = No matching entries
palette-empty-hint = Type to search all { $count } commands and notes
palette-footer-select = Select
palette-footer-open = Open
palette-footer-close = Close

# Context Menu & Actions
menu-split-right = Split Right
menu-split-down = Split Down
menu-close-split = Close Split
menu-close-tab = Close Tab
menu-close-other-tabs = Close Other Tabs
menu-close-right-tabs = Close Tabs to the Right
menu-copy-path = Copy File Path
menu-reveal-file = Reveal in File Manager
menu-rename = Rename
menu-move-to = Move To…
menu-bookmark-toggle = Bookmark / Unbookmark
menu-delete = Delete
menu-new-note = New Note
menu-new-folder = New Folder
menu-trash = Trash
menu-diff = Diff

# Bookmarks
bookmark-rename-group = Rename Group
bookmark-rename = Rename Bookmark
bookmark-move-up = Move Up
bookmark-move-down = Move Down
bookmark-move-to-group = Move to Group…
bookmark-ungroup = Ungroup
bookmark-remove-group = Remove Group
bookmark-remove = Remove Bookmark

# Keymap
keymap-search-placeholder = Search shortcuts…
keymap-reset-all = Reset All to Defaults
keymap-reset-all-confirm-title = Reset All Shortcuts
keymap-reset-all-confirm-desc = All shortcuts will be restored to defaults. Custom keybindings will be lost.
keymap-reset-all-confirm-btn = Reset to Defaults
keymap-no-matches = No matching commands found
keymap-reserved-title = Text Editing Keys (Reserved)
keymap-reserved-desc = These keys are reserved for text editing and cannot be reassigned.
keymap-capturing = Press new shortcut…
keymap-unbound = Unassigned
keymap-edit = Edit
keymap-cancel = Cancel
keymap-reset = Reset
keymap-capture-hint = Esc to cancel · Backspace to unassign

# Daily & Templates
settings-daily-format-example = Example: { $example }
settings-daily-note-hint = Existing daily notes will not be renamed; new settings only affect notes created afterwards.
settings-template-folder = Templates Folder
settings-template-folder-desc = Folder to read templates from when inserting or creating notes, relative to vault root
settings-template-builtin = Built-in Template
settings-template-change = Change

# Sync Settings
sync-backend = Storage Backend
sync-connection = Connection
sync-server-addr = Server Address
sync-server-addr-desc-s3 = Starts with http:// or https://; for RustFS / MinIO enter their URL
sync-server-addr-desc-dav = WebDAV root address, e.g. https://dav.example.com/dav
sync-region = Region
sync-region-desc = Compatible services can use us-east-1
sync-bucket = Bucket
sync-bucket-desc = Must be created on the server beforehand
sync-bucket-placeholder = Bucket Name
sync-prefix = Path Prefix
sync-prefix-desc = Folder in bucket for this vault; defaults to jiyue/<vault>/ if empty
sync-access-key-id = Access Key ID
sync-access-key-id-desc = Access Key ID
sync-secret-access-key = Secret Access Key
sync-secret-desc = Stored in plaintext in credentials.toml, never written to vault
sync-path-style = Path-style Addressing
sync-path-style-desc = Typically required for self-hosted services like RustFS and MinIO
sync-username = Username
sync-username-desc = WebDAV account
sync-password = Password
sync-dav-root = Remote Directory
sync-dav-root-desc = Folder to store this vault
sync-auto-sync-section = Automatic Sync
sync-enable = Enable Sync
sync-enable-desc = When off, remote is not accessed; click Save Settings after modifying
sync-periodic = Periodic Sync
sync-periodic-desc = Sync automatically at fixed intervals
sync-interval = Sync Interval
sync-interval-desc = Used during periodic sync
sync-on-save = Sync on Save
sync-on-save-desc = Wait briefly after saving a note before syncing
sync-debounce = Wait Time
sync-debounce-desc = Wait time after the last save
sync-on-start = Sync on Startup
sync-on-start-desc = Sync once when opening the vault
sync-on-exit = Sync on Exit
sync-on-exit-desc = Sync once before closing window, waiting up to 8s
sync-device-name = Device Name
sync-device-name-desc = Device identifier used in conflict file copies
sync-device-name-placeholder = Device Name
sync-ignore-rules = Ignore Rules
sync-ignore-desc = One per line, e.g. *.tmp, drafts/, merged with built-in rules
sync-test-connection = Test Connection
sync-sync-now = Sync Now
sync-save-settings = Save Settings
sync-danger-zone = Danger Zone
sync-reset-state = Reset Sync State
sync-reset-state-desc = Deletes .jiyue/sync/ (base records, conflict list, device id). Next sync re-compares local and remote and may produce conflict copies.
sync-reset-btn = Reset
sync-secret-unsaved = Not saved
sync-secret-saved-plain = { $masked } (saved, plaintext)
sync-time-10s = 10s
sync-time-30s = 30s
sync-time-1m = 1m
sync-time-5m = 5m
sync-time-15m = 15m

# AI Settings
ai-enable = Enable AI Features
ai-enable-desc = Disabled by default. You will be informed what content is sent before first use.
ai-providers = Providers
ai-no-providers-hint = No providers yet. Supports OpenAI-compatible APIs and Anthropic; keys are stored in plaintext in credentials.toml.
ai-add-provider = Add Provider
ai-edit-provider = Edit Provider
ai-chat-service = Chat Service
ai-add-service-first = Add a provider first
ai-embed-service = Embedding Service (for vector search, optional)
ai-do-not-use = Do not use
ai-excluded-folders = Excluded Folders
ai-excluded-tags = Excluded Tags
ai-exclusions-desc = These notes will not be sent to any model or indexed; their AI actions will be disabled.
ai-none = (None)
ai-example-folder = e.g. Private/Journal
ai-example-tag = e.g. secret
ai-folder-relative-hint = e.g. Private/Journal (folder relative to vault root)
ai-add-btn = Add
ai-vector-index = Vector Index
ai-index-desc = Each note is chunked into ~800 words and sent as an embedding request, billed by your provider. Stored in .jiyue/index, can be rebuilt anytime.
ai-rebuild-index = Rebuild Vector Index
ai-privacy = Privacy
ai-privacy-desc = A preview is always shown before sending. API keys are stored only in credentials.toml in user home, never in vault or logs.
ai-key-saved = Saved to credentials.toml
ai-key-unset = No key set
ai-key-reading = Checking key status…
ai-edit-btn = Edit
ai-name = Name
ai-kind = API Kind
ai-base-url = Base URL
ai-chat-model = Chat Model
ai-embed-model = Embedding Model (optional)
ai-api-key = API Key
ai-key-hint-saved = Saved in credentials.toml (leave empty to keep existing key)
ai-key-hint-unsaved = Saved to credentials.toml upon entry (plaintext)
ai-save-btn = Save
ai-cancel-btn = Cancel
ai-test-btn = Test Connection
ai-testing = Testing connection…
ai-test-ok = Connection OK

# Reserved Shortcuts
keymap-reserved-conflict-prefix = Text editing key
keymap-reserved-select-all = Select All
keymap-reserved-copy = Copy
keymap-reserved-cut = Cut
keymap-reserved-paste = Paste
keymap-reserved-undo = Undo
keymap-reserved-redo = Redo
keymap-reserved-move-left = Move Cursor Left
keymap-reserved-move-right = Move Cursor Right
keymap-reserved-move-up = Move Cursor Up
keymap-reserved-move-down = Move Cursor Down
keymap-reserved-select-left = Select Left
keymap-reserved-select-right = Select Right
keymap-reserved-select-up = Select Up
keymap-reserved-select-down = Select Down
keymap-reserved-word-left = Move Word Left
keymap-reserved-word-right = Move Word Right
keymap-reserved-select-word = Select Word
keymap-reserved-line-start = Line Start
keymap-reserved-line-end = Line End
keymap-reserved-select-line-start = Select to Line Start
keymap-reserved-select-line-end = Select to Line End
keymap-reserved-doc-start = Document Start
keymap-reserved-doc-end = Document End
keymap-reserved-select-doc-start = Select to Document Start
keymap-reserved-select-doc-end = Select to Document End
keymap-reserved-page-up = Page Up
keymap-reserved-page-down = Page Down
keymap-reserved-select-page-up = Select Page Up
keymap-reserved-select-page-down = Select Page Down
keymap-reserved-del-prev-char = Delete Previous Character
keymap-reserved-del-next-char = Delete Next Character
keymap-reserved-del-prev-word = Delete Previous Word
keymap-reserved-del-next-word = Delete Next Word
keymap-reserved-line-break = Insert Line Break
keymap-reserved-indent = Indent
keymap-reserved-outdent = Outdent
keymap-reserved-close-cancel = Close or Cancel
keymap-reserved-move-line-up = Move Line Up
keymap-reserved-move-line-down = Move Line Down
keymap-reserved-dup-line = Duplicate Line

# Vaults Settings
settings-vault-name = Vault Name
settings-vault-name-desc = Shown in sidebar, window title, and recent vaults list
settings-vault-rename = Rename
settings-vault-path = Vault Location
settings-vault-path-desc = Directory where this vault is stored (hover to see full path)
settings-vault-note-count = Note Count
settings-vault-note-count-desc = Markdown files in this vault
settings-vault-tag-count = Tag Count
settings-vault-tag-count-desc = Distinct tags across all notes
settings-vault-reveal = Show in File Explorer
settings-vault-copy-path = Copy Path
settings-vault-open-other = Open Other Vault…
settings-vault-new = New Vault…
settings-vault-rename-folder = Rename Directory…
settings-vault-recent-title = Recent Vaults
settings-vault-recent-empty = No recent vaults yet
settings-vault-badge-current = Current
settings-vault-badge-missing = Folder Missing
settings-vault-open = Open
settings-vault-remove = Remove
