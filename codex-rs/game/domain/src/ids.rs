use serde::Deserialize;
use serde::Serialize;

macro_rules! define_id {
    ($name:ident) => {
        #[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl From<String> for $name {
            fn from(value: String) -> Self {
                Self(value)
            }
        }
    };
}

define_id!(ProjectId);
define_id!(ConversationId);
define_id!(InteractionId);
define_id!(FocusWorkflowId);
define_id!(TaskId);
define_id!(TaskAttemptId);
define_id!(ArtifactId);
define_id!(ArtBibleVersionId);
define_id!(ConversationCodexThreadId);
