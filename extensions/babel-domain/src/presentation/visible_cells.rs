use exaudio_core::Project;
use exaudio_editor::transcript_model::TranscriptRowVisibleCells;

const META_HIDE_TIMESTAMPS: &str = "studio.babel.table.hideTimestamps";

/// Cells every row of the task shows: tasks that hide timestamps keep the text and its checks.
pub(super) fn visible_cells(project: &Project) -> TranscriptRowVisibleCells {
    let hide_timestamps = project
        .metadata
        .get(META_HIDE_TIMESTAMPS)
        .is_some_and(|value| value == "true");
    TranscriptRowVisibleCells {
        id: !hide_timestamps,
        track: !hide_timestamps,
        start: !hide_timestamps,
        end: !hide_timestamps,
        text: true,
        linter: true,
        actions: !hide_timestamps,
    }
}
