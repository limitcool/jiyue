# Keyboard shortcuts

The defaults are listed below. Every command with a shortcut can be given other keys in Settings (Ctrl+,, page 快捷键). "Ctrl" is the Control key on Windows; see [Changing shortcuts](#changing-shortcuts).

## Whole window

| Action | Keys | Notes |
| --- | --- | --- |
| Quick switcher (open a note by name) | Ctrl+K or Ctrl+O | |
| Command palette | Ctrl+P | Lists all commands, including the ones without a shortcut. |
| New note | Ctrl+N | |
| Save the active note | Ctrl+S | Notes also autosave after a short pause. |
| Today's daily note | Ctrl+Shift+D | |
| Search panel (whole vault) | Ctrl+Shift+F | |
| Find in the active note | Ctrl+F | |
| Find and replace in the active note | Ctrl+H | |
| Next match | F3 | In the find bar. |
| Previous match | Shift+F3, or Shift+Enter in the find field | |
| Knowledge graph | Ctrl+G | |
| Toggle left sidebar | Ctrl+B | |
| Toggle split view | Ctrl+\\ | Splits the editor in two panes. |
| Cycle theme | Ctrl+T | Paper, Ink, Moss, Graphite, Fog. |
| Settings | Ctrl+, | |
| Open or close the AI assistant panel | Ctrl+Shift+A | |
| AI actions on the selection | Ctrl+J | Opens the menu for rewrite, shorten, expand, translate, fix and summarise. |
| Close a dialog, menu or palette | Escape | In Source view, Escape returns to the rendered view. |

## Editor

| Action | Keys |
| --- | --- |
| Undo | Ctrl+Z |
| Redo | Ctrl+Y or Ctrl+Shift+Z |
| Select all | Ctrl+A |
| Copy / Cut / Paste | Ctrl+C / Ctrl+X / Ctrl+V |
| Paste a clipboard image as an attachment | Ctrl+V (when the clipboard holds an image) |
| Move the caret by word | Ctrl+Left / Ctrl+Right |
| Select by word | Ctrl+Shift+Left / Ctrl+Shift+Right |
| Start / end of line | Home / End (Shift to select) |
| Start / end of document | Ctrl+Home / Ctrl+End (Ctrl+Shift to select) |
| Page up / down | PageUp / PageDown (Shift to select) |
| Delete word backwards / forwards | Ctrl+Backspace / Ctrl+Delete |
| Indent / outdent | Tab / Shift+Tab |
| Move line up / down | Alt+Up / Alt+Down |
| Duplicate line | Alt+Shift+Down |
| Open the wikilink under the pointer | Ctrl+click on `[[Note]]` |
| Open the slash menu | `/` at the start of a line, or after a space |

In the slash menu, Up and Down move, Enter or Tab inserts, and Escape closes. The menu offers headings 1 to 3, bullet, numbered and task lists, quote, divider, code block, table, the callouts (note, tip, warning, danger) and a link. Chinese names and pinyin also match.

## File tree

Select a row first (click it).

| Action | Keys |
| --- | --- |
| Rename the selected note or folder | F2 |
| Delete the selected note or folder (after confirmation; it goes to `.trash`) | Delete |
| Open or close a selected folder | Enter |
| Move a note or folder | Drag it onto a folder |

## Panes and tabs

| Action | Keys |
| --- | --- |
| Split right (open the active note beside it) | Ctrl+\\ |
| Move focus to the previous / next pane | Ctrl+Alt+Left or Ctrl+Alt+Up / Ctrl+Alt+Right or Ctrl+Alt+Down |
| Previous / next tab | Ctrl+PageUp / Ctrl+PageDown |
| Move the tab left / right | Ctrl+Shift+PageUp / Ctrl+Shift+PageDown |

Commands without a default shortcut (set one in Settings if you want): split down, close split, toggle the right panel, new folder, insert template, new note from template, insert embed, paste image, version history, previous and next daily note, copy the note's path, show the note in Explorer, immediate sync and sync settings.

## Changing shortcuts

Open Settings (Ctrl+,) and choose 快捷键. Pick a command, press the new keys, and the change is saved at once. Your changes are written to the `[keys]` section of `config.toml` in the user configuration folder (`%LOCALAPPDATA%\Jiyue` on Windows; see [configuration.md](configuration.md)); commands you do not change keep their defaults. When you choose keys that another command already uses, or a text-editing key (arrows, Ctrl+C/V/X/Z/A/Y, Backspace, Enter and so on), the settings page reports the conflict. A broken `keymap.json` is ignored as a whole and reported.
