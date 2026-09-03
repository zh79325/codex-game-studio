use serde::Deserialize;
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum WorkflowState {
    Draft,
    Clarifying,
    BriefReady,
    Reviewing,
    Merging,
    UserReview,
    Confirmed,
    Versioned,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkflowCommand {
    StartFocus,
    SubmitClarification,
    AcceptBrief,
    CompleteReviews,
    CompleteMerge,
    RecordConflictDecision,
    ConfirmArtBible,
    VersionArtBible,
    Cancel,
    Retry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmationKind {
    ToolApproval,
    DesignDecision,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum WorkflowError {
    #[error("command {command:?} is invalid in state {state:?}")]
    InvalidTransition {
        state: WorkflowState,
        command: WorkflowCommand,
    },
    #[error("stale input version: expected {expected}, received {actual}")]
    StaleInputVersion { expected: u64, actual: u64 },
}

impl WorkflowState {
    pub fn apply(self, command: WorkflowCommand) -> Result<Self, WorkflowError> {
        let next = match (self, command) {
            (Self::Draft, WorkflowCommand::StartFocus) => Self::Clarifying,
            (Self::Clarifying, WorkflowCommand::SubmitClarification) => Self::BriefReady,
            (Self::BriefReady, WorkflowCommand::AcceptBrief) => Self::Reviewing,
            (Self::Reviewing, WorkflowCommand::CompleteReviews) => Self::Merging,
            (Self::Merging, WorkflowCommand::CompleteMerge) => Self::UserReview,
            (Self::UserReview, WorkflowCommand::RecordConflictDecision) => Self::UserReview,
            (Self::UserReview, WorkflowCommand::ConfirmArtBible) => Self::Confirmed,
            (Self::Confirmed, WorkflowCommand::VersionArtBible) => Self::Versioned,
            (Self::Clarifying, WorkflowCommand::Retry)
            | (Self::Reviewing, WorkflowCommand::Retry)
            | (Self::Merging, WorkflowCommand::Retry) => self,
            (Self::Draft, WorkflowCommand::Cancel)
            | (Self::Clarifying, WorkflowCommand::Cancel)
            | (Self::BriefReady, WorkflowCommand::Cancel)
            | (Self::Reviewing, WorkflowCommand::Cancel)
            | (Self::Merging, WorkflowCommand::Cancel)
            | (Self::UserReview, WorkflowCommand::Cancel) => Self::Cancelled,
            _ => {
                return Err(WorkflowError::InvalidTransition {
                    state: self,
                    command,
                });
            }
        };
        Ok(next)
    }
}

pub fn validate_input_version(expected: u64, actual: u64) -> Result<(), WorkflowError> {
    if expected == actual {
        Ok(())
    } else {
        Err(WorkflowError::StaleInputVersion { expected, actual })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn follows_the_focus_happy_path() {
        let mut state = WorkflowState::Draft;
        for command in [
            WorkflowCommand::StartFocus,
            WorkflowCommand::SubmitClarification,
            WorkflowCommand::AcceptBrief,
            WorkflowCommand::CompleteReviews,
            WorkflowCommand::CompleteMerge,
            WorkflowCommand::ConfirmArtBible,
            WorkflowCommand::VersionArtBible,
        ] {
            state = state.apply(command).expect("valid transition");
        }
        assert_eq!(state, WorkflowState::Versioned);
    }

    #[test]
    fn rejects_illegal_and_stale_updates() {
        assert!(matches!(
            WorkflowState::Draft.apply(WorkflowCommand::ConfirmArtBible),
            Err(WorkflowError::InvalidTransition { .. })
        ));
        assert_eq!(
            validate_input_version(2, 3),
            Err(WorkflowError::StaleInputVersion {
                expected: 2,
                actual: 3,
            })
        );
    }

    #[test]
    fn cannot_confirm_twice() {
        assert!(matches!(
            WorkflowState::Confirmed.apply(WorkflowCommand::ConfirmArtBible),
            Err(WorkflowError::InvalidTransition { .. })
        ));
    }
}
