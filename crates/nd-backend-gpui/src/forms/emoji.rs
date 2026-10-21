//! nd's own emoji picker, for a text area on a platform without a system one (Linux): every
//! emoji the `emojis` crate carries in its CLDR groups, search by name and shortcode, a skin
//! tone, and the emoji used last, kept across launches. It opens above the button that asked for
//! it, or at the field's caret; the field keeps focus, so typing while it is open searches and the arrows, Return, Tab and Escape drive
//! it, as GTK's emoji chooser does.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use emojis::{Emoji, EmojiVersion, Group, SkinTone};
use nd_components::Icon;
use nd_gpui::gpui::{
    self, Anchor, AnyElement, App, ElementId, InteractiveElement, IntoElement, Keystroke, MouseDownEvent,
    ParentElement, Pixels, Point, ScrollStrategy, SharedString, StatefulInteractiveElement, Styled,
    UniformListScrollHandle, Window, anchored, deferred, div, point, prelude::FluentBuilder, px, uniform_list,
};

use super::FormLook;
use super::glyph::{Glyph, glyph};

/// Emoji per grid row.
pub(crate) const COLUMNS: usize = 8;
const CELL: f32 = 36.;
const TAB: f32 = 28.;
const ROWS: usize = 6;
/// How many recently used emoji are kept.
pub(crate) const RECENT_MAX: usize = 24;
/// The newest emoji the common colour fonts draw (Noto Color Emoji 2.047, Apple Color Emoji);
/// later ones would show as boxes.
const NEWEST: EmojiVersion = EmojiVersion::new(16, 0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Tab {
    Recent,
    Group(Group),
}

pub(crate) const TABS: [Tab; 10] = [
    Tab::Recent,
    Tab::Group(Group::SmileysAndEmotion),
    Tab::Group(Group::PeopleAndBody),
    Tab::Group(Group::AnimalsAndNature),
    Tab::Group(Group::FoodAndDrink),
    Tab::Group(Group::TravelAndPlaces),
    Tab::Group(Group::Activities),
    Tab::Group(Group::Objects),
    Tab::Group(Group::Symbols),
    Tab::Group(Group::Flags),
];

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Tab::Recent => "Recently Used",
            Tab::Group(Group::SmileysAndEmotion) => "Smileys & Emotion",
            Tab::Group(Group::PeopleAndBody) => "People & Body",
            Tab::Group(Group::AnimalsAndNature) => "Animals & Nature",
            Tab::Group(Group::FoodAndDrink) => "Food & Drink",
            Tab::Group(Group::TravelAndPlaces) => "Travel & Places",
            Tab::Group(Group::Activities) => "Activities",
            Tab::Group(Group::Objects) => "Objects",
            Tab::Group(Group::Symbols) => "Symbols",
            Tab::Group(Group::Flags) => "Flags",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Tab::Recent => Icon::Clock,
            Tab::Group(Group::SmileysAndEmotion) => Icon::Smile,
            Tab::Group(Group::PeopleAndBody) => Icon::Hand,
            Tab::Group(Group::AnimalsAndNature) => Icon::PawPrint,
            Tab::Group(Group::FoodAndDrink) => Icon::Utensils,
            Tab::Group(Group::TravelAndPlaces) => Icon::Car,
            Tab::Group(Group::Activities) => Icon::PartyPopper,
            Tab::Group(Group::Objects) => Icon::Lightbulb,
            Tab::Group(Group::Symbols) => Icon::Heart,
            Tab::Group(Group::Flags) => Icon::Flag,
        }
    }
}

fn drawable(e: &&'static Emoji) -> bool {
    e.emoji_version() <= NEWEST
}

/// The skin tones the tone row offers, in `SkinTone` order.
const TONES: [SkinTone; 6] =
    [SkinTone::Default, SkinTone::Light, SkinTone::MediumLight, SkinTone::Medium, SkinTone::MediumDark, SkinTone::Dark];

/// `e` in `tone`, when it comes in skin tones.
pub(crate) fn toned(e: &'static Emoji, tone: SkinTone) -> &'static Emoji {
    if tone == SkinTone::Default { e } else { e.with_skin_tone(tone).unwrap_or(e) }
}

/// The emoji whose tones `e` is one of, or `e`.
fn base(e: &'static Emoji) -> &'static Emoji {
    e.skin_tones().and_then(|mut tones| tones.next()).unwrap_or(e)
}

/// The words of a name or query: lowercase, split at anything not a letter or digit.
fn words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).map(str::to_lowercase).collect()
}

/// How well `e` matches: 0 its whole name, 1 a whole shortcode, 2 its name's start, 3 a prefix
/// of a word of its name for each query word, 4 the same against its shortcodes.
fn rank(e: &Emoji, query: &str, terms: &[String]) -> Option<u8> {
    let name = e.name().to_lowercase();
    if name == query {
        return Some(0);
    }
    if e.shortcodes().any(|c| c == query) {
        return Some(1);
    }
    if name.starts_with(query) {
        return Some(2);
    }
    let covers = |have: &[String]| terms.iter().all(|t| have.iter().any(|w| w.starts_with(t.as_str())));
    if covers(&words(&name)) {
        return Some(3);
    }
    let codes: Vec<String> = e.shortcodes().flat_map(words).collect();
    covers(&codes).then_some(4)
}

/// Every drawable emoji matching `query`, best first, then in Unicode order. An emoji typed or
/// pasted as itself finds itself; a query with no letters or digits (`+1`) matches shortcodes
/// exactly.
pub(crate) fn search(query: &str) -> Vec<&'static Emoji> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Vec::new();
    }
    if let Some(e) = emojis::get(&query) {
        return vec![base(e)];
    }
    let terms = words(&query);
    if terms.is_empty() {
        return emojis::iter().filter(drawable).filter(|e| e.shortcodes().any(|c| c == query)).collect();
    }
    let mut hits: Vec<(u8, &'static Emoji)> =
        emojis::iter().filter(drawable).filter_map(|e| rank(e, &query, &terms).map(|r| (r, e))).collect();
    hits.sort_by_key(|(r, _)| *r);
    hits.into_iter().map(|(_, e)| e).collect()
}

/// The emoji of a group, in Unicode order.
pub(crate) fn group(g: Group) -> Vec<&'static Emoji> {
    g.emojis().filter(drawable).collect()
}

/// The emoji used last, newest first, and the skin tone last picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Recents {
    pub emoji: Vec<String>,
    pub tone: SkinTone,
}

impl Default for Recents {
    fn default() -> Self {
        Self { emoji: Vec::new(), tone: SkinTone::Default }
    }
}

impl Recents {
    /// Reads what [`Recents::serialize`] wrote; lines it does not know are skipped.
    pub fn parse(text: &str) -> Self {
        let mut out = Self::default();
        for line in text.lines() {
            if let Some(tone) = line.strip_prefix("tone=") {
                out.tone = tone.parse::<usize>().ok().and_then(|i| TONES.get(i).copied()).unwrap_or(SkinTone::Default);
            } else if let Some(e) = emojis::get(line)
                && out.emoji.len() < RECENT_MAX
                && !out.emoji.iter().any(|x| x == e.as_str())
            {
                out.emoji.push(e.as_str().to_owned());
            }
        }
        out
    }

    pub fn serialize(&self) -> String {
        let tone = TONES.iter().position(|t| *t == self.tone).unwrap_or(0);
        let mut out = format!("tone={tone}\n");
        for e in &self.emoji {
            out.push_str(e);
            out.push('\n');
        }
        out
    }

    /// `emoji` was used: it moves to the front, replacing the same emoji in another tone.
    pub fn push(&mut self, emoji: &str) {
        let Some(e) = emojis::get(emoji) else { return };
        let b = base(e);
        self.emoji.retain(|x| emojis::get(x).is_none_or(|x| base(x) != b));
        self.emoji.insert(0, e.as_str().to_owned());
        self.emoji.truncate(RECENT_MAX);
    }

    pub fn list(&self) -> Vec<&'static Emoji> {
        self.emoji.iter().filter_map(|e| emojis::get(e)).collect()
    }
}

/// `$XDG_STATE_HOME/nd/emoji-recents`, shared by every nd app as GTK shares its own.
fn recents_path() -> Option<PathBuf> {
    let state = std::env::var_os("XDG_STATE_HOME")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
    Some(state.join("nd").join("emoji-recents"))
}

thread_local! {
    static RECENTS: RefCell<Option<Recents>> = const { RefCell::new(None) };
}

fn with_recents<R>(f: impl FnOnce(&mut Recents) -> R) -> R {
    RECENTS.with(|cell| {
        let mut cell = cell.borrow_mut();
        let recents = cell.get_or_insert_with(|| {
            recents_path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| Recents::parse(&t)).unwrap_or_default()
        });
        f(recents)
    })
}

fn save(recents: &Recents) {
    let Some(path) = recents_path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, recents.serialize());
}

/// What a key did to the open picker.
pub(crate) enum Outcome {
    /// Not the picker's: the field takes it.
    Pass,
    Handled,
    Close,
    Pick(&'static str),
}

/// An open picker.
pub(crate) struct Picker {
    query: String,
    tab: Tab,
    /// The keyboard's or the pointer's emoji, an index into `items`.
    highlight: Option<usize>,
    tone: SkinTone,
    tones_open: bool,
    items: Rc<Vec<&'static Emoji>>,
    scroll: UniformListScrollHandle,
}

impl Picker {
    pub fn new() -> Self {
        let (tone, has_recent) = with_recents(|r| (r.tone, !r.emoji.is_empty()));
        let tab = if has_recent { Tab::Recent } else { Tab::Group(Group::SmileysAndEmotion) };
        let mut p = Self {
            query: String::new(),
            tab,
            highlight: None,
            tone,
            tones_open: false,
            items: Rc::default(),
            scroll: UniformListScrollHandle::new(),
        };
        p.refresh();
        p
    }

    fn refresh(&mut self) {
        let items = if !self.query.trim().is_empty() {
            search(&self.query)
        } else {
            match self.tab {
                Tab::Recent => with_recents(|r| r.list()),
                Tab::Group(g) => group(g),
            }
        };
        self.items = Rc::new(items);
        self.highlight = if self.query.is_empty() || self.items.is_empty() { None } else { Some(0) };
        self.scroll.scroll_to_item(0, ScrollStrategy::Top);
    }

    /// Recents show the tone they were used in; everything else the chosen tone.
    fn shown(&self, e: &'static Emoji) -> &'static Emoji {
        if self.query.is_empty() && self.tab == Tab::Recent { e } else { toned(e, self.tone) }
    }

    fn select_tab(&mut self, tab: Tab) {
        self.tab = tab;
        self.query.clear();
        self.refresh();
    }

    fn step(&mut self, by: isize) {
        let n = self.items.len();
        if n == 0 {
            return;
        }
        let next = match self.highlight {
            Some(h) => (h as isize + by).clamp(0, n as isize - 1) as usize,
            None => 0,
        };
        self.highlight = Some(next);
        self.scroll.scroll_to_item(next / COLUMNS, ScrollStrategy::Nearest);
    }

    /// The emoji Return inserts: the highlighted one, or the first.
    fn chosen(&self) -> Option<&'static str> {
        let e = self.items.get(self.highlight.unwrap_or(0))?;
        Some(self.shown(e).as_str())
    }

    pub fn key(&mut self, ks: &Keystroke) -> Outcome {
        let m = ks.modifiers;
        if m.control || m.alt || m.platform || m.function {
            return Outcome::Pass;
        }
        let page = (COLUMNS * ROWS) as isize;
        match ks.key.as_str() {
            "escape" if !self.query.is_empty() => {
                self.query.clear();
                self.refresh();
            }
            "escape" => return Outcome::Close,
            "enter" => return self.chosen().map_or(Outcome::Handled, Outcome::Pick),
            "left" => self.step(-1),
            "right" => self.step(1),
            "up" => self.step(-(COLUMNS as isize)),
            "down" => self.step(COLUMNS as isize),
            "pageup" => self.step(-page),
            "pagedown" => self.step(page),
            "home" => self.step(-(self.items.len() as isize)),
            "end" => self.step(self.items.len() as isize),
            "tab" => {
                let i = TABS.iter().position(|t| *t == self.tab).unwrap_or(0) as isize;
                let n = TABS.len() as isize;
                let next = (i + if m.shift { -1 } else { 1 }).rem_euclid(n) as usize;
                self.select_tab(TABS[next]);
            }
            "backspace" => {
                if self.query.pop().is_some() {
                    self.refresh();
                }
            }
            _ => match ks.key_char.as_deref() {
                Some(c) if !c.is_empty() && c.chars().all(|c| !c.is_control()) => {
                    self.query.push_str(c);
                    self.refresh();
                }
                _ => return Outcome::Pass,
            },
        }
        Outcome::Handled
    }
}

/// Records `emoji` as used, for the next picker.
pub(crate) fn used(emoji: &str) {
    with_recents(|r| {
        r.push(emoji);
        save(r);
    });
}

/// Runs `f` on the open picker; false when it closed meanwhile.
pub(crate) type With = Rc<dyn Fn(&mut dyn FnMut(&mut Picker)) -> bool>;
pub(crate) type Pick = Rc<dyn Fn(&'static str, &mut Window, &mut App)>;
pub(crate) type Close = Rc<dyn Fn(&mut Window, &mut App)>;

/// What the view does back to the field that owns the picker.
pub(crate) struct Host {
    pub with: With,
    pub pick: Pick,
    pub close: Close,
}

/// The open picker as a light-dismiss popover over the window, its `corner` at `at` (window
/// coordinates); the window's edges push it back on screen.
#[allow(clippy::too_many_arguments)]
pub(crate) fn popover(
    picker: &Picker,
    host: Host,
    at: Point<Pixels>,
    corner: Anchor,
    look: FormLook,
    fluent: bool,
    priority: usize,
    swallow: impl Fn(&mut Window) + 'static,
    window: &Window,
) -> AnyElement {
    let viewport = window.viewport_size();
    let backdrop = {
        let close = host.close.clone();
        div()
            .id("nd-emoji-backdrop")
            .block_mouse_except_scroll()
            .absolute()
            .top_0()
            .left_0()
            .w(viewport.width)
            .h(viewport.height)
            .on_any_mouse_down(move |_: &MouseDownEvent, window, cx| {
                cx.stop_propagation();
                swallow(window);
                close(window, cx);
            })
    };
    let width = px(CELL * COLUMNS as f32);
    let refresh = |window: &mut Window| window.refresh();

    let search = {
        let empty = picker.query.is_empty();
        let text: SharedString = if empty { "Search Emoji".into() } else { picker.query.clone().into() };
        div()
            .id("nd-emoji-search")
            .flex_1()
            .min_w_0()
            .h(look.control_height)
            .px(look.field_padding_x)
            .flex()
            .flex_row()
            .items_center()
            .gap(look.gap)
            .rounded(look.radius)
            .bg(look.fill)
            .border_2()
            .border_color(look.focus_ring)
            .role(gpui::Role::SearchInput)
            .aria_label("Search Emoji")
            .aria_value(picker.query.clone())
            .child(glyph(Glyph::Search, look.icon_size, look.secondary_text, look.glyph_stroke))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .min_w_0()
                    .overflow_hidden()
                    .whitespace_nowrap()
                    .text_color(if empty { look.secondary_text } else { look.text })
                    .when(empty, |d| d.child(caret_bar(look)))
                    .child(text)
                    .when(!empty, |d| d.child(caret_bar(look))),
            )
    };

    let tone_button = {
        let with = host.with.clone();
        div()
            .id("nd-emoji-tone")
            .flex_none()
            .size(look.control_height)
            .flex()
            .items_center()
            .justify_center()
            .rounded(look.radius)
            .text_size(px(18.))
            .cursor(look.cursor)
            .hover(move |s| s.bg(look.fill_hover))
            .when(picker.tones_open, |d| d.bg(look.fill_active))
            .role(gpui::Role::Button)
            .aria_label("Skin Tone")
            .aria_expanded(picker.tones_open)
            .child(hand(picker.tone))
            .on_click(move |_, window, _| {
                with(&mut |p| p.tones_open = !p.tones_open);
                refresh(window);
            })
    };

    let tones = picker.tones_open.then(|| {
        TONES.iter().enumerate().fold(div().flex().flex_row().justify_between(), |row, (i, tone)| {
            let (with, tone) = (host.with.clone(), *tone);
            row.child(
                div()
                    .id(ElementId::Name(format!("nd-emoji-tone-{i}").into()))
                    .size(px(CELL))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(look.radius)
                    .text_size(px(20.))
                    .cursor(look.cursor)
                    .hover(move |s| s.bg(look.fill_hover))
                    .when(tone == picker.tone, |d| d.bg(look.fill_active))
                    .role(gpui::Role::RadioButton)
                    .aria_label(tone_name(tone))
                    .aria_selected(tone == picker.tone)
                    .child(hand(tone))
                    .on_click(move |_, window, _| {
                        with(&mut |p| {
                            p.tone = tone;
                            p.tones_open = false;
                        });
                        with_recents(|r| {
                            r.tone = tone;
                            save(r);
                        });
                        refresh(window);
                    }),
            )
        })
    });

    let tabs = TABS.iter().enumerate().fold(div().flex().flex_row().justify_between(), |row, (i, tab)| {
        let (with, tab) = (host.with.clone(), *tab);
        let active = picker.query.is_empty() && picker.tab == tab;
        row.child(
            div()
                .id(ElementId::Name(format!("nd-emoji-tab-{i}").into()))
                .size(px(TAB))
                .flex()
                .items_center()
                .justify_center()
                .rounded(look.radius)
                .cursor(look.cursor)
                .hover(move |s| s.bg(look.fill_hover))
                .when(active, |d| d.bg(look.fill_active))
                .role(gpui::Role::Tab)
                .aria_label(tab.title())
                .aria_selected(active)
                .child(glyph(
                    Glyph::of(tab.icon()),
                    look.icon_size,
                    if active { look.accent_text } else { look.secondary_text },
                    look.glyph_stroke,
                ))
                .on_click(move |_, window, _| {
                    with(&mut |p| p.select_tab(tab));
                    refresh(window);
                }),
        )
    });

    let title = if picker.query.is_empty() { picker.tab.title() } else { "Search Results" };
    let heading = div().text_size(look.text_size * 0.85).text_color(look.secondary_text).child(title);

    let items = picker.items.clone();
    let grid: AnyElement = if items.is_empty() {
        let empty = if picker.query.is_empty() { "No Recent Emoji" } else { "No Emoji Found" };
        div()
            .w(width)
            .h(px(CELL * ROWS as f32))
            .flex()
            .items_center()
            .justify_center()
            .text_color(look.secondary_text)
            .child(empty)
            .into_any_element()
    } else {
        let rows = items.len().div_ceil(COLUMNS);
        let (highlight, show_tone) = (picker.highlight, picker.query.is_empty() && picker.tab == Tab::Recent);
        let tone = picker.tone;
        let (with, pick) = (host.with.clone(), host.pick.clone());
        uniform_list("nd-emoji-grid", rows, move |range, _, _| {
            range
                .map(|row| {
                    let start = row * COLUMNS;
                    let end = (start + COLUMNS).min(items.len());
                    (start..end).fold(div().flex().flex_row(), |line, i| {
                        let e = if show_tone { items[i] } else { toned(items[i], tone) };
                        let (with, pick) = (with.clone(), pick.clone());
                        line.child(
                            div()
                                .id(ElementId::Integer(i as u64))
                                .size(px(CELL))
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded(look.radius)
                                .text_size(px(22.))
                                .cursor(look.cursor)
                                .when(highlight == Some(i), |d| d.bg(look.selection))
                                .hover(move |s| s.bg(look.fill_hover))
                                .role(gpui::Role::Button)
                                .aria_label(SharedString::from(e.name()))
                                .child(e.as_str())
                                .on_mouse_move(move |_, window, _| {
                                    let mut moved = false;
                                    with(&mut |p| {
                                        moved = p.highlight != Some(i);
                                        p.highlight = Some(i);
                                    });
                                    if moved {
                                        window.refresh();
                                    }
                                })
                                .on_click(move |_, window, cx| pick(e.as_str(), window, cx)),
                        )
                    })
                })
                .collect()
        })
        .track_scroll(&picker.scroll)
        .w(width)
        .h(px(CELL * ROWS as f32))
        .into_any_element()
    };

    let name: SharedString = picker
        .highlight
        .and_then(|h| picker.items.get(h))
        .map(|e| SharedString::from(e.name().to_owned()))
        .unwrap_or_else(|| " ".into());
    let footer = div()
        .h(look.line_height)
        .overflow_hidden()
        .whitespace_nowrap()
        .text_size(look.text_size * 0.85)
        .text_color(look.secondary_text)
        .child(name);

    let header = div().flex().flex_row().items_center().gap(look.gap).child(search).child(tone_button);
    // GTK's chooser keeps its sections along the bottom; Windows' panel puts them over the grid.
    let mut card = div()
        .id("nd-emoji-picker")
        .occlude()
        .role(gpui::Role::Dialog)
        .aria_label("Emoji")
        .p(look.popover_inset * 2.)
        .flex()
        .flex_col()
        .gap(look.gap)
        .rounded(look.popover_radius)
        .bg(look.popover)
        .border_1()
        .border_color(look.border)
        .shadow(vec![gpui::BoxShadow {
            color: gpui::hsla(0., 0., 0., 0.18),
            offset: point(px(0.), px(4.)),
            blur_radius: px(12.),
            spread_radius: px(0.),
            inset: false,
        }])
        .text_size(look.text_size)
        .text_color(look.text)
        .child(header)
        .children(tones);
    card = if fluent {
        card.child(tabs).child(heading).child(grid).child(footer)
    } else {
        card.child(heading).child(grid).child(footer).child(tabs)
    };
    let card = anchored().position(at).anchor(corner).snap_to_window_with_margin(px(8.)).child(card);
    deferred(div().child(anchored().position(point(px(0.), px(0.))).child(backdrop)).child(card))
        .with_priority(priority)
        .into_any_element()
}

/// The search box's caret: the field it serves keeps focus, so it draws one of its own.
fn caret_bar(look: FormLook) -> impl IntoElement {
    div().flex_none().w(px(1.)).h(look.line_height * 0.9).bg(look.text)
}

fn hand(tone: SkinTone) -> &'static str {
    let raised = emojis::get("\u{270b}").expect("raised hand is an emoji");
    toned(raised, tone).as_str()
}

fn tone_name(tone: SkinTone) -> &'static str {
    match tone {
        SkinTone::Light => "Light Skin Tone",
        SkinTone::MediumLight => "Medium-Light Skin Tone",
        SkinTone::Medium => "Medium Skin Tone",
        SkinTone::MediumDark => "Medium-Dark Skin Tone",
        SkinTone::Dark => "Dark Skin Tone",
        _ => "Default Skin Tone",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strs(list: &[&'static Emoji]) -> Vec<&'static str> {
        list.iter().map(|e| e.as_str()).collect()
    }

    #[test]
    fn search_ranks_the_whole_name_first_then_its_start_then_its_words() {
        let hits = search("grinning face");
        assert_eq!(hits[0].as_str(), "\u{1f600}", "the emoji named exactly that comes first");
        assert!(strs(&hits).contains(&"\u{1f603}"), "grinning face with big eyes: {:?}", &strs(&hits)[..5]);
        let cat = search("cat");
        assert_eq!(cat[0].name(), "cat", "{:?}", &strs(&cat)[..5]);
        let pos = |list: &[&'static Emoji], s: &str| list.iter().position(|e| e.as_str() == s).expect(s);
        assert!(pos(&cat, "\u{1f408}") < pos(&cat, "\u{1f63a}"), "cat before grinning cat");
    }

    #[test]
    fn search_matches_word_prefixes_in_any_order_and_shortcodes() {
        assert!(strs(&search("up thumbs")).contains(&"\u{1f44d}"));
        assert!(strs(&search("thu")).contains(&"\u{1f44d}"), "a prefix finds it");
        assert_eq!(search("+1")[0].as_str(), "\u{1f44d}", "a whole shortcode comes first");
        assert!(strs(&search("tada")).contains(&"\u{1f389}"), "a shortcode word finds party popper");
        assert!(search("zzqx").is_empty());
        assert!(search("   ").is_empty());
    }

    #[test]
    fn search_finds_an_emoji_typed_as_itself_and_ignores_case_and_tones() {
        assert_eq!(strs(&search("\u{1f44d}\u{1f3fd}")), ["\u{1f44d}"], "a toned emoji finds its base");
        assert_eq!(search("CAT")[0].name(), "cat");
        assert!(
            search("hand").iter().all(|e| e.skin_tone().is_none_or(|t| t == SkinTone::Default)),
            "one entry per emoji"
        );
    }

    #[test]
    fn search_leaves_out_emoji_newer_than_the_fonts() {
        assert!(emojis::iter().any(|e| e.emoji_version() > NEWEST), "the data has newer emoji to leave out");
        assert!(search("face").iter().all(|e| e.emoji_version() <= NEWEST));
    }

    #[test]
    fn every_group_has_emoji_and_tones_apply() {
        for tab in TABS {
            if let Tab::Group(g) = tab {
                assert!(group(g).len() > 50, "{g:?}");
            }
        }
        let thumbs = emojis::get("\u{1f44d}").unwrap();
        assert_eq!(toned(thumbs, SkinTone::Dark).as_str(), "\u{1f44d}\u{1f3ff}");
        let cat = emojis::get("\u{1f408}").unwrap();
        assert_eq!(toned(cat, SkinTone::Dark).as_str(), "\u{1f408}", "no tones, no change");
    }

    #[test]
    fn recents_put_the_newest_first_once_and_cap() {
        let mut r = Recents::default();
        r.push("\u{1f600}");
        r.push("\u{1f389}");
        r.push("\u{1f600}");
        assert_eq!(r.emoji, ["\u{1f600}", "\u{1f389}"]);
        r.push("not an emoji");
        assert_eq!(r.emoji.len(), 2, "text that is no emoji is ignored");
        for e in emojis::iter().take(RECENT_MAX + 5) {
            r.push(e.as_str());
        }
        assert_eq!(r.emoji.len(), RECENT_MAX);
        assert_eq!(r.emoji[0], emojis::iter().nth(RECENT_MAX + 4).unwrap().as_str());
    }

    #[test]
    fn recents_keep_one_tone_of_an_emoji() {
        let mut r = Recents::default();
        r.push("\u{1f44d}");
        r.push("\u{1f389}");
        r.push("\u{1f44d}\u{1f3fd}");
        assert_eq!(r.emoji, ["\u{1f44d}\u{1f3fd}", "\u{1f389}"], "the toned use replaces the plain one");
        assert_eq!(strs(&r.list()), ["\u{1f44d}\u{1f3fd}", "\u{1f389}"]);
    }

    #[test]
    fn recents_round_trip_and_survive_junk() {
        let mut r = Recents { tone: SkinTone::MediumDark, ..Recents::default() };
        r.push("\u{1f389}");
        r.push("\u{2764}\u{fe0f}");
        let back = Recents::parse(&r.serialize());
        assert_eq!(back, r);
        let junk = Recents::parse("tone=99\nhello\n\u{1f389}\n\u{1f389}\n");
        assert_eq!(junk.tone, SkinTone::Default);
        assert_eq!(junk.emoji, ["\u{1f389}"]);
    }

    fn key(k: &str) -> Keystroke {
        Keystroke::parse(k).unwrap()
    }

    fn typed(c: &str) -> Keystroke {
        let mut ks = key(c);
        ks.key_char = Some(c.to_owned());
        ks
    }

    fn picker() -> Picker {
        RECENTS.with(|cell| *cell.borrow_mut() = Some(Recents::default()));
        Picker::new()
    }

    #[test]
    fn typing_searches_and_return_picks_the_top_hit() {
        let mut p = picker();
        assert_eq!(p.tab, Tab::Group(Group::SmileysAndEmotion), "no recents: smileys first");
        for c in ["c", "a", "t"] {
            assert!(matches!(p.key(&typed(c)), Outcome::Handled));
        }
        assert_eq!(p.query, "cat");
        assert!(matches!(p.key(&key("enter")), Outcome::Pick("\u{1f408}")));
        assert!(matches!(p.key(&key("backspace")), Outcome::Handled));
        assert_eq!(p.query, "ca");
        assert!(matches!(p.key(&key("escape")), Outcome::Handled), "escape clears the search first");
        assert!(p.query.is_empty());
        assert!(matches!(p.key(&key("escape")), Outcome::Close));
        assert!(matches!(p.key(&key("ctrl-c")), Outcome::Pass), "shortcuts stay the field's");
    }

    #[test]
    fn arrows_move_by_cell_and_row_and_tab_cycles_groups() {
        let mut p = picker();
        p.key(&key("right"));
        assert_eq!(p.highlight, Some(0), "the first arrow lands on the first emoji");
        p.key(&key("right"));
        p.key(&key("down"));
        assert_eq!(p.highlight, Some(1 + COLUMNS));
        p.key(&key("up"));
        p.key(&key("up"));
        assert_eq!(p.highlight, Some(0), "clamped at the top");
        let first = p.items[1];
        p.key(&key("right"));
        assert!(matches!(p.key(&key("enter")), Outcome::Pick(e) if e == first.as_str()));
        p.key(&key("tab"));
        assert_eq!(p.tab, Tab::Group(Group::PeopleAndBody));
        p.key(&key("shift-tab"));
        p.key(&key("shift-tab"));
        assert_eq!(p.tab, Tab::Recent);
        assert!(p.items.is_empty());
        assert!(matches!(p.key(&key("enter")), Outcome::Handled), "nothing to pick");
    }

    #[test]
    fn the_tone_applies_to_picks_outside_recents() {
        let mut p = picker();
        p.tone = SkinTone::Medium;
        for c in ["t", "h", "u", "m", "b", "s", " ", "u", "p"] {
            p.key(&typed(c));
        }
        assert!(matches!(p.key(&key("enter")), Outcome::Pick("\u{1f44d}\u{1f3fd}")));
    }
}
