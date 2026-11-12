//! Transcript time-cell editing: begin/validate/commit rules for start and end cells.

use super::format::{format_time, parse_time};
use crate::i18n::localized;

/// Which boundary a time cell edits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TimeField {
    Start,
    End,
}

impl TimeField {
    pub fn value_from(self, start_seconds: f64, end_seconds: f64) -> f64 {
        match self {
            Self::Start => start_seconds,
            Self::End => end_seconds,
        }
    }

    /// The boundary's name inside a status line ("… start time …"); the Russian form is the
    /// genitive that follows «время».
    pub fn label(self) -> &'static str {
        match self {
            Self::Start => localized("start", "начала"),
            Self::End => localized("end", "конца"),
        }
    }
}

/// Per-row time-cell edit state (one active field at a time).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TimeEditState {
    pub active_field: Option<TimeField>,
    pub draft_value: String,
    pub error: Option<TimeEditError>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeRangeEdit {
    pub start_seconds: f64,
    pub end_seconds: f64,
}

/// Why a typed timestamp was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeEditError {
    InvalidFormat,
    Negative,
    StartNotBeforeEnd,
    EndNotAfterStart,
}

impl std::fmt::Display for TimeEditError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::InvalidFormat => localized("Invalid format", "Неверный формат"),
            Self::Negative => localized(
                "Timestamp must be >= 00:00.00",
                "Время должно быть не меньше 00:00.00",
            ),
            Self::StartNotBeforeEnd => localized(
                "Start must be earlier than end",
                "Начало должно быть раньше конца",
            ),
            Self::EndNotAfterStart => localized(
                "End must be later than start",
                "Конец должен быть позже начала",
            ),
        })
    }
}

impl std::error::Error for TimeEditError {}

/// Inputs for committing a time cell (Enter or blur).
#[derive(Debug, Clone, PartialEq)]
pub struct CommitTimeCellEdit {
    pub active_field: Option<TimeField>,
    pub field: TimeField,
    pub draft_value: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub from_blur: bool,
}

/// Result of committing a time cell.
#[derive(Debug, Clone, PartialEq)]
pub enum CommitTimeCellResult {
    /// The field was not being edited; nothing to do.
    Ignored,
    Handled {
        /// Field that stays in edit mode (`Some` only for an invalid Enter commit).
        active_field: Option<TimeField>,
        /// Value to show in the cell.
        draft_value: String,
        /// Error to show next to the cell (invalid Enter commit only).
        error: Option<TimeEditError>,
        /// Boundaries to apply to the segment when the value was valid.
        next_times: Option<TimeRangeEdit>,
    },
}

/// Begins editing `field` whose current value is `value` seconds.
pub fn start_time_cell_edit(field: TimeField, value: f64) -> TimeEditState {
    TimeEditState {
        active_field: Some(field),
        draft_value: format_time(value),
        error: None,
    }
}

/// Commit rules: a valid value always applies and leaves edit mode; an invalid value keeps the
/// typed text and reports an error on Enter, but silently reverts to the stored value on blur.
pub fn commit_time_cell_edit(input: CommitTimeCellEdit) -> CommitTimeCellResult {
    if input.active_field != Some(input.field) {
        return CommitTimeCellResult::Ignored;
    }

    let range = TimeRangeEdit {
        start_seconds: input.start_seconds,
        end_seconds: input.end_seconds,
    };
    match validate_time_edit(input.field, &input.draft_value, range) {
        Err(_) if input.from_blur => CommitTimeCellResult::Handled {
            active_field: None,
            draft_value: format_time(
                input
                    .field
                    .value_from(range.start_seconds, range.end_seconds),
            ),
            error: None,
            next_times: None,
        },
        Err(error) => CommitTimeCellResult::Handled {
            active_field: Some(input.field),
            draft_value: input.draft_value,
            error: Some(error),
            next_times: None,
        },
        Ok(next_times) => CommitTimeCellResult::Handled {
            active_field: None,
            draft_value: input.draft_value,
            error: None,
            next_times: Some(next_times),
        },
    }
}

/// Validates a typed timestamp for `field` against the other boundary in `range`.
pub fn validate_time_edit(
    field: TimeField,
    value: &str,
    range: TimeRangeEdit,
) -> Result<TimeRangeEdit, TimeEditError> {
    let parsed = parse_time(value).ok_or(TimeEditError::InvalidFormat)?;
    let start_seconds = if field == TimeField::Start {
        parsed
    } else {
        range.start_seconds
    };
    let end_seconds = if field == TimeField::End {
        parsed
    } else {
        range.end_seconds
    };

    if start_seconds < 0.0 || end_seconds < 0.0 {
        return Err(TimeEditError::Negative);
    }
    if start_seconds >= end_seconds {
        return Err(match field {
            TimeField::Start => TimeEditError::StartNotBeforeEnd,
            TimeField::End => TimeEditError::EndNotAfterStart,
        });
    }
    Ok(TimeRangeEdit {
        start_seconds,
        end_seconds,
    })
}
