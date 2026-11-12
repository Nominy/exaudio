//! Structural edits: split, merge, delete, insert rows, annotation type, cut-preview commit and
//! loop, decoded-audio trimming, auto-insert and silence auto-segmentation.

use super::{EditError, EditResult, EditorDoc, Effect, Outcome};
use crate::edit::silence::{
    AudioTrimPlan, AutoSegmentMergePlan, audio_trim_plan_for_segment, auto_insert_plans_for_caret,
    auto_segment_merge_plans, auto_segment_split_plans, cut_preview_scroll_target,
    merge_auto_insert_adjacent_segments, nearest_same_lane_segment, segment_is_silent,
    visible_transcription_segments,
};
use crate::edit::structure::{
    CUT_PREVIEW_MIN_DURATION_SECONDS, SplitError, merge_neighbor_segment_id, split_ratio,
    split_text_by_word_ratio,
};
use crate::edit::text::join_segment_text;
use crate::edit::text_placement::{baseline_groups, place_group_text};
use crate::i18n::{Text, localized};
use crate::localized_format;
use crate::timing::{SplitSource, TimeRange, smart_split_text};
use exaudio_core::SegmentUpdate;

/// Smart splits must leave this much of the row on either side.
pub const SMART_SPLIT_EDGE_GUARD_SECONDS: f64 = 0.05;

/// Cap on merge attempts in one auto-segmentation run.
const AUTO_SEGMENT_MERGE_ATTEMPTS: usize = 200;

const SPLIT: Text = Text::new("Split", "Разрез");
const SMART_SPLIT: Text = Text::new("Smart split", "Умный разрез");
const MERGE: Text = Text::new("Merge", "Объединение");
const DELETE: Text = Text::new("Delete", "Удаление");
const CUT_PREVIEW: Text = Text::new("Cut preview", "Разрез по выделению");
const AUDIO_TRIM: Text = Text::new("Audio trim", "Обрезка по речи");
const TRIM_VISIBLE: Text = Text::new("Trim visible segments", "Обрезка видимых сегментов");
const AUTO_INSERT: Text = Text::new("Auto insert segment", "Автодобавление сегмента");
const AUTO_SEGMENTATION: Text = Text::new("Auto segmentation", "Автосегментация");

/// What the silent-row phase of auto-segmentation did.
#[derive(Debug, Default)]
struct SilentCleanup {
    removed: usize,
    /// Silent rows left in place (text with no same-lane row to take it, or a refused edit).
    kept: usize,
    /// Rows that took a removed row's text.
    stitched: Vec<String>,
}

impl EditorDoc {
    /// Ctrl+Click: splits a row at `at_seconds`, allocating its (draft) words by time ratio;
    /// selects the new right-hand row. Rejects points outside the row or within 100ms of its
    /// end.
    pub fn split_segment_at(&mut self, segment_id: &str, at_seconds: f64) -> EditResult {
        self.guard(SPLIT.get())?;
        let segment = self.segment(segment_id)?;
        let ratio = split_ratio(&segment, at_seconds)?;
        let text = self.drafts.text_or(&segment).to_string();
        let (left, right) = split_text_by_word_ratio(&text, ratio);
        self.split_with_texts(SPLIT.get(), segment_id, at_seconds, left, right, "")
    }

    /// `S` outside a cut selection: splits the selected row at the playhead (rejected when the
    /// playhead is outside it, as [`Self::split_segment_at`] does).
    pub fn split_selected_at_playhead(&mut self) -> EditResult {
        let segment_id = self.selected_segment_id(SPLIT.get())?;
        let at_seconds = self.session.playhead_seconds;
        self.split_segment_at(&segment_id, at_seconds)
    }

    /// Shift+Ctrl+Click: splits a row and allocates its (draft) text with the lane's word
    /// timings — the exact completed-word boundary, else the interpolated timing offset, else
    /// the word ratio.
    pub fn smart_split_segment_at(&mut self, segment_id: &str, at_seconds: f64) -> EditResult {
        self.guard(SMART_SPLIT.get())?;
        let segment = self.segment(segment_id)?;
        split_ratio(&segment, at_seconds)?;
        if at_seconds <= segment.start_seconds + SMART_SPLIT_EDGE_GUARD_SECONDS {
            return Err(SplitError::OutsideSegment.into());
        }
        let text = self.drafts.text_or(&segment).to_string();
        let parts = smart_split_text(
            &text,
            self.word_timings.lane(&segment.lane_id),
            TimeRange::new(segment.start_seconds, segment.end_seconds),
            at_seconds,
        )
        .ok_or(EditError::Split(SplitError::OutsideSegment))?;
        let note = split_source_note(parts.source);
        self.split_with_texts(
            SMART_SPLIT.get(),
            segment_id,
            at_seconds,
            parts.first,
            parts.second,
            note,
        )
    }

    fn split_with_texts(
        &mut self,
        action: &'static str,
        segment_id: &str,
        at_seconds: f64,
        left: String,
        right: String,
        note: &str,
    ) -> EditResult {
        let created = self.apply(action, |workflow| {
            workflow.split_segment_with_text(segment_id, at_seconds, left, right)
        })?;
        self.drafts.refresh(self.workflow.project(), segment_id);
        self.mark_dirty();
        self.reconcile(true);
        let outcome = self.select_segment_without_seek(&created);
        let status = localized_format!(
            "Split {} at {at_seconds:.3}s into {}{note}.",
            "Разрезан {} на {at_seconds:.3} с, создан {}{note}.",
            self.segment_label(segment_id),
            self.segment_label(&created)
        );
        Ok(outcome.with_status(status).with(Effect::DraftChanged {
            segment_id: segment_id.to_string(),
        }))
    }

    /// Merges a row into the previous same-speaker row on its lane (pending text drafts of both
    /// rows are applied first).
    pub fn merge_above(&mut self, segment_id: &str) -> EditResult {
        self.merge(segment_id, true)
    }

    /// Merges the next same-speaker row on the lane into this row.
    pub fn merge_below(&mut self, segment_id: &str) -> EditResult {
        self.merge(segment_id, false)
    }

    pub fn merge_selected_above(&mut self) -> EditResult {
        let segment_id = self.selected_segment_id(MERGE.get())?;
        self.merge(&segment_id, true)
    }

    pub fn merge_selected_below(&mut self) -> EditResult {
        let segment_id = self.selected_segment_id(MERGE.get())?;
        self.merge(&segment_id, false)
    }

    fn merge(&mut self, segment_id: &str, previous: bool) -> EditResult {
        self.session.open_actions_segment_id = None;
        self.guard(MERGE.get())?;
        let pending: Vec<SegmentUpdate> =
            merge_neighbor_segment_id(self.workflow.project(), segment_id, previous)
                .map(|neighbor| self.pending_text_updates(&[segment_id, &neighbor]))
                .unwrap_or_default();
        let label = self.segment_label(segment_id);
        self.apply(MERGE.get(), |workflow| {
            if !pending.is_empty() {
                workflow.apply_segment_edits(pending)?;
            }
            if previous {
                workflow.merge_with_previous(segment_id)
            } else {
                workflow.merge_with_next(segment_id)
            }
        })?;
        self.drafts.refresh(self.workflow.project(), segment_id);
        self.mark_dirty();
        self.reconcile(false);
        Ok(Outcome::status(if previous {
            localized_format!(
                "Merged {label} into the previous segment on its track.",
                "Объединён {label} с предыдущим сегментом дорожки."
            )
        } else {
            localized_format!(
                "Merged {label} with the next segment on its track.",
                "Объединён {label} со следующим сегментом дорожки."
            )
        }))
    }

    fn pending_text_updates(&self, segment_ids: &[&str]) -> Vec<SegmentUpdate> {
        let project = self.workflow.project();
        segment_ids
            .iter()
            .filter_map(|id| {
                let segment = project.segment(id)?;
                let draft = self.drafts.get(id)?;
                (draft.text() != segment.text).then(|| SegmentUpdate {
                    segment_id: segment.id.clone(),
                    start_seconds: segment.start_seconds,
                    end_seconds: segment.end_seconds,
                    text: draft.text().to_string(),
                })
            })
            .collect()
    }

    pub fn delete_segment(&mut self, segment_id: &str) -> EditResult {
        self.session.open_actions_segment_id = None;
        self.guard(DELETE.get())?;
        let label = self.segment_label(segment_id);
        self.apply(DELETE.get(), |workflow| workflow.delete_segment(segment_id))?;
        self.drafts.remove(segment_id);
        self.mark_dirty();
        self.reconcile(false);
        Ok(Outcome::status(localized_format!(
            "Deleted {label}.",
            "Удалён {label}."
        )))
    }

    /// D outside text inputs.
    pub fn delete_selected(&mut self) -> EditResult {
        let segment_id = self.selected_segment_id(DELETE.get())?;
        self.delete_segment(&segment_id)
    }

    /// Inserts an empty row in the gap before `segment_id` on its lane and selects it.
    pub fn create_segment_before(&mut self, segment_id: &str) -> EditResult {
        self.create_adjacent(segment_id, true)
    }

    /// Inserts an empty row in the gap after `segment_id` on its lane and selects it.
    pub fn create_segment_after(&mut self, segment_id: &str) -> EditResult {
        self.create_adjacent(segment_id, false)
    }

    fn create_adjacent(&mut self, segment_id: &str, before: bool) -> EditResult {
        self.session.open_actions_segment_id = None;
        let created = self.apply(
            localized("Create", "Создание сегмента"),
            |workflow| {
                if before {
                    workflow.create_segment_before(segment_id)
                } else {
                    workflow.create_segment_after(segment_id)
                }
            },
        )?;
        self.mark_dirty();
        self.reconcile(true);
        let outcome = self.select_segment_without_seek(&created);
        let (new, anchor) = (self.segment_label(&created), self.segment_label(segment_id));
        let status = if before {
            localized_format!(
                "Created {new} before {anchor}.",
                "Создан {new}, после него — {anchor}."
            )
        } else {
            localized_format!(
                "Created {new} after {anchor}.",
                "Создан {new}, перед ним — {anchor}."
            )
        };
        Ok(outcome.with_status(status))
    }

    // ---- cut preview ------------------------------------------------------------------

    /// Shared by cut validation and the selection's eligibility indicator.
    pub fn minimum_cut_duration_seconds(&self) -> f64 {
        (self.environment().policy.minimum_cut_milliseconds as f64 / 1_000.0)
            .max(CUT_PREVIEW_MIN_DURATION_SECONDS)
    }

    /// S / Shift+S: cuts the overlay range out of the lane the cut preview was drawn on; with
    /// `smart_split` a row covering the whole range gets its text split at the range midpoint
    /// (word timings first, like Ctrl+Shift+Click); clears the overlay and selects the row
    /// after the cut.
    pub fn commit_cut_preview(&mut self, smart_split: bool) -> EditResult {
        self.guard(CUT_PREVIEW.get())?;
        let (lane_id, range) = self
            .session
            .overlay
            .as_ref()
            .map(|overlay| (overlay.lane_id.clone(), overlay.range))
            .ok_or_else(|| {
                EditError::Unavailable(
                    localized(
                        "Cut preview unavailable: no active overlay range.",
                        "Разрез недоступен: нет выделения для разреза.",
                    )
                    .into(),
                )
            })?;
        let minimum = self.minimum_cut_duration_seconds();
        if range.duration_seconds() < minimum {
            return Err(EditError::Unavailable(localized_format!(
                "Cut needs a selection of at least {minimum} seconds; shorter selections can still loop.",
                "Для разреза нужно выделение не короче {minimum} с; более короткое можно только повторять."
            )));
        }
        let project = self.workflow.project();
        let affected: Vec<String> = project
            .segments
            .iter()
            .filter(|segment| {
                segment.lane_id == lane_id
                    && segment.end_seconds > range.start_seconds
                    && segment.start_seconds < range.end_seconds
            })
            .map(|segment| segment.id.clone())
            .collect();
        if affected.is_empty() {
            return Err(EditError::Unavailable(
                localized(
                    "Cut preview found no overlapping segment on the target lane.",
                    "На дорожке нет сегментов, пересекающихся с выделением для разреза.",
                )
                .into(),
            ));
        }
        let split = smart_split
            .then(|| {
                project.segments.iter().find(|segment| {
                    segment.lane_id == lane_id
                        && segment.start_seconds < range.start_seconds
                        && segment.end_seconds > range.end_seconds
                })
            })
            .flatten()
            .and_then(|segment| {
                smart_split_text(
                    self.drafts.text_or(segment),
                    self.word_timings.lane(&lane_id),
                    TimeRange::new(segment.start_seconds, segment.end_seconds),
                    (range.start_seconds + range.end_seconds) / 2.0,
                )
                .map(|parts| (segment.id.clone(), parts))
            });
        let note = split
            .as_ref()
            .map_or("", |(_, parts)| split_source_note(parts.source));
        let split_source = split.as_ref().map(|(id, _)| id.clone());
        let split_texts = split.map(|(_, parts)| (parts.first, parts.second));
        self.apply(CUT_PREVIEW.get(), |workflow| {
            workflow.cut_lane_range(
                &lane_id,
                range.start_seconds,
                range.end_seconds,
                split_texts,
            )
        })?;
        // The cut moved the rows' bounds (and split the source's text): their drafts follow,
        // keeping text typed into trimmed rows, so nothing is left pending and Undo works.
        let mut effects = Vec::new();
        for segment_id in &affected {
            match self.workflow.project().segment(segment_id).cloned() {
                Some(segment) if split_source.as_deref() != Some(segment_id.as_str()) => {
                    if let Some(draft) = self.drafts.get_mut(segment_id) {
                        draft.replace_times(segment.start_seconds, segment.end_seconds);
                    }
                }
                _ => self.drafts.refresh(self.workflow.project(), segment_id),
            }
            effects.push(Effect::DraftChanged {
                segment_id: segment_id.clone(),
            });
        }
        self.mark_dirty();
        self.reconcile(true);
        self.session.overlay = None;
        let status = localized_format!(
            "Cut {:.3}s-{:.3}s on {}{}{note}.",
            "Вырезано {:.3}–{:.3} с ({}){}{note}.",
            range.start_seconds,
            range.end_seconds,
            self.lane_label(&lane_id),
            if smart_split {
                localized(" with smart split", " с умным разрезом")
            } else {
                ""
            }
        );
        let outcome = match cut_preview_scroll_target(self.workflow.project(), &lane_id, range) {
            Some(target) => self
                .select_segment_without_seek(&target)
                .with_status(status),
            None => Outcome::status(status),
        };
        Ok(effects.into_iter().fold(outcome, Outcome::with))
    }

    /// L with a cut selection: toggles looping playback over it (no project edit). Turning the
    /// loop on moves a playhead outside the selection to its start.
    pub fn loop_cut_preview(&mut self) -> EditResult {
        let overlay = self.session.overlay.as_mut().ok_or_else(|| {
            EditError::Unavailable(
                localized(
                    "Selection loop unavailable: no cut selection.",
                    "Повтор недоступен: нет выделения для разреза.",
                )
                .into(),
            )
        })?;
        overlay.looping = !overlay.looping;
        if !overlay.looping {
            return Ok(Outcome::status(localized(
                "Stopped looping the selection.",
                "Повтор выделения остановлен.",
            )));
        }
        let range = overlay.range;
        let status = localized_format!(
            "Looping selection {:.3}s-{:.3}s (L stops, Escape clears).",
            "Повтор выделения {:.3}–{:.3} с (L — стоп, Escape — снять выделение).",
            range.start_seconds,
            range.end_seconds
        );
        let playhead = self.session.playhead_seconds;
        let outcome = if (range.start_seconds..range.end_seconds).contains(&playhead) {
            Outcome::none()
        } else {
            self.seek(range.start_seconds)
        };
        Ok(outcome.with_status(status))
    }

    // ---- decoded-audio timing ---------------------------------------------------------

    /// Alt+R: trims the selected row to nearby speech in its lane's decoded audio.
    pub fn trim_current_segment_to_audio(&mut self) -> EditResult {
        self.guard(AUDIO_TRIM.get())?;
        let segment_id = self.selected_segment_id(AUDIO_TRIM.get())?;
        let segment = self.segment(&segment_id)?;
        let label = self.segment_label(&segment_id);
        let plan =
            match audio_trim_plan_for_segment(self.workflow.project(), &self.session, &segment) {
                Some(Ok(plan)) => plan,
                Some(Err(reason)) => {
                    return Err(EditError::Unavailable(localized_format!(
                        "Audio trim skipped {label}: {reason}.",
                        "Обрезка по речи пропустила {label}: {reason}."
                    )));
                }
                None => {
                    return Ok(Outcome::status(localized_format!(
                        "Audio trim made no timestamp changes for {label}.",
                        "Обрезка по речи: границы не изменились ({label})."
                    )));
                }
            };
        let action = localized("Trim current segment", "Обрезка сегмента");
        self.apply_trim_plans(vec![plan], action)?;
        Ok(Outcome::status(localized_format!(
            "Trimmed {label} to nearby decoded audio.",
            "Обрезан по речи {label}."
        )))
    }

    /// Alt+Shift+R: trims every visible transcription row with decoded audio.
    pub fn trim_visible_segments_to_audio(&mut self) -> EditResult {
        self.guard(AUDIO_TRIM.get())?;
        let plans = self.visible_trim_plans();
        if plans.is_empty() {
            return Err(EditError::Unavailable(
                localized(
                    "Audio trim found no visible segments with decoded speech.",
                    "Обрезка по речи: нет видимых сегментов с речью в загруженном аудио.",
                )
                .into(),
            ));
        }
        let count = plans.len();
        self.apply_trim_plans(plans, TRIM_VISIBLE.get())?;
        Ok(Outcome::status(localized_format!(
            "Trimmed {count} visible segments to nearby decoded audio.",
            "Обрезано видимых сегментов по речи: {count}."
        )))
    }

    /// Plans rows in time order against the bounds already planned for earlier rows (one
    /// row is trimmed at a time): two same-lane neighbours never both grow into the gap
    /// between them.
    fn visible_trim_plans(&self) -> Vec<AudioTrimPlan> {
        let mut working = self.workflow.project().clone();
        let ids: Vec<String> = visible_transcription_segments(
            &working,
            &self.session.track_filter,
            &self.session.collapsed_lane_ids,
            self.environment(),
        )
        .into_iter()
        .map(|segment| segment.id.clone())
        .collect();
        let mut plans = Vec::new();
        for id in ids {
            let Some(segment) = working.segment(&id).cloned() else {
                continue;
            };
            let Some(Ok(plan)) = audio_trim_plan_for_segment(&working, &self.session, &segment)
            else {
                continue;
            };
            // Growing to the next silence and trimming back can land on the same bounds.
            if (plan.start_seconds - segment.start_seconds).abs() < 1e-9
                && (plan.end_seconds - segment.end_seconds).abs() < 1e-9
            {
                continue;
            }
            if let Some(planned) = working.segments.iter_mut().find(|row| row.id == id) {
                planned.start_seconds = plan.start_seconds;
                planned.end_seconds = plan.end_seconds;
            }
            plans.push(plan);
        }
        plans
    }

    fn apply_trim_plans(
        &mut self,
        plans: Vec<AudioTrimPlan>,
        action: &'static str,
    ) -> Result<(), EditError> {
        let ids: Vec<String> = plans.iter().map(|plan| plan.segment_id.clone()).collect();
        let updates: Vec<SegmentUpdate> = plans.into_iter().map(AudioTrimPlan::update).collect();
        self.apply(action, |workflow| workflow.apply_segment_edits(updates))?;
        for id in &ids {
            self.drafts.refresh(self.workflow.project(), id);
        }
        self.mark_dirty();
        self.reconcile(true);
        Ok(())
    }

    /// Alt+C: creates an empty row around the nearest uncovered speech within 1s of the
    /// playhead on each visible lane, merging with same-speaker rows closer than 1s.
    pub fn auto_insert_segment_at_playhead(&mut self) -> EditResult {
        self.guard(AUTO_INSERT.get())?;
        let plans = auto_insert_plans_for_caret(self.workflow.project(), &self.session);
        if plans.is_empty() {
            return Err(EditError::Unavailable(
                localized(
                    "Auto-insert found no uncovered speech near caret.",
                    "Рядом с курсором нет речи без сегмента.",
                )
                .into(),
            ));
        }
        let mut created = Vec::new();
        let mut merges = 0;
        self.apply(AUTO_INSERT.get(), |workflow| {
            for plan in &plans {
                let id = workflow.create_segment_in_lane(
                    &plan.lane_id,
                    plan.start_seconds,
                    plan.end_seconds,
                )?;
                merges += merge_auto_insert_adjacent_segments(workflow, &id)?;
                created.push(id);
            }
            Ok(())
        })?;
        self.mark_dirty();
        self.reconcile(true);
        for id in &created {
            self.drafts.refresh(self.workflow.project(), id);
        }
        let status = localized_format!(
            "Auto-inserted {} segment(s) around nearest uncovered speech, merged {merges} adjacent row(s).",
            "Добавлено сегментов вокруг ближайшей речи без сегмента: {}; объединено соседних: {merges}.",
            created.len()
        );
        Ok(
            match created
                .last()
                .filter(|id| self.workflow.project().has_segment(id))
            {
                Some(last) => {
                    let last = last.clone();
                    self.select_segment_without_seek(&last).with_status(status)
                }
                None => Outcome::status(status),
            },
        )
    }

    /// Alt+Shift+S, in this phase order: trims the visible
    /// rows to audio; merges same-lane rows at most 1s apart; captures the rows' text; splits
    /// rows in the middle of decoded silences of 1s or more; trims again; removes rows with no
    /// audio, stitching their text onto the nearest same-lane row; then places each captured
    /// text group back over its rows — through word timings when the lane has them, else by
    /// whole sentences and duration. Text is conserved exactly: a group whose placement would
    /// lose words keeps the split-time allocation and is reported.
    pub fn auto_segment_visible_silences(&mut self) -> EditResult {
        self.guard(AUTO_SEGMENTATION.get())?;
        if self.visible_segments().is_empty() {
            return Err(EditError::Unavailable(
                localized(
                    "Auto-segmentation found no visible segments.",
                    "Автосегментация: нет видимых сегментов.",
                )
                .into(),
            ));
        }
        let pre_trim = self.visible_trim_plans();
        let mut trimmed = pre_trim.len();
        if !pre_trim.is_empty() {
            self.apply_trim_plans(pre_trim, TRIM_VISIBLE.get())?;
        }
        let merged = self.merge_close_visible_rows()?;

        let baseline = baseline_groups(&self.visible_segments());
        let mut plans = auto_segment_split_plans(self.workflow.project(), &self.session);
        // Split later points first so earlier split ids stay valid.
        plans.sort_by(|left, right| {
            left.segment_id
                .cmp(&right.segment_id)
                .then_with(|| right.split_seconds.total_cmp(&left.split_seconds))
        });
        let count = plans.len();
        let mut created = Vec::new();
        if !plans.is_empty() {
            self.apply(AUTO_SEGMENTATION.get(), |workflow| {
                for plan in &plans {
                    created.push(workflow.split_segment_with_project_text_ratio(
                        &plan.segment_id,
                        plan.split_seconds,
                    )?);
                }
                Ok(())
            })?;
            self.mark_dirty();
            self.reconcile(true);
        }
        let post_trim = self.visible_trim_plans();
        trimmed += post_trim.len();
        if !post_trim.is_empty() {
            self.apply_trim_plans(post_trim, TRIM_VISIBLE.get())?;
        }
        let cleanup = self.remove_silent_visible_rows()?;

        let placement = place_group_text(&baseline, &self.visible_segments(), &self.word_timings);
        let placed: Vec<String> = placement
            .updates
            .iter()
            .map(|update| update.segment_id.clone())
            .collect();
        if !placement.updates.is_empty() {
            let updates = placement.updates.clone();
            let action = localized("Place segment text", "Размещение текста");
            self.apply(action, |workflow| workflow.apply_segment_edits(updates))?;
            self.mark_dirty();
        }
        if trimmed == 0
            && merged.is_empty()
            && count == 0
            && cleanup.removed == 0
            && placed.is_empty()
        {
            return Err(EditError::Unavailable(
                localized(
                    "Auto-segmentation found no close same-speaker segments, silence runs over \
                     1000ms or silent segments.",
                    "Автосегментация: нет близких сегментов одного спикера, пауз длиннее \
                     1000 мс и сегментов без звука.",
                )
                .into(),
            ));
        }
        let split_sources = plans.iter().map(|plan| &plan.segment_id);
        for id in merged
            .iter()
            .chain(split_sources)
            .chain(&created)
            .chain(&cleanup.stitched)
            .chain(&placed)
        {
            self.drafts.refresh(self.workflow.project(), id);
        }

        let mut status = localized_format!(
            "Auto-segmented {count} silence run(s) over 1000ms and trimmed all",
            "Автосегментация: разрезов по паузам от 1000 мс — {count}, все сегменты обрезаны"
        );
        if !merged.is_empty() {
            status.push_str(&localized_format!(
                "; merged {} close same-speaker segment pair(s) first",
                "; сначала объединено близких пар сегментов одного спикера: {}",
                merged.len()
            ));
        }
        if cleanup.removed > 0 {
            status.push_str(&localized_format!(
                "; removed {} silent segment(s)",
                "; удалено сегментов без звука: {}",
                cleanup.removed
            ));
        }
        if cleanup.kept > 0 {
            status.push_str(&localized_format!(
                "; kept {} silent segment(s) with text and no same-speaker neighbour",
                "; оставлено сегментов без звука с текстом и без соседа того же спикера: {}",
                cleanup.kept
            ));
        }
        match (placement.timed_groups, placement.sentence_groups) {
            (0, 0) => {}
            (timed, 0) => status.push_str(&localized_format!(
                "; placed text by word timestamps ({timed})",
                "; текст размещён по таймингам слов ({timed})"
            )),
            (0, sentences) => status.push_str(&localized_format!(
                "; placed text by sentences ({sentences})",
                "; текст размещён по предложениям ({sentences})"
            )),
            (timed, sentences) => status.push_str(&localized_format!(
                "; placed text by word timestamps ({timed}) and sentences ({sentences})",
                "; текст размещён по таймингам слов ({timed}) и по предложениям ({sentences})"
            )),
        }
        if let Some((lane, error)) = placement.failed.first() {
            status.push_str(&localized_format!(
                "; {} group(s) kept split text ({}: {error})",
                "; групп, где текст остался как после разреза: {} ({}: {error})",
                placement.failed.len(),
                self.lane_label(lane)
            ));
        }
        status.push('.');
        Ok(match created.last() {
            Some(last) if self.workflow.project().has_segment(last) => {
                let last = last.clone();
                self.select_segment_without_seek(&last).with_status(status)
            }
            _ => Outcome::status(status),
        })
    }

    /// Merges visible same-lane rows at most 1s apart one
    /// pair at a time (merges cascade), skipping pairs the project refuses. Returns the ids of
    /// the rows that absorbed a neighbour.
    fn merge_close_visible_rows(&mut self) -> Result<Vec<String>, EditError> {
        let track_filter = self.session.track_filter.clone();
        let collapsed = self.session.collapsed_lane_ids.clone();
        if auto_segment_merge_plans(&self.visible_segments()).is_empty() {
            return Ok(Vec::new());
        }
        let mut merged = Vec::new();
        let environment = self.environment().clone();
        self.apply(AUTO_SEGMENTATION.get(), |workflow| {
            let mut skipped: Vec<AutoSegmentMergePlan> = Vec::new();
            for _ in 0..AUTO_SEGMENT_MERGE_ATTEMPTS {
                let rows = visible_transcription_segments(
                    workflow.project(),
                    &track_filter,
                    &collapsed,
                    &environment,
                );
                let Some(plan) = auto_segment_merge_plans(&rows)
                    .into_iter()
                    .find(|plan| !skipped.contains(plan))
                else {
                    break;
                };
                if workflow
                    .merge_segments(&plan.left_id, &plan.right_id)
                    .is_ok()
                {
                    merged.push(plan.left_id);
                    skipped.clear();
                } else {
                    skipped.push(plan);
                }
            }
            Ok(())
        })?;
        if !merged.is_empty() {
            self.mark_dirty();
            self.reconcile(true);
        }
        Ok(merged)
    }

    /// Removes visible rows with no decoded audio above
    /// the trim threshold, latest first. A silent row's text is first stitched onto the nearest
    /// same-lane row (in time order); a silent row with text and no such row is kept.
    fn remove_silent_visible_rows(&mut self) -> Result<SilentCleanup, EditError> {
        let silent: Vec<String> = self
            .visible_segments()
            .into_iter()
            .filter(|segment| segment_is_silent(&self.session, segment))
            .map(|segment| segment.id.clone())
            .collect();
        let mut cleanup = SilentCleanup::default();
        if silent.is_empty() {
            return Ok(cleanup);
        }
        let track_filter = self.session.track_filter.clone();
        let collapsed = self.session.collapsed_lane_ids.clone();
        let environment = self.environment().clone();
        let action = localized("Remove silent segments", "Удаление сегментов без звука");
        self.apply(action, |workflow| {
            for id in silent.iter().rev() {
                let rows = visible_transcription_segments(
                    workflow.project(),
                    &track_filter,
                    &collapsed,
                    &environment,
                );
                let Some(row) = rows.iter().copied().find(|row| row.id == *id) else {
                    continue;
                };
                let silent_text = row.text.trim().to_string();
                let stitch = match nearest_same_lane_segment(&rows, row) {
                    None if !silent_text.is_empty() => {
                        cleanup.kept += 1;
                        continue;
                    }
                    Some(nearest) if !silent_text.is_empty() => {
                        let text = if row.start_seconds < nearest.start_seconds {
                            join_segment_text(&silent_text, &nearest.text)
                        } else {
                            join_segment_text(&nearest.text, &silent_text)
                        };
                        let update = |text: String| SegmentUpdate {
                            segment_id: nearest.id.clone(),
                            start_seconds: nearest.start_seconds,
                            end_seconds: nearest.end_seconds,
                            text,
                        };
                        (text != nearest.text).then(|| (update(text), update(nearest.text.clone())))
                    }
                    _ => None,
                };
                if let Some((stitched, _)) = &stitch
                    && workflow
                        .apply_segment_edits(vec![stitched.clone()])
                        .is_err()
                {
                    cleanup.kept += 1;
                    continue;
                }
                if workflow.delete_segment(id).is_ok() {
                    cleanup.removed += 1;
                    if let Some((stitched, _)) = stitch {
                        cleanup.stitched.push(stitched.segment_id);
                    }
                } else {
                    // The neighbour must not keep a copy of text still in the silent row.
                    cleanup.kept += 1;
                    if let Some((_, restore)) = stitch {
                        workflow.apply_segment_edits(vec![restore])?;
                    }
                }
            }
            Ok(())
        })?;
        if cleanup.removed > 0 {
            self.mark_dirty();
            self.reconcile(true);
        }
        Ok(cleanup)
    }

    /// Visible transcription rows sorted by start.
    fn visible_segments(&self) -> Vec<&exaudio_core::Segment> {
        visible_transcription_segments(
            self.workflow.project(),
            &self.session.track_filter,
            &self.session.collapsed_lane_ids,
            self.environment(),
        )
    }
}

/// Status suffix naming how a smart split placed its text.
fn split_source_note(source: SplitSource) -> &'static str {
    match source {
        SplitSource::CompletedWord => localized(" at a word timestamp", " по таймингу слова"),
        SplitSource::Interpolated => {
            localized(" between word timestamps", " между таймингами слов")
        }
        SplitSource::WordRatio | SplitSource::Empty => "",
    }
}
