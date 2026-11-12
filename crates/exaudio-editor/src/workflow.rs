mod history;
mod io;
mod segments;

use std::{cell::RefCell, cmp::Ordering, sync::Arc};

use exaudio_core::{CommandHistory, NativeSegmentMutationPolicy, Project, SegmentMutationPolicy};

#[derive(Clone, Debug)]
pub struct DocumentWorkflow {
    project: Project,
    history: CommandHistory,
    pending_boundary_edit_segment_id: RefCell<Option<String>>,
    mutation_policy: Arc<dyn SegmentMutationPolicy>,
}

impl DocumentWorkflow {
    pub fn from_project(project: Project) -> Self {
        Self::from_project_with_mutation_policy(project, Arc::new(NativeSegmentMutationPolicy))
    }

    pub fn from_project_with_mutation_policy(
        project: Project,
        mutation_policy: Arc<dyn SegmentMutationPolicy>,
    ) -> Self {
        Self {
            project,
            history: CommandHistory::default(),
            pending_boundary_edit_segment_id: RefCell::new(None),
            mutation_policy,
        }
    }

    pub fn mutation_policy(&self) -> Arc<dyn SegmentMutationPolicy> {
        Arc::clone(&self.mutation_policy)
    }

    pub fn set_mutation_policy(&mut self, mutation_policy: Arc<dyn SegmentMutationPolicy>) {
        self.clear_pending_boundary_edit();
        self.mutation_policy = mutation_policy;
    }
}

fn compare_seconds(left: f64, right: f64) -> Ordering {
    left.partial_cmp(&right).unwrap_or(Ordering::Equal)
}
