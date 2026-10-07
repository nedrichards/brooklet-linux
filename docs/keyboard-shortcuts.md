# Keyboard shortcuts

Brooklet routes article shortcuts to the focused list or reader. Selection and
movement never open an article or change its read state. Enter deliberately
opens the selected article. Reader previous/next navigation also opens and
marks an article read.

| Context | Shortcut | Action |
| --- | --- | --- |
| List or reader | Down / J; Up / K | Select next/previous article, or scroll the reader |
| List or reader | Home / End | First/last article, or article beginning/end |
| List or reader | Page Down / Page Up | Move selection approximately a viewport, or scroll a page |
| Article list | Enter | Open selected article |
| Reader | Space / Shift+Space | Scroll a page down/up |
| Reader | N / P | Open next/previous article in the source collection |
| Article list | R | Toggle the selected article’s read state |
| Reader | R | Mark as Unread and Return to the source list |
| List or reader | S / Ctrl+D | Save or unsave selected/open article |
| List or reader | B | Open selected/open article in browser |
| List or reader | Ctrl+Shift+C | Copy selected/open article's link |
| List or reader | U / Ctrl+Z | Undo Mark Read |
| Navigation | Escape / Alt+Left | Dismiss popup/dialog, return from reader, or go back within Library |
| Navigation | F6 / Shift+F6 | Switch focus between list and reader in wide layouts |
| Navigation | Ctrl+1 / Ctrl+2 / Ctrl+3 | Show Inbox / Saved / Library |
| Application | Ctrl+F | Open the integrated article search page |
| Application | Ctrl+R | Sync articles |
| Application | Ctrl+, | Preferences |
| Application | F10 | Main menu |
| Application | F1 / Ctrl+? | Show shortcut help |
| Application | Ctrl+W / Ctrl+Q | Close window / quit |

Text fields, dropdowns and popups retain their native navigation and editing.
Selectable reader text retains cursor movement, Shift selection and Ctrl+C.
Enter and Space activate a focused button or link. Bare Ctrl+/ remains available
for native text selection; Ctrl+? opens shortcut help.

Navigation clamps at boundaries without wrapping. Holding navigation keys
repeats movement; holding article actions does not repeatedly toggle state or
open articles. Lock modifiers do not disable shortcuts. List-header navigation
moves into that list, while reader-header navigation scrolls the article.
Returning from the reader restores source-list focus and selection.

Subscribe, Refresh Feeds, Mark All Read and Karakeep delivery are reachable
through keyboard-operated menus. Shortcuts and their help entries are defined
together in `src/keyboard.rs`; button hints use the same definitions.

Double-click or double-tap the Inbox navigation icon to scroll Inbox to the
top. Single presses keep normal destination navigation. Read article titles
use normal weight; unread titles use bold weight across article lists.

## Regression checks

`brooklet --keyboard-test` uses a temporary database with synthetic articles;
it never accesses an installed account. The handler checks cover routing,
reader scrolling, list paging, selected/open action targets, native text and
button keys, and mutation repeat suppression.

For real X11 events, run under Xvfb with `GDK_BACKEND=x11` and
`BROOKLET_KEYBOARD_DRIVER` set to the absolute path of
`build-aux/keyboard-event.py`. The driver requires Python 3, libX11 and libXtst;
it targets only the uniquely named `Brooklet Keyboard Regression` window
created by that test process. The required
CI gate checks physical arrows, Enter, pane switching, Ctrl+Z, dialog typing,
Escape, Saved/Library switching, and returning to a list in narrow layouts.
The full `--smoke-test` separately checks reader rendering and image lifecycle.

The X11 gate proves event delivery in the synthetic fixture. It does not
substitute for a manual pass under the desktop's Wayland compositor or with
assistive technology.

## Reader actions

Open in Browser and Mark as Unread and Return remain in the reader header at
all supported widths. Previous/next appear together when the reader pane is
wider than 620sp; the duplicate header title hides at 400sp and below so that
primary actions and window controls fit even with larger text. The complete
headline remains in the article body. Previous/next remain available through
the article menu and P/N.
The menu groups primary actions, saving/link actions, then navigation, with
shortcut hints drawn from the same definitions as hover descriptions and help.
Save changes to Unsave for a starred article. Reader controls are disabled
until an article is open; previous/next are disabled at collection boundaries.
Mark as Unread and Return and R both restore source-list focus and selection,
including when the article is already unread. List shortcuts continue to act
on the selected article independently of the reader.
