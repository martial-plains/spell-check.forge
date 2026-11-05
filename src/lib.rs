// SPDX-License-Identifier: MIT OR Apache-2.0
//! spell-check: underline misspelled words as you type, and fix them.
//!
//! - `M-x spell-check-mode` toggles checking in this buffer. In code
//!   (any buffer whose major mode has a grammar) only comments and
//!   strings are checked.
//! - `M-$` (spell-correct-word) offers corrections for the word at point;
//!   pick one with TAB completion, or type your own.
//! - `M-x spell-accept-word` accepts the word at point for this session.
//! - `M-x spell-check-buffer` checks once and says how many it found.
//!
//! Words come from the system list, `/usr/share/dict/words` (Debian and
//! Ubuntu: `apt install wamerican`; macOS has one). Reading it is the one
//! permission this plugin asks for.
//!
//!   cargo build --release --target wasm32-wasip2
//!   cp target/wasm32-wasip2/release/forge_spell_check.wasm ~/.config/forge/plugins/spell-check.wasm

pub mod spell;

use forge_plugin_sdk::{
    Buffer, Decoration, Event, EventKind, Position, Scope, Span, command, completing_read,
    decorations, extend_submenu, forge_plugin, keymap, menu_item, message, modes, on,
};
use spell::{Dictionary, Region, Word};
use std::cell::RefCell;

pub const MODE: &str = "spell-check-mode";
/// Where word lists are looked for, in order.
/// (On Debian and Ubuntu `words` is a symlink out of /usr/share/dict,
/// which the sandbox doesn't follow, so the real files come next.)
pub const WORD_LISTS: &[&str] = &[
    "/usr/share/dict/words",
    "/usr/share/dict/web2",
    "/usr/share/dict/american-english",
    "/usr/share/dict/british-english",
];
/// Buffers bigger than this aren't checked (the whole text is read each time).
const MAX_CHARS: usize = 2_000_000;

thread_local! {
    static DICT: RefCell<Option<Dictionary>> = const { RefCell::new(None) };
}

/// Run `f` with the dictionary, loading it the first time.
fn with_dict<R>(f: impl FnOnce(&mut Dictionary) -> R) -> Result<R, String> {
    DICT.with(|d| {
        let mut d = d.borrow_mut();
        if d.is_none() {
            let list = WORD_LISTS.iter().find_map(|p| std::fs::read_to_string(p).ok()).ok_or_else(|| {
                format!(
                    "no word list found (looked for {}): install one, e.g. apt install wamerican",
                    WORD_LISTS.join(", ")
                )
            })?;
            *d = Some(Dictionary::from_list(&list));
        }
        Ok(f(d.as_mut().expect("just loaded")))
    })
}

fn pos(line: u32, column: u32) -> Position {
    Position { line, column }
}

/// Comments and strings, if the buffer is code; None for text.
fn regions(b: &Buffer, text: &str) -> Option<Vec<Region>> {
    let lines = text.split('\n').count() as u32;
    let all = Span {
        start: pos(0, 0),
        end: pos(lines, 0),
    };
    let faces = decorations::syntax(b, all);
    if faces.is_empty() {
        return None;
    }
    Some(
        faces
            .into_iter()
            .filter(|d| d.face == "comment" || d.face == "string")
            .map(|d| {
                (
                    (d.span.start.line, d.span.start.column),
                    (d.span.end.line, d.span.end.column),
                )
            })
            .collect(),
    )
}

/// Check a buffer and underline what's misspelled. Returns how many.
fn check(b: &Buffer) -> Result<usize, String> {
    let text = b.text();
    if text.chars().count() > MAX_CHARS {
        return Err(format!("{} is too big to spell check", b.name()));
    }
    let regions = regions(b, &text);
    let bad = with_dict(|d| spell::misspelled(d, &text, regions.as_deref()))?;
    let decos: Vec<Decoration> = bad
        .iter()
        .map(|w| Decoration {
            span: Span {
                start: pos(w.line, w.start),
                end: pos(w.line, w.end),
            },
            face: "spell-error".into(),
            message: Some(format!(
                "{}: not in the dictionary (M-$ to correct)",
                w.text
            )),
        })
        .collect();
    decorations::set(b, &decos)?;
    Ok(bad.len())
}

fn is_on(b: &Buffer) -> bool {
    b.minor_modes().iter().any(|m| m == MODE)
}

fn word_at_point(b: &Buffer) -> Option<Word> {
    let p = b.point();
    spell::word_at(&b.text(), p.line, p.column)
}

fn init() -> Result<(), String> {
    modes::define_minor_mode(MODE)?;

    command(
        MODE,
        "Toggle spell checking as you type in this buffer (only comments and strings in code).",
        |_| {
            let b = Buffer::current();
            if is_on(&b) {
                b.disable_minor_mode(MODE);
                decorations::clear(&b);
                message("Spell checking off");
            } else {
                b.enable_minor_mode(MODE)?;
                let n = check(&b)?;
                message(&format!(
                    "Spell checking on: {n} unknown word{}",
                    if n == 1 { "" } else { "s" }
                ));
            }
            Ok(())
        },
    )?;

    command(
        "spell-check-buffer",
        "Check this buffer's spelling once and underline unknown words.",
        |_| {
            let n = check(&Buffer::current())?;
            message(&match n {
                0 => "No unknown words".to_string(),
                1 => "1 unknown word".to_string(),
                n => format!("{n} unknown words"),
            });
            Ok(())
        },
    )?;

    command(
        "spell-correct-word",
        "Offer corrections for the word at point and replace it with the one you pick.",
        |_| {
            let b = Buffer::current();
            let Some(w) = word_at_point(&b) else {
                message("No word at point");
                return Ok(());
            };
            if with_dict(|d| d.knows(&w.text))? {
                message(&format!("{} is spelled correctly", w.text));
                return Ok(());
            }
            let options = with_dict(|d| d.suggestions(&w.text, 10))?;
            let version = b.version();
            let prompt = if options.is_empty() {
                format!("No suggestions; replace {} with: ", w.text)
            } else {
                format!("Correct {} to: ", w.text)
            };
            let id = b.id();
            completing_read(&prompt, &options, false, move |answer| {
                let (Some(new), Some(b)) = (answer.filter(|a| !a.is_empty()), Buffer::from_id(id))
                else {
                    return;
                };
                // Only if the text didn't change while we were asking.
                if b.version() == version {
                    b.replace(
                        Span {
                            start: pos(w.line, w.start),
                            end: pos(w.line, w.end),
                        },
                        &new,
                    );
                    if is_on(&b) {
                        let _ = check(&b);
                    }
                }
            });
            Ok(())
        },
    )?;

    command(
        "spell-accept-word",
        "Accept the word at point as correct for the rest of this session.",
        |_| {
            let b = Buffer::current();
            let Some(w) = word_at_point(&b) else {
                message("No word at point");
                return Ok(());
            };
            with_dict(|d| d.add(&w.text))?;
            message(&format!("Accepted {}", w.text));
            for b in Buffer::all().iter().filter(|b| is_on(b)) {
                check(b)?;
            }
            Ok(())
        },
    )?;

    keymap::bind(&Scope::Global, "M-$", "spell-correct-word")?;

    // The Edit menu's Spell submenu, where Emacs's is. Unloading the
    // plugin takes it back out again.
    extend_submenu(
        "Edit",
        "Spell",
        &[
            menu_item("Check Spelling", "spell-check-buffer"),
            menu_item("Correct Word", "spell-correct-word"),
            menu_item("Accept Word", "spell-accept-word"),
        ],
    )?;

    // Re-check when typing pauses (Forge sends buffer-changed after idle).
    on(EventKind::BufferChanged, |e| {
        if let Event::BufferChanged(id) = e
            && let Some(b) = Buffer::from_id(*id).filter(is_on)
            && let Err(err) = check(&b)
        {
            forge_plugin_sdk::warn(&err);
        }
    });
    // Turned off some other way: take the underlines away.
    on(EventKind::ModeChanged, |e| {
        if let Event::ModeChanged(id) = e
            && let Some(b) = Buffer::from_id(*id).filter(|b| !is_on(b))
        {
            decorations::clear(&b);
        }
    });
    Ok(())
}

forge_plugin!(init);
