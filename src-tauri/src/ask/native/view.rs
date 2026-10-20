//! What the quick ask window shows, and how it behaves (the logic of the
//! WebView page this replaced). The card first stands where the recording
//! indicator's dot is, in the same place and colours, with its spinner. Once
//! it is on screen the native dot goes and this one unfolds into a card with
//! the model's answers; its dot stays as the card's status light.
//!
//! The questions aren't shown, only the answers; a click on one copies it, a
//! click on a link in it opens the page, and the button in the corner of a
//! code block copies that code alone. While the model searches the web,
//! "Search…" blinks slowly where the answer will be. While the window is
//! open, every recording is a follow-up: the dot turns orange and pulses
//! while the user speaks. The cross folds the card back into the dot, which
//! slides behind the edge of the screen, and the window closes.
//!
//! Quick chat is the same window opened empty by its hotkey: the dot slides
//! in from behind the edge and unfolds into a card that says how to ask. The
//! questions are shown too, on the right in a box.

use std::cell::RefCell;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Model, ModelRc, SharedString, Timer, TimerMode, VecModel};

use super::markdown::{self, Ink, Kind};
use super::platform::{self, NativeWindow};
use super::shadow;
use super::{AskWindow, Block, Message, Theme};
use crate::ask::window::{Geometry, CARD_WIDTH, ORB_INSET};

/// What the window is opened with.
#[derive(Debug, Clone)]
pub struct Layout {
    pub geometry: Geometry,
    /// The indicator is at the bottom of the screen: the card grows upwards
    pub bottom: bool,
    /// "en" or "ru"
    pub language: &'static str,
    /// Quick chat: opened empty, the questions shown too
    pub chat: bool,
    /// The dictation hotkey, for the empty chat to name
    pub hotkey: String,
    pub push_to_talk: bool,
}

/// What the recorder is doing, while the window is open
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recording {
    Recording,
    Transcribing,
    Ready,
}

pub enum Command {
    /// A new window, hidden, its dot over the indicator's
    Open(Layout),
    /// On screen: the dot unfolds (quick chat's slides in first)
    Reveal,
    /// Quick chat: a question, with the start of the text it is about
    Question { id: u64, text: String, selection: Option<String> },
    /// An answer is coming
    Answer { id: u64 },
    /// The model searches the web
    Search { id: u64 },
    Delta { id: u64, text: String },
    /// The whole answer; `stopped` when a new question cut it short
    Done { id: u64, text: String, stopped: bool },
    Error { id: u64, message: String },
    /// Nothing was answered: the answer (and its question) go
    Cancel { id: u64 },
    Recording(Recording),
    RecordingError(String),
    /// Folds the card away, as its cross does
    Fold,
    /// Gone at once
    Destroy,
    /// The demo: the window's pixels now, to the callback
    Snapshot(Box<dyn FnOnce(Option<(u32, u32, Vec<u8>)>) + Send>),
    /// The demo's measurements: what the window's thread has done so far
    Stats(Box<dyn FnOnce(platform::Stats) + Send>),
}

pub enum Event {
    /// The window's first frame is on screen, its dot exactly over the
    /// indicator's: the native one can go
    Shown(Instant),
    /// Folded away: the window can be destroyed
    Closed,
}

pub struct Strings {
    pub copied: &'static str,
    pub code_copied: &'static str,
    pub searching: &'static str,
    pub chat_empty: &'static str,
    pub chat_empty_ptt: &'static str,
    pub stopped: &'static str,
    pub error: &'static str,
    pub mouse_middle: &'static str,
    pub mouse_back: &'static str,
    pub mouse_forward: &'static str,
}

const EN: Strings = Strings {
    copied: "Copied",
    code_copied: "Code copied",
    searching: "Search…",
    chat_empty: "Press {hotkey} and ask out loud",
    chat_empty_ptt: "Hold {hotkey} and ask out loud",
    stopped: "Stopped",
    error: "Couldn't answer: {error}",
    mouse_middle: "Mouse: middle",
    mouse_back: "Mouse: back",
    mouse_forward: "Mouse: forward",
};

const RU: Strings = Strings {
    copied: "Скопировано",
    code_copied: "Код скопирован",
    searching: "Поиск…",
    chat_empty: "Нажмите {hotkey} и задайте вопрос вслух",
    chat_empty_ptt: "Удерживайте {hotkey} и задайте вопрос вслух",
    stopped: "Остановлено",
    error: "Не удалось ответить: {error}",
    mouse_middle: "Мышь: средняя",
    mouse_back: "Мышь: назад",
    mouse_forward: "Мышь: вперёд",
};

pub fn strings(language: &str) -> &'static Strings {
    if language == "ru" {
        &RU
    } else {
        &EN
    }
}

/// A saved hotkey as the interface shows it, e.g. "Ctrl+Shift+Space"
fn format_hotkey(hotkey: &str, strings: &Strings) -> String {
    hotkey
        .split('+')
        .map(|key| match key {
            "MouseMiddle" => strings.mouse_middle.to_string(),
            "MouseBack" => strings.mouse_back.to_string(),
            "MouseForward" => strings.mouse_forward.to_string(),
            "Control" | "Ctrl" | "CmdOrCtrl" => "Ctrl".to_string(),
            "Meta" | "Super" => "Win".to_string(),
            key if key.starts_with("Key") => key[3..].to_uppercase(),
            key if key.starts_with("Digit") => key[5..].to_string(),
            key => key.to_string(),
        })
        .collect::<Vec<_>>()
        .join("+")
}

const NOTICE: Duration = Duration::from_millis(6000);
const TOAST: Duration = Duration::from_millis(1400);
/// When the text shows and the unfolding is over; quick chat, opened by its
/// hotkey, unfolds a little faster (ask.slint)
const UNFOLD_CHAT: (u64, u64) = (330, 500);
const UNFOLD_ASK: (u64, u64) = (420, 640);
/// Quick chat's dot slides in from behind the edge first
const ARRIVE: Duration = Duration::from_millis(210);
/// Folding back: the frame slides away, then the window can go
const FOLD_GONE: Duration = Duration::from_millis(340);
const FOLD_DONE: Duration = Duration::from_millis(600);

const STATE_DOT: i32 = 0;
const STATE_OPEN: i32 = 1;
const STATE_CLOSING: i32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Idle = 0,
    Listening = 1,
    Thinking = 2,
    Answering = 3,
}

/// Colours the formatting adds, as ARGB
const INK: Ink = Ink {
    strong: 0xfff8f8f6,
    muted: 0xff9c9a92,
};

struct Answer {
    /// The Markdown as written so far
    text: String,
    pending: bool,
    blocks: Rc<VecModel<Block>>,
    /// What the blocks show, to update only those that changed
    shown: Vec<markdown::Block>,
}

struct View {
    ui: AskWindow,
    window: Rc<NativeWindow>,
    layout: Layout,
    strings: &'static Strings,
    on_event: Rc<dyn Fn(Event)>,
    state: i32,
    phase: Phase,
    /// A quick ask window waits for its first answer to be announced
    awaiting_first: bool,
    messages: Rc<VecModel<Message>>,
    answers: HashMap<u64, Answer>,
    notice_timer: Timer,
    toast_timer: Timer,
    code_timer: Timer,
    /// The answer the left button went down on, and whether a link or a
    /// button took the click since
    pressed_answer: i32,
    click_taken: bool,
}

/// Runs the window on its thread: takes the commands, sends the events.
pub struct Controller {
    on_event: Rc<dyn Fn(Event)>,
    view: Option<Rc<RefCell<View>>>,
    card_shadow: Option<shadow::CardShadow>,
}

impl Controller {
    pub fn new(on_event: impl Fn(Event) + 'static) -> Self {
        Self {
            on_event: Rc::new(on_event),
            view: None,
            card_shadow: None,
        }
    }

    pub fn handle(&mut self, command: Command) {
        match command {
            Command::Open(layout) => {
                self.destroy();
                let shadow = self.card_shadow.get_or_insert_with(shadow::card);
                match View::open(layout, shadow, self.on_event.clone()) {
                    Ok(view) => self.view = Some(view),
                    Err(e) => log::error!("{}", e),
                }
            }
            Command::Destroy => self.destroy(),
            Command::Snapshot(then) => then(platform::current().and_then(|window| window.pixels())),
            Command::Stats(then) => then(platform::stats()),
            command => {
                if let Some(view) = &self.view {
                    View::handle(view, command);
                }
            }
        }
    }

    fn destroy(&mut self) {
        if let Some(view) = self.view.take() {
            let view = view.borrow();
            let _ = view.ui.hide();
            view.window.destroy();
        }
    }
}

/// The installed fonts closest to the page's: Segoe UI Variable (Windows 11)
/// or Segoe UI, Cascadia Mono or Consolas
fn fonts() -> (&'static str, &'static str) {
    let fonts = std::env::var_os("WINDIR")
        .map(|windir| std::path::PathBuf::from(windir).join("Fonts"))
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows\Fonts"));
    let ui = if fonts.join("SegUIVar.ttf").exists() {
        "Segoe UI Variable Text"
    } else {
        "Segoe UI"
    };
    let mono = if fonts.join("CascadiaMono.ttf").exists() {
        "Cascadia Mono"
    } else {
        "Consolas"
    };
    (ui, mono)
}

impl View {
    fn open(layout: Layout, card_shadow: &shadow::CardShadow, on_event: Rc<dyn Fn(Event)>) -> Result<Rc<RefCell<View>>, String> {
        let ui = AskWindow::new().map_err(|e| format!("The quick ask window couldn't be created: {}", e))?;
        let window = platform::current().ok_or("The quick ask window has no native window")?;
        let g = layout.geometry;
        window.open(g.x, g.y, g.width, g.height, g.scale as f32)?;

        let strings = strings(layout.language);
        let (ui_font, mono) = fonts();
        ui.set_ui_font(ui_font.into());
        ui.global::<Theme>().set_mono(mono.into());
        ui.set_bottom(layout.bottom);
        ui.set_chat(layout.chat);
        ui.set_orb_x(g.orb_x as f32);
        ui.set_orb_y(g.orb_y as f32);
        ui.set_orb_inset(ORB_INSET as f32);
        ui.set_card_width(CARD_WIDTH as f32);
        ui.set_card_max_height(g.max_height as f32);
        ui.set_card_shadow(card_shadow.image.clone());
        ui.set_card_shadow_left(card_shadow.left);
        ui.set_card_shadow_top(card_shadow.top);
        ui.set_card_shadow_right(card_shadow.right);
        ui.set_card_shadow_bottom(card_shadow.bottom);
        ui.set_dot_shadow(shadow::dot(g.scale as f32));
        ui.set_dot_shadow_reach(shadow::DOT_REACH);
        ui.set_searching_label(strings.searching.into());
        ui.set_stopped_label(strings.stopped.into());

        let messages = Rc::new(VecModel::default());
        ui.set_messages(ModelRc::from(messages.clone()));

        let view = Rc::new(RefCell::new(View {
            ui: ui.clone_strong(),
            window: window.clone(),
            layout,
            strings,
            on_event,
            state: STATE_DOT,
            phase: Phase::Thinking,
            awaiting_first: true,
            messages,
            answers: HashMap::new(),
            notice_timer: Timer::default(),
            toast_timer: Timer::default(),
            code_timer: Timer::default(),
            pressed_answer: -1,
            click_taken: false,
        }));
        view.borrow_mut().configure();

        // Only the card takes clicks
        let weak_ui = ui.as_weak();
        window.set_hit_area(Box::new(move || {
            weak_ui
                .upgrade()
                .map(|ui| (ui.get_card_x(), ui.get_card_y(), ui.get_card_w(), ui.get_card_h()))
        }));

        // A click on an answer copies it, unless it was on a link or a
        // button in it: the answer's text takes every click for its links,
        // so the window sorts the click out once Slint has handled it
        let weak = Rc::downgrade(&view);
        window.set_button_hook(Box::new(move |down| {
            let Some(view) = weak.upgrade() else { return };
            let mut v = view.borrow_mut();
            let hovered = v.ui.get_hovered_answer();
            if down {
                v.pressed_answer = hovered;
                v.click_taken = false;
            } else if !v.click_taken && hovered >= 0 && hovered == v.pressed_answer {
                v.pressed_answer = -1;
                v.copy_answer(hovered as u64);
            }
        }));
        let weak = Rc::downgrade(&view);
        ui.on_copy_code(move |id, index| {
            if let Some(view) = weak.upgrade() {
                let mut v = view.borrow_mut();
                v.click_taken = true;
                v.copy_code(id as u64, index as usize);
            }
        });
        let weak = Rc::downgrade(&view);
        ui.on_open_link(move |url| {
            if let Some(view) = weak.upgrade() {
                view.borrow_mut().click_taken = true;
            }
            if let Err(e) = crate::links::open_web_page(&url) {
                log::warn!("{}", e);
            }
        });
        let weak = Rc::downgrade(&view);
        ui.on_close_clicked(move || {
            if let Some(view) = weak.upgrade() {
                view.borrow_mut().click_taken = true;
                View::close(&view);
            }
        });

        ui.show().map_err(|e| format!("The quick ask window couldn't be shown: {}", e))?;
        Ok(view)
    }

    /// Lays the window out: the dot's place, the mode, the language
    fn configure(&mut self) {
        self.awaiting_first = !self.layout.chat;
        // Quick chat opens with nothing to do yet; its dot waits behind the
        // edge of the screen, where the indicator would come from
        if self.layout.chat {
            self.set_phase(Phase::Idle);
            self.ui.set_gone(true);
        } else {
            self.set_phase(Phase::Thinking);
        }
        self.render_empty();
    }

    fn handle(view: &Rc<RefCell<View>>, command: Command) {
        match command {
            Command::Reveal => View::reveal(view),
            Command::Fold => View::close(view),
            Command::Question { id, text, selection } => view.borrow_mut().on_question(id, &text, selection),
            Command::Answer { id } => view.borrow_mut().on_answer(id),
            Command::Search { id } => view.borrow_mut().on_search(id),
            Command::Delta { id, text } => view.borrow_mut().on_delta(id, &text),
            Command::Done { id, text, stopped } => view.borrow_mut().on_done(id, text, stopped),
            Command::Error { id, message } => view.borrow_mut().on_error(id, &message),
            Command::Cancel { id } => view.borrow_mut().on_cancel(id),
            Command::Recording(recording) => view.borrow_mut().on_recording(recording),
            Command::RecordingError(message) => view.borrow_mut().on_recording_error(message),
            Command::Open(_) | Command::Destroy | Command::Snapshot(_) | Command::Stats(_) => {}
        }
    }

    fn set_phase(&mut self, phase: Phase) {
        self.phase = phase;
        self.ui.set_phase(phase as i32);
    }

    // ── Unfolding and folding ────────────────────────────

    /// The dot is drawn and put on screen over the indicator's, then it
    /// unfolds (quick chat's slides in from behind the edge first).
    fn reveal(view: &Rc<RefCell<View>>) {
        let (chat, on_event) = {
            let v = view.borrow();
            if v.state != STATE_DOT {
                return;
            }
            v.window.render();
            v.window.show();
            (v.layout.chat, v.on_event.clone())
        };
        on_event(Event::Shown(Instant::now()));
        if chat {
            View::arrive(view);
        } else {
            View::unfold(view);
        }
    }

    fn arrive(view: &Rc<RefCell<View>>) {
        {
            let v = view.borrow();
            v.ui.set_arriving(true);
            v.ui.set_gone(false);
        }
        after(view, ARRIVE, |view| {
            view.borrow().ui.set_arriving(false);
            View::unfold(view);
        });
    }

    /// The dot unfolds into the card
    fn unfold(view: &Rc<RefCell<View>>) {
        let (content_in, done) = {
            let mut v = view.borrow_mut();
            if v.state != STATE_DOT {
                return;
            }
            v.state = STATE_OPEN;
            v.ui.set_motion(1);
            v.ui.set_state(STATE_OPEN);
            let (content_in, done) = if v.layout.chat { UNFOLD_CHAT } else { UNFOLD_ASK };
            (Duration::from_millis(content_in), Duration::from_millis(done))
        };
        after(view, content_in, |view| view.borrow().ui.set_content_in(true));
        after(view, done, |view| {
            let v = view.borrow();
            if v.state == STATE_OPEN {
                v.ui.set_motion(0);
            }
        });
    }

    fn close(view: &Rc<RefCell<View>>) {
        {
            let mut v = view.borrow_mut();
            if v.state == STATE_CLOSING {
                return;
            }
            v.state = STATE_CLOSING;
            v.ui.set_content_in(false);
            v.ui.set_motion(2);
            v.ui.set_state(STATE_CLOSING);
        }
        after(view, FOLD_GONE, |view| view.borrow().ui.set_gone(true));
        after(view, FOLD_DONE, |view| {
            let on_event = view.borrow().on_event.clone();
            on_event(Event::Closed);
        });
    }

    // ── Messages ─────────────────────────────────────────

    fn row_of(&self, id: u64) -> Option<usize> {
        (0..self.messages.row_count()).find(|&row| {
            self.messages
                .row_data(row)
                .is_some_and(|m| m.id as u64 == id && !m.question)
        })
    }

    fn update_row(&self, id: u64, change: impl FnOnce(&mut Message)) {
        if let Some(row) = self.row_of(id) {
            if let Some(mut message) = self.messages.row_data(row) {
                change(&mut message);
                self.messages.set_row_data(row, message);
            }
        }
    }

    /// Space above a new message, after the one before it
    fn spacing(&self, question: bool) -> (f32, bool) {
        let count = self.messages.row_count();
        let Some(previous) = count.checked_sub(1).and_then(|row| self.messages.row_data(row)) else {
            return (0.0, false);
        };
        match (previous.question, question) {
            (false, false) => (9.0, true),
            (true, false) => (6.0, false),
            _ => (14.0, false),
        }
    }

    fn answer_for(&mut self, id: u64) -> &mut Answer {
        if !self.answers.contains_key(&id) {
            let blocks = Rc::new(VecModel::default());
            let (gap, divider) = self.spacing(false);
            self.messages.push(Message {
                id: id as i32,
                question: false,
                text: SharedString::new(),
                quote: SharedString::new(),
                blocks: ModelRc::from(blocks.clone()),
                status: 0,
                settled: false,
                error: SharedString::new(),
                stopped: false,
                copied: 0,
                copied_block: -1,
                gap,
                divider,
            });
            self.answers.insert(
                id,
                Answer {
                    text: String::new(),
                    pending: true,
                    blocks,
                    shown: Vec::new(),
                },
            );
            self.ui.set_stick(true);
            self.render_empty();
        }
        self.answers.get_mut(&id).unwrap()
    }

    fn has_pending(&self) -> bool {
        self.answers.values().any(|answer| answer.pending)
    }

    /// The first words (or the end) arrived: the dots or "Search…" fold away
    fn settle(&mut self, id: u64) {
        let answer = self.answer_for(id);
        if !answer.pending {
            return;
        }
        answer.pending = false;
        self.update_row(id, |message| message.settled = true);
    }

    /// The phase after an answer, unless the user is already speaking again
    fn after_answer(&mut self, next: Phase) {
        if self.phase != Phase::Listening {
            self.set_phase(next);
        }
    }

    fn on_question(&mut self, id: u64, text: &str, selection: Option<String>) {
        if !self.layout.chat {
            return;
        }
        let (gap, _) = self.spacing(true);
        self.messages.push(Message {
            id: id as i32,
            question: true,
            text: text.into(),
            quote: selection.unwrap_or_default().into(),
            blocks: ModelRc::default(),
            status: 0,
            settled: true,
            error: SharedString::new(),
            stopped: false,
            copied: 0,
            copied_block: -1,
            gap,
            divider: false,
        });
        self.ui.set_stick(true);
        self.render_empty();
    }

    fn on_answer(&mut self, id: u64) {
        self.awaiting_first = false;
        self.answer_for(id);
        self.after_answer(Phase::Thinking);
    }

    fn on_search(&mut self, id: u64) {
        self.awaiting_first = false;
        if !self.answer_for(id).pending {
            return;
        }
        self.update_row(id, |message| message.status = 1);
        self.after_answer(Phase::Thinking);
    }

    fn on_delta(&mut self, id: u64, text: &str) {
        self.awaiting_first = false;
        self.settle(id);
        let answer = self.answer_for(id);
        answer.text.push_str(text);
        render(answer);
        self.after_answer(Phase::Answering);
    }

    fn on_done(&mut self, id: u64, text: String, stopped: bool) {
        self.awaiting_first = false;
        self.settle(id);
        let answer = self.answer_for(id);
        answer.text = text;
        render(answer);
        if stopped {
            self.update_row(id, |message| message.stopped = true);
        }
        let next = if self.has_pending() { Phase::Thinking } else { Phase::Idle };
        self.after_answer(next);
    }

    fn on_error(&mut self, id: u64, message: &str) {
        self.awaiting_first = false;
        self.settle(id);
        let answer = self.answer_for(id);
        answer.text.clear();
        answer.shown.clear();
        answer.blocks.set_vec(Vec::new());
        let error = self.strings.error.replace("{error}", message);
        self.update_row(id, |m| m.error = error.into());
        let next = if self.has_pending() { Phase::Thinking } else { Phase::Idle };
        self.after_answer(next);
    }

    fn on_cancel(&mut self, id: u64) {
        self.answers.remove(&id);
        // Nothing was answered: the question goes too, it has left the context
        let mut row = 0;
        while row < self.messages.row_count() {
            if self.messages.row_data(row).is_some_and(|m| m.id as u64 == id) {
                self.messages.remove(row);
            } else {
                row += 1;
            }
        }
        self.render_empty();
    }

    /// A follow-up: the recorder's own states, from the hotkey press on
    fn on_recording(&mut self, recording: Recording) {
        if self.state != STATE_OPEN {
            return;
        }
        match recording {
            Recording::Recording => {
                self.hide_notice();
                self.set_phase(Phase::Listening);
            }
            Recording::Transcribing => self.set_phase(Phase::Thinking),
            Recording::Ready => {
                if !self.awaiting_first && !self.has_pending() && self.phase != Phase::Answering {
                    self.set_phase(Phase::Idle);
                }
            }
        }
    }

    fn on_recording_error(&mut self, message: String) {
        if self.state != STATE_OPEN {
            return;
        }
        self.ui.set_notice(message.into());
        let ui = self.ui.as_weak();
        self.notice_timer.start(TimerMode::SingleShot, NOTICE, move || {
            if let Some(ui) = ui.upgrade() {
                ui.set_notice(SharedString::new());
            }
        });
        if !self.has_pending() && matches!(self.phase, Phase::Listening | Phase::Thinking) {
            self.set_phase(Phase::Idle);
        }
    }

    fn hide_notice(&self) {
        self.notice_timer.stop();
        self.ui.set_notice(SharedString::new());
    }

    /// Quick chat before the first question: which hotkey asks
    fn render_empty(&self) {
        let empty = self.layout.chat && self.messages.row_count() == 0;
        self.ui.set_show_empty(empty);
        if !empty {
            return;
        }
        let template = if self.layout.push_to_talk {
            self.strings.chat_empty_ptt
        } else {
            self.strings.chat_empty
        };
        let (before, after) = template.split_once("{hotkey}").unwrap_or((template, ""));
        self.ui.set_empty_before(before.into());
        self.ui.set_empty_key(format_hotkey(&self.layout.hotkey, self.strings).into());
        self.ui.set_empty_after(after.into());
    }

    // ── Copying ──────────────────────────────────────────

    fn copy_answer(&mut self, id: u64) {
        let Some(answer) = self.answers.get(&id) else { return };
        if answer.pending || answer.text.trim().is_empty() {
            return;
        }
        if let Err(e) = crate::paste::copy_text(&answer.text) {
            log::error!("Copy failed: {}", e);
            return;
        }
        // The answer lights up and the strip says so
        self.update_row(id, |message| message.copied += 1);
        self.toast(self.strings.copied);
    }

    fn copy_code(&mut self, id: u64, index: usize) {
        let code = match self.answers.get(&id).and_then(|answer| answer.shown.get(index)) {
            Some(markdown::Block {
                kind: Kind::Code(code), ..
            }) => code.clone(),
            _ => return,
        };
        if code.trim().is_empty() {
            return;
        }
        if let Err(e) = crate::paste::copy_text(&code) {
            log::error!("Copy failed: {}", e);
            return;
        }
        // The button turns into a check mark for a moment
        self.update_row(id, |message| message.copied_block = index as i32);
        let messages = Rc::downgrade(&self.messages);
        self.code_timer.start(TimerMode::SingleShot, TOAST, move || {
            let Some(messages) = messages.upgrade() else { return };
            for row in 0..messages.row_count() {
                if let Some(mut message) = messages.row_data(row).filter(|m| m.copied_block >= 0) {
                    message.copied_block = -1;
                    messages.set_row_data(row, message);
                }
            }
        });
        self.toast(self.strings.code_copied);
    }

    /// "Copied" in the status strip, next to the dot
    fn toast(&self, message: &str) {
        self.ui.set_toast(message.into());
        self.ui.set_toast_visible(true);
        let ui = self.ui.as_weak();
        self.toast_timer.start(TimerMode::SingleShot, TOAST, move || {
            if let Some(ui) = ui.upgrade() {
                ui.set_toast_visible(false);
            }
        });
    }
}

/// Runs `then` on the view after `delay`, unless it is gone.
fn after(view: &Rc<RefCell<View>>, delay: Duration, then: impl FnOnce(&Rc<RefCell<View>>) + 'static) {
    let weak = Rc::downgrade(view);
    Timer::single_shot(delay, move || {
        if let Some(view) = weak.upgrade() {
            then(&view);
        }
    });
}

/// The answer's blocks after its text changed: only those that differ are
/// replaced, so the text above the newest words isn't laid out again.
fn render(answer: &mut Answer) {
    let blocks = markdown::parse(&answer.text);
    for (index, block) in blocks.iter().enumerate() {
        if answer.shown.get(index) == Some(block) {
            continue;
        }
        let row = to_row(block);
        if index < answer.blocks.row_count() {
            answer.blocks.set_row_data(index, row);
        } else {
            answer.blocks.push(row);
        }
    }
    while answer.blocks.row_count() > blocks.len() {
        answer.blocks.remove(answer.blocks.row_count() - 1);
    }
    answer.shown = blocks;
}

fn to_row(block: &markdown::Block) -> Block {
    let mut row = Block {
        kind: 0,
        text: Default::default(),
        code: SharedString::new(),
        marker: SharedString::new(),
        indent: 0,
        size: 14.0,
        gap: block.gap,
        quoted: block.quoted,
        joined: block.joined,
        cells: ModelRc::default(),
        header: false,
    };
    match &block.kind {
        Kind::Paragraph(lines) => row.text = markdown::styled(lines, &INK, false),
        Kind::Heading(level, lines) => {
            row.kind = 1;
            row.size = if *level == 3 { 15.0 } else { 14.0 };
            row.text = markdown::styled(lines, &INK, true);
        }
        Kind::ListItem { marker, depth, lines } => {
            row.kind = 2;
            row.marker = marker.as_str().into();
            row.indent = *depth as i32;
            row.text = markdown::styled(lines, &INK, false);
        }
        Kind::Code(code) => {
            row.kind = 3;
            row.code = code.as_str().into();
        }
        Kind::Rule => row.kind = 4,
        Kind::TableRow { cells, header } => {
            row.kind = 5;
            row.header = *header;
            let cells: Vec<slint::StyledText> = cells
                .iter()
                .map(|cell| markdown::styled(std::slice::from_ref(cell), &INK, *header))
                .collect();
            row.cells = ModelRc::new(VecModel::from(cells));
        }
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_format_hotkey() {
        assert_eq!(format_hotkey("Ctrl+Shift+Space", &EN), "Ctrl+Shift+Space");
        assert_eq!(format_hotkey("Control+KeyQ", &EN), "Ctrl+Q");
        assert_eq!(format_hotkey("MouseMiddle", &RU), "Мышь: средняя");
    }
}
