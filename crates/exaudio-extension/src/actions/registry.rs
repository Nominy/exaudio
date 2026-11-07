//! One registry for everything a user can invoke. Menus, toolbar, keyboard, hotkeys dialog
//! and the command palette are views over it.
//!
//! Registration is atomic per owner; every index row references an existing [`Entry`];
//! key resolution is a chord lookup, never a scan.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use thiserror::Error;

use super::keys::{KeyChord, legacy_chord};
use super::manifest::{ActionManifest, KeyBinding, Localized, MenuEntry};
use super::when::{Context, When};
use crate::manifest::{ManifestError, valid_id};

pub type ActionId = String;

/// Owner id of the shell and editor's own actions.
pub const CORE_OWNER: &str = "exaudio.core";

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Core,
    Extension,
}

/// A registered action: manifest metadata with parsed clauses.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: ActionId,
    pub owner: String,
    pub owner_name: String,
    pub title: Localized,
    pub category: Option<Localized>,
    pub when: Option<When>,
    pub palette: bool,
    pub keys: Vec<KeyBinding>,
    pub menus: Vec<MenuEntry>,
    pub priority: Priority,
}

impl Entry {
    pub fn title_in(&self, lang: &str) -> &str {
        self.title.resolve(lang)
    }

    /// Category in `lang` (falling back to `en`), or the owner's display name.
    pub fn category_in(&self, lang: &str) -> &str {
        self.category
            .as_ref()
            .map_or(self.owner_name.as_str(), |category| category.resolve(lang))
    }

    /// Whether the action's own `when` holds (missing clause → always).
    pub fn holds_in(&self, ctx: &Context) -> bool {
        self.when.as_ref().is_none_or(|when| when.eval(ctx))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum RegistryError {
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("owner `{0}` is already registered")]
    OwnerRegistered(String),
    #[error("unknown action `{0}`")]
    UnknownAction(String),
}

/// Outcome of a key press.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolution {
    pub chosen: Option<ActionId>,
    /// Lower-precedence candidates that would also have fired, best first.
    pub shadowed: Vec<ActionId>,
}

/// A chord with several candidates whose clauses can hold at the same time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Conflict {
    /// Chord resolved for the platform the conflicts were computed for.
    pub chord: KeyChord,
    pub actions: Vec<ActionId>,
}

/// An action placed in a menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MenuItem<'a> {
    pub entry: &'a Entry,
    pub group: &'a str,
    pub order: i32,
}

/// A user's rebinding of one action.
#[derive(Clone, Debug, PartialEq)]
pub struct UserBinding {
    pub action: ActionId,
    pub key: KeyChord,
    pub when: Option<When>,
}

/// User overrides keyed by action id. Overrides replace an action's default bindings;
/// `removed` actions have no bindings at all.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UserBindings {
    pub bindings: Vec<UserBinding>,
    pub removed: BTreeSet<ActionId>,
}

#[derive(Serialize, Deserialize)]
struct UserBindingsFile {
    #[serde(default)]
    bindings: Vec<UserBindingFile>,
    #[serde(default)]
    removed: Vec<String>,
}

#[derive(Serialize, Deserialize)]
struct UserBindingFile {
    action: String,
    key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    when: Option<String>,
}

/// Action id for an `EditorAction` id persisted by the previous keymap
/// (`deleteSelectedSegment` → `exaudio.core.delete-selected-segment`).
pub fn legacy_action_id(editor_action: &str) -> ActionId {
    let mut id = String::with_capacity(CORE_OWNER.len() + 1 + editor_action.len() + 4);
    id.push_str(CORE_OWNER);
    id.push('.');
    for c in editor_action.chars() {
        if c.is_ascii_uppercase() {
            id.push('-');
            id.push(c.to_ascii_lowercase());
        } else {
            id.push(c);
        }
    }
    id
}

impl UserBindings {
    /// Loads persisted overrides. Accepts the current `{ bindings, removed }` document and
    /// the previous per-`EditorAction` maps (`{"<id>": "<key>"}` and
    /// `{"<id>": {"code": "KeyK", "modifiers": ["alt"]}}`). Entries that do not parse are
    /// dropped; a malformed document yields no overrides.
    pub fn from_json(source: &str) -> Self {
        let Ok(Json::Object(root)) = serde_json::from_str::<Json>(source) else {
            return Self::default();
        };
        if root.contains_key("bindings") || root.contains_key("removed") {
            let Ok(file) = serde_json::from_value::<UserBindingsFile>(Json::Object(root)) else {
                return Self::default();
            };
            let bindings = file
                .bindings
                .into_iter()
                .filter_map(|binding| {
                    let key = KeyChord::parse(&binding.key)?;
                    let when = binding
                        .when
                        .map(|clause| When::parse(&clause))
                        .transpose()
                        .ok()?;
                    Some(UserBinding {
                        action: binding.action,
                        key,
                        when,
                    })
                })
                .collect();
            return Self {
                bindings,
                removed: file.removed.into_iter().collect(),
            };
        }
        let bindings = root
            .into_iter()
            .filter_map(|(id, value)| {
                let key = match &value {
                    Json::String(keystroke) => KeyChord::parse(keystroke),
                    Json::Object(_) => legacy_chord(&value),
                    _ => None,
                }?;
                Some(UserBinding {
                    action: legacy_action_id(&id),
                    key,
                    when: None,
                })
            })
            .collect();
        Self {
            bindings,
            removed: BTreeSet::new(),
        }
    }

    /// Serializes in the current format.
    pub fn to_json(&self) -> String {
        let file = UserBindingsFile {
            bindings: self
                .bindings
                .iter()
                .map(|binding| UserBindingFile {
                    action: binding.action.clone(),
                    key: binding.key.to_string(),
                    when: binding.when.as_ref().map(ToString::to_string),
                })
                .collect(),
            removed: self.removed.iter().cloned().collect(),
        };
        serde_json::to_string(&file).expect("user bindings serialize")
    }

    pub fn for_action<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a UserBinding> + 'a {
        self.bindings
            .iter()
            .filter(move |binding| binding.action == id)
    }

    /// Whether the user changed this action's bindings (rebound or removed).
    pub fn is_overridden(&self, id: &str) -> bool {
        self.removed.contains(id) || self.for_action(id).next().is_some()
    }
}

struct Owner {
    name: String,
    enabled: bool,
    actions: Vec<ActionId>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum BindingSource {
    User,
    Default,
}

struct BindingRef {
    action: ActionId,
    source: BindingSource,
    /// Position within the action's binding list; earlier wins among equals.
    order: usize,
    when: Option<When>,
}

struct MenuRef {
    action: ActionId,
    placement: usize,
}

struct SearchRow {
    lang: String,
    title: String,
    category: String,
}

pub struct ActionRegistry<H> {
    actions: BTreeMap<ActionId, Entry>,
    handlers: HashMap<ActionId, H>,
    owners: BTreeMap<String, Owner>,
    /// Resolved chord → candidates, one index per platform (`[non-mac, mac]`).
    keys: [HashMap<KeyChord, Vec<BindingRef>>; 2],
    /// Menu id → placements sorted by (group, order, action id).
    menus: BTreeMap<String, Vec<MenuRef>>,
    /// Lowercased title and category per action per language.
    search: BTreeMap<ActionId, Vec<SearchRow>>,
    user: UserBindings,
}

impl<H> Default for ActionRegistry<H> {
    fn default() -> Self {
        Self {
            actions: BTreeMap::new(),
            handlers: HashMap::new(),
            owners: BTreeMap::new(),
            keys: [HashMap::new(), HashMap::new()],
            menus: BTreeMap::new(),
            search: BTreeMap::new(),
            user: UserBindings::default(),
        }
    }
}

impl<H> ActionRegistry<H> {
    pub fn new() -> Self {
        Self::default()
    }

    // ---- owners ---------------------------------------------------------------------------

    /// Registers every action of `owner` or nothing: all actions must compile against the
    /// owner prefix and no id may already be registered.
    pub fn register_owner(
        &mut self,
        owner: &str,
        owner_name: &str,
        actions: Vec<ActionManifest>,
        priority: Priority,
    ) -> Result<(), RegistryError> {
        if !valid_id(owner) {
            return Err(ManifestError::InvalidId(owner.to_string()).into());
        }
        if self.owners.contains_key(owner) {
            return Err(RegistryError::OwnerRegistered(owner.to_string()));
        }
        let mut entries = Vec::with_capacity(actions.len());
        let mut seen = BTreeSet::new();
        for action in actions {
            let compiled = action.compile(owner)?;
            if self.actions.contains_key(&action.id) || !seen.insert(action.id.clone()) {
                return Err(ManifestError::DuplicateAction(action.id).into());
            }
            entries.push(Entry {
                id: action.id,
                owner: owner.to_string(),
                owner_name: owner_name.to_string(),
                title: action.title,
                category: action.category,
                when: compiled.when,
                palette: action.palette,
                keys: compiled.keys,
                menus: compiled.menus,
                priority,
            });
        }
        self.owners.insert(
            owner.to_string(),
            Owner {
                name: owner_name.to_string(),
                enabled: true,
                actions: entries.iter().map(|entry| entry.id.clone()).collect(),
            },
        );
        for entry in entries {
            let id = entry.id.clone();
            self.actions.insert(id.clone(), entry);
            self.index_keys(&id);
            self.index_menus(&id);
            self.index_search(&id);
        }
        Ok(())
    }

    /// Removes the owner, its actions, handlers and every index row. Returns whether the
    /// owner was registered.
    pub fn unregister_owner(&mut self, owner: &str) -> bool {
        let Some(removed) = self.owners.remove(owner) else {
            return false;
        };
        for id in removed.actions {
            self.unindex_keys(&id);
            self.unindex_menus(&id);
            self.search.remove(&id);
            self.handlers.remove(&id);
            self.actions.remove(&id);
        }
        true
    }

    /// Disabled owners keep their entries but are hidden everywhere and never fire.
    pub fn set_owner_enabled(&mut self, owner: &str, enabled: bool) {
        if let Some(owner) = self.owners.get_mut(owner) {
            owner.enabled = enabled;
        }
    }

    pub fn is_owner_enabled(&self, owner: &str) -> bool {
        self.owners.get(owner).is_some_and(|owner| owner.enabled)
    }

    pub fn owner_of(&self, id: &str) -> Option<&str> {
        self.actions.get(id).map(|entry| entry.owner.as_str())
    }

    pub fn owner_name(&self, owner: &str) -> Option<&str> {
        self.owners.get(owner).map(|owner| owner.name.as_str())
    }

    // ---- handlers -------------------------------------------------------------------------

    pub fn bind(&mut self, id: &str, handler: H) -> Result<(), RegistryError> {
        if !self.actions.contains_key(id) {
            return Err(RegistryError::UnknownAction(id.to_string()));
        }
        self.handlers.insert(id.to_string(), handler);
        Ok(())
    }

    pub fn unbind(&mut self, id: &str) -> Option<H> {
        self.handlers.remove(id)
    }

    pub fn is_bound(&self, id: &str) -> bool {
        self.handlers.contains_key(id)
    }

    // ---- queries --------------------------------------------------------------------------

    pub fn entry(&self, id: &str) -> Option<&Entry> {
        self.actions.get(id)
    }

    /// Every registered action in id order, including disabled owners' actions.
    pub fn actions(&self) -> impl Iterator<Item = &Entry> {
        self.actions.values()
    }

    /// Actions of enabled owners whose `when` holds in `ctx`, in id order.
    pub fn actions_in<'a>(&'a self, ctx: &'a Context) -> impl Iterator<Item = &'a Entry> + 'a {
        self.actions
            .values()
            .filter(move |entry| self.is_active(entry, ctx))
    }

    /// Items of `menu_id` whose placement and action clauses hold, sorted by
    /// (group, order, id).
    pub fn menu<'a>(&'a self, menu_id: &str, ctx: &Context) -> Vec<MenuItem<'a>> {
        let Some(refs) = self.menus.get(menu_id) else {
            return Vec::new();
        };
        refs.iter()
            .filter_map(|row| {
                let entry = &self.actions[&row.action];
                let placement = &entry.menus[row.placement];
                let holds = self.is_active(entry, ctx)
                    && placement.when.as_ref().is_none_or(|when| when.eval(ctx));
                holds.then_some(MenuItem {
                    entry,
                    group: &placement.group,
                    order: placement.order,
                })
            })
            .collect()
    }

    /// Resolves a key press. Candidates come from the chord index; precedence is user
    /// override › core › extension, then binding order, then id.
    pub fn resolve(&self, chord: &KeyChord, ctx: &Context, mac: bool) -> Resolution {
        let chord = chord.resolved(mac);
        let Some(refs) = self.keys[usize::from(mac)].get(&chord) else {
            return Resolution::default();
        };
        let mut candidates: Vec<(u8, usize, &str)> = refs
            .iter()
            .filter(|row| self.binding_active(row, ctx))
            .map(|row| {
                let rank = match (row.source, self.actions[&row.action].priority) {
                    (BindingSource::User, _) => 0,
                    (BindingSource::Default, Priority::Core) => 1,
                    (BindingSource::Default, Priority::Extension) => 2,
                };
                (rank, row.order, row.action.as_str())
            })
            .collect();
        candidates.sort_unstable();
        let mut ordered: Vec<ActionId> = Vec::with_capacity(candidates.len());
        for (_, _, id) in candidates {
            if ordered.iter().all(|seen| seen != id) {
                ordered.push(id.to_string());
            }
        }
        let mut ordered = ordered.into_iter();
        Resolution {
            chosen: ordered.next(),
            shadowed: ordered.collect(),
        }
    }

    /// Chords of enabled owners with more than one action whose clauses can hold together.
    pub fn conflicts(&self, mac: bool) -> Vec<Conflict> {
        let mut conflicts = Vec::new();
        for (chord, refs) in &self.keys[usize::from(mac)] {
            let refs: Vec<&BindingRef> = refs
                .iter()
                .filter(|row| self.is_owner_enabled(&self.actions[&row.action].owner))
                .collect();
            let mut actions = BTreeSet::new();
            for (index, first) in refs.iter().enumerate() {
                for second in &refs[index + 1..] {
                    if first.action != second.action && self.may_coexist(first, second) {
                        actions.insert(first.action.clone());
                        actions.insert(second.action.clone());
                    }
                }
            }
            if actions.len() > 1 {
                conflicts.push(Conflict {
                    chord: chord.clone(),
                    actions: actions.into_iter().collect(),
                });
            }
        }
        conflicts.sort_by(|a, b| a.chord.cmp(&b.chord));
        conflicts
    }

    /// Active actions whose title or category (in `lang`, falling back to `en`) contains
    /// every whitespace-separated term of `query`, case-insensitively. Ranked: title starts
    /// with the query › every term starts a word › substring; ties in id order.
    pub fn search(&self, query: &str, lang: &str, ctx: &Context) -> Vec<&Entry> {
        let query = query.trim().to_lowercase();
        let terms: Vec<&str> = query.split_whitespace().collect();
        let mut hits: Vec<(u8, &Entry)> = Vec::new();
        for (id, rows) in &self.search {
            let entry = &self.actions[id];
            if !self.is_active(entry, ctx) {
                continue;
            }
            let Some(row) = rows
                .iter()
                .find(|row| row.lang == lang)
                .or_else(|| rows.iter().find(|row| row.lang == "en"))
            else {
                continue;
            };
            let matches = terms
                .iter()
                .all(|term| row.title.contains(term) || row.category.contains(term));
            if !matches {
                continue;
            }
            let rank = if row.title.starts_with(&query) {
                0
            } else if terms
                .iter()
                .all(|term| word_starts(&row.title, term) || word_starts(&row.category, term))
            {
                1
            } else {
                2
            };
            hits.push((rank, entry));
        }
        hits.sort_by_key(|(rank, _)| *rank);
        hits.into_iter().map(|(_, entry)| entry).collect()
    }

    // ---- user bindings --------------------------------------------------------------------

    pub fn user(&self) -> &UserBindings {
        &self.user
    }

    /// Replaces all user overrides (e.g. after loading from storage).
    pub fn set_user(&mut self, user: UserBindings) {
        self.user = user;
        self.reindex_all_keys();
    }

    /// `Some(chord)` replaces the action's bindings with this one and un-removes it;
    /// `None` drops the user's rebinding (defaults apply again unless removed).
    pub fn set_user_binding(&mut self, id: &str, chord: Option<KeyChord>, when: Option<When>) {
        self.user.bindings.retain(|binding| binding.action != id);
        if let Some(key) = chord {
            self.user.removed.remove(id);
            self.user.bindings.push(UserBinding {
                action: id.to_string(),
                key,
                when,
            });
        }
        self.reindex_keys(id);
    }

    /// The action gets no bindings at all.
    pub fn remove_default(&mut self, id: &str) {
        self.user.bindings.retain(|binding| binding.action != id);
        self.user.removed.insert(id.to_string());
        self.reindex_keys(id);
    }

    /// Restores the action's default bindings.
    pub fn reset(&mut self, id: &str) {
        self.user.bindings.retain(|binding| binding.action != id);
        self.user.removed.remove(id);
        self.reindex_keys(id);
    }

    pub fn reset_all(&mut self) {
        self.user = UserBindings::default();
        self.reindex_all_keys();
    }

    /// Effective chords of a registered action on the platform (unresolved, `secondary`
    /// kept symbolic): user override, else defaults; none when removed.
    pub fn bindings_for(&self, id: &str, mac: bool) -> Vec<KeyChord> {
        let Some(entry) = self.actions.get(id) else {
            return Vec::new();
        };
        if self.user.removed.contains(id) {
            return Vec::new();
        }
        let user: Vec<KeyChord> = self
            .user
            .for_action(id)
            .map(|binding| binding.key.clone())
            .collect();
        if !user.is_empty() {
            return user;
        }
        entry
            .keys
            .iter()
            .map(|binding| binding.chord(mac).clone())
            .collect()
    }

    // ---- internals ------------------------------------------------------------------------

    fn is_active(&self, entry: &Entry, ctx: &Context) -> bool {
        self.is_owner_enabled(&entry.owner) && entry.holds_in(ctx)
    }

    fn binding_active(&self, row: &BindingRef, ctx: &Context) -> bool {
        self.is_active(&self.actions[&row.action], ctx)
            && row.when.as_ref().is_none_or(|when| when.eval(ctx))
    }

    fn may_coexist(&self, first: &BindingRef, second: &BindingRef) -> bool {
        let mine = self.clauses_of(first);
        let theirs = self.clauses_of(second);
        mine.iter().all(|a| theirs.iter().all(|b| a.compatible(b)))
    }

    /// Action clause plus binding clause: both must hold for the binding to fire.
    fn clauses_of<'a>(&'a self, row: &'a BindingRef) -> Vec<&'a When> {
        self.actions[&row.action]
            .when
            .iter()
            .chain(row.when.iter())
            .collect()
    }

    fn index_keys(&mut self, id: &str) {
        let Self {
            actions,
            keys,
            user,
            ..
        } = self;
        let Some(entry) = actions.get(id) else {
            return;
        };
        if user.removed.contains(id) {
            return;
        }
        let mut rows: Vec<(usize, BindingSource, &KeyChord, &Option<When>)> = user
            .for_action(id)
            .enumerate()
            .map(|(order, binding)| (order, BindingSource::User, &binding.key, &binding.when))
            .collect();
        let mut insert = |mac: bool,
                          order: usize,
                          source: BindingSource,
                          chord: &KeyChord,
                          when: &Option<When>| {
            keys[usize::from(mac)]
                .entry(chord.resolved(mac))
                .or_default()
                .push(BindingRef {
                    action: id.to_string(),
                    source,
                    order,
                    when: when.clone(),
                });
        };
        if rows.is_empty() {
            for (order, binding) in entry.keys.iter().enumerate() {
                insert(
                    false,
                    order,
                    BindingSource::Default,
                    &binding.key,
                    &binding.when,
                );
                insert(
                    true,
                    order,
                    BindingSource::Default,
                    binding.chord(true),
                    &binding.when,
                );
            }
        } else {
            for (order, source, chord, when) in rows.drain(..) {
                insert(false, order, source, chord, when);
                insert(true, order, source, chord, when);
            }
        }
    }

    fn unindex_keys(&mut self, id: &str) {
        for index in &mut self.keys {
            index.retain(|_, rows| {
                rows.retain(|row| row.action != id);
                !rows.is_empty()
            });
        }
    }

    fn reindex_keys(&mut self, id: &str) {
        self.unindex_keys(id);
        self.index_keys(id);
    }

    fn reindex_all_keys(&mut self) {
        self.keys = [HashMap::new(), HashMap::new()];
        let ids: Vec<ActionId> = self.actions.keys().cloned().collect();
        for id in &ids {
            self.index_keys(id);
        }
    }

    fn index_menus(&mut self, id: &str) {
        let Self { actions, menus, .. } = self;
        let Some(entry) = actions.get(id) else {
            return;
        };
        for (placement, menu) in entry.menus.iter().enumerate() {
            let rows = menus.entry(menu.menu.clone()).or_default();
            rows.push(MenuRef {
                action: id.to_string(),
                placement,
            });
            rows.sort_by_cached_key(|row| {
                let placement = &actions[&row.action].menus[row.placement];
                (placement.group.clone(), placement.order, row.action.clone())
            });
        }
    }

    fn unindex_menus(&mut self, id: &str) {
        self.menus.retain(|_, rows| {
            rows.retain(|row| row.action != id);
            !rows.is_empty()
        });
    }

    fn index_search(&mut self, id: &str) {
        let Some(entry) = self.actions.get(id) else {
            return;
        };
        let languages: BTreeSet<&str> = entry
            .title
            .languages()
            .chain(entry.category.iter().flat_map(Localized::languages))
            .chain(std::iter::once("en"))
            .collect();
        let rows = languages
            .into_iter()
            .map(|lang| SearchRow {
                lang: lang.to_string(),
                title: entry.title_in(lang).to_lowercase(),
                category: entry.category_in(lang).to_lowercase(),
            })
            .collect();
        self.search.insert(id.to_string(), rows);
    }

    /// Walks every index and asserts each row references a live entry with consistent
    /// owner, binding and search state.
    #[cfg(test)]
    pub fn check_invariants(&self) {
        for (owner_id, owner) in &self.owners {
            for id in &owner.actions {
                let entry = self
                    .actions
                    .get(id)
                    .expect("owner lists a registered action");
                assert_eq!(&entry.owner, owner_id, "entry owner matches its owner list");
                assert_eq!(entry.owner_name, owner.name);
            }
        }
        for (id, entry) in &self.actions {
            let owner = self
                .owners
                .get(&entry.owner)
                .expect("entry owner is registered");
            assert!(owner.actions.contains(id), "owner lists every entry");
            let rows = self.search.get(id).expect("every entry has search rows");
            assert!(
                rows.iter().any(|row| row.lang == "en"),
                "search has an en row"
            );
        }
        for id in self.search.keys().chain(self.handlers.keys()) {
            assert!(
                self.actions.contains_key(id),
                "search/handler row references an entry"
            );
        }
        for index in &self.keys {
            for rows in index.values() {
                assert!(!rows.is_empty(), "no empty chord buckets");
                for row in rows {
                    assert!(
                        self.actions.contains_key(&row.action),
                        "key row references an entry"
                    );
                    assert!(
                        !self.user.removed.contains(&row.action),
                        "removed actions have no rows"
                    );
                    let overridden = self.user.for_action(&row.action).next().is_some();
                    assert_eq!(
                        row.source == BindingSource::User,
                        overridden,
                        "user override replaces default rows"
                    );
                }
            }
        }
        for rows in self.menus.values() {
            assert!(!rows.is_empty(), "no empty menus");
            let keys: Vec<_> = rows
                .iter()
                .map(|row| {
                    let entry = self
                        .actions
                        .get(&row.action)
                        .expect("menu row references an entry");
                    let placement = entry.menus.get(row.placement).expect("placement exists");
                    (placement.group.clone(), placement.order, row.action.clone())
                })
                .collect();
            assert!(
                keys.windows(2).all(|pair| pair[0] <= pair[1]),
                "menu rows sorted"
            );
        }
    }
}

impl<H: Clone> ActionRegistry<H> {
    pub fn handler(&self, id: &str) -> Option<H> {
        self.handlers.get(id).cloned()
    }
}

fn word_starts(text: &str, term: &str) -> bool {
    text.split(|c: char| !c.is_alphanumeric())
        .any(|word| word.starts_with(term))
}
