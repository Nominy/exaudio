//! The language switch for status lines and refusals. The language is process-wide, so this
//! file has a single test: no other test in its binary can observe the switch.

mod edit_support;

use edit_support::*;
use exaudio_editor::i18n::set_russian;
use exaudio_editor::localized_format;

#[test]
fn statuses_and_refusals_follow_the_language() {
    let mut doc = read_only_doc(vec![seg("a", "lane-1", 0.0, 2.0, "one")]);
    let count = 3;

    let refusal = doc.delete_segment("a").unwrap_err().to_string();
    assert_eq!(refusal, "Delete unavailable: project is read-only.");
    assert_eq!(
        localized_format!("{count} of {}", "{count} из {}", 7),
        "3 of 7"
    );

    set_russian(true);

    let refusal = doc.delete_segment("a").unwrap_err().to_string();
    assert_eq!(refusal, "Удаление: недоступно, проект только для чтения.");
    assert_eq!(
        localized_format!("{count} of {}", "{count} из {}", 7),
        "3 из 7"
    );
}
