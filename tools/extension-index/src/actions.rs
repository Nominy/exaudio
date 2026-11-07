//! `contributes.actions` as `inspect` and `validate-package` show them: one row per
//! action with its English title, default chords and menu placements. Validation itself is
//! `ExtensionManifest::validate` (ids, `when` clauses, chords), which every catalog parse
//! and package check runs.
use exaudio_extension::ExtensionManifest;
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ActionSummary {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    pub palette: bool,
    /// `key` or `key (mac: chord)`, each with its own `when` when present.
    pub keys: Vec<String>,
    /// `menu/group[@order]` with its `when` when present.
    pub menus: Vec<String>,
}

#[derive(Serialize)]
pub struct ReleaseActions<'a> {
    pub id: &'a str,
    pub version: String,
    pub actions: Vec<ActionSummary>,
}

pub fn summarize(manifest: &ExtensionManifest) -> Vec<ActionSummary> {
    manifest
        .contributes
        .actions
        .iter()
        .map(|action| ActionSummary {
            id: action.id.clone(),
            title: action.title.resolve("en").to_owned(),
            when: action.when.clone(),
            palette: action.palette,
            keys: action
                .keys
                .iter()
                .map(|binding| {
                    let mut text = binding.key.clone();
                    if let Some(mac) = &binding.mac {
                        text.push_str(&format!(" (mac: {mac})"));
                    }
                    if let Some(when) = &binding.when {
                        text.push_str(&format!(" when {when}"));
                    }
                    text
                })
                .collect(),
            menus: action
                .menus
                .iter()
                .map(|placement| {
                    let mut text = format!("{}/{}", placement.menu, placement.group);
                    if placement.order != 0 {
                        text.push_str(&format!("@{}", placement.order));
                    }
                    if let Some(when) = &placement.when {
                        text.push_str(&format!(" when {when}"));
                    }
                    text
                })
                .collect(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summary_lists_ids_keys_and_menus() {
        let manifest: ExtensionManifest = serde_json::from_str(include_str!(
            "../../../extensions/examples/hello-panel/manifest.json"
        ))
        .unwrap();
        manifest.validate().unwrap();
        let summary = summarize(&manifest);
        assert_eq!(
            summary,
            vec![ActionSummary {
                id: "example.hello-panel.greet".into(),
                title: "Insert greeting".into(),
                when: Some("editorOpen".into()),
                palette: true,
                keys: vec!["ctrl-alt-g".into()],
                menus: vec!["editor/overflow/example".into()],
            }]
        );
    }
}
