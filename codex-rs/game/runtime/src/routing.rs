use serde::Deserialize;
use serde::Serialize;
use std::collections::HashMap;
use std::collections::HashSet;
use std::sync::Mutex;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    TextReasoning,
    TextStructuredOutput,
    VisionAnalysis,
    ImageTextToImage,
    ImageImageToImage,
    ImageReferenceConsistency,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteCandidate {
    pub account_id: String,
    pub provider: String,
    pub model: String,
    pub capabilities: Vec<Capability>,
    pub available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RouteDecision {
    pub account_id: String,
    pub provider: String,
    pub model: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuotaRequirement {
    pub metric: String,
    pub amount: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum RouteEvent {
    Selected {
        scope: String,
        decision: RouteDecision,
    },
    Switched {
        scope: String,
        decision: RouteDecision,
    },
    UsageUpdated {
        account_id: String,
        metric: String,
        amount: u64,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RouteError {
    #[error("no provider supports the requested capability")]
    CapabilityUnavailable,
    #[error("route state is unavailable")]
    StateUnavailable,
    #[error("quota is insufficient for account {account_id} metric {metric}")]
    QuotaExceeded { account_id: String, metric: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteFailureKind {
    Retryable,
    ContextTooLarge,
    CapabilityUnavailable,
    InvalidRequest,
    Fatal,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteOutcome {
    Succeeded,
    Failed(RouteFailureKind),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteHealth {
    pub capability: Capability,
    pub available_candidates: usize,
}

#[derive(Debug)]
pub struct RouteSelector {
    candidates: Vec<RouteCandidate>,
    bindings: Mutex<HashMap<String, RouteDecision>>,
    unavailable_accounts: Mutex<HashSet<String>>,
    remaining_quota: Mutex<HashMap<(String, String), u64>>,
    usage_keys: Mutex<HashSet<(String, String)>>,
}

impl RouteSelector {
    pub fn new(candidates: Vec<RouteCandidate>) -> Self {
        Self {
            candidates,
            bindings: Mutex::new(HashMap::new()),
            unavailable_accounts: Mutex::new(HashSet::new()),
            remaining_quota: Mutex::new(HashMap::new()),
            usage_keys: Mutex::new(HashSet::new()),
        }
    }

    pub fn candidates(&self) -> &[RouteCandidate] {
        &self.candidates
    }

    pub fn current_binding(&self, scope: &str) -> Result<Option<RouteDecision>, RouteError> {
        self.bindings
            .lock()
            .map_err(|_| RouteError::StateUnavailable)
            .map(|bindings| bindings.get(scope).cloned())
    }

    pub fn restore_binding(
        &self,
        scope: String,
        decision: RouteDecision,
    ) -> Result<(), RouteError> {
        self.bindings
            .lock()
            .map_err(|_| RouteError::StateUnavailable)?
            .entry(scope)
            .or_insert(decision);
        Ok(())
    }

    pub fn set_quota(&self, account_id: &str, metric: &str, amount: u64) -> Result<(), RouteError> {
        self.remaining_quota
            .lock()
            .map_err(|_| RouteError::StateUnavailable)?
            .insert((account_id.to_string(), metric.to_string()), amount);
        Ok(())
    }

    pub fn select(
        &self,
        capability: Capability,
        binding_scope: &str,
    ) -> Result<(RouteDecision, RouteEvent), RouteError> {
        let mut bindings = self
            .bindings
            .lock()
            .map_err(|_| RouteError::StateUnavailable)?;
        let unavailable = self
            .unavailable_accounts
            .lock()
            .map_err(|_| RouteError::StateUnavailable)?;
        if let Some(bound) = bindings.get(binding_scope)
            && self.candidates.iter().any(|candidate| {
                candidate.available
                    && !unavailable.contains(&candidate.account_id)
                    && candidate.account_id == bound.account_id
                    && candidate.capabilities.contains(&capability)
            })
        {
            return Ok((
                bound.clone(),
                RouteEvent::Selected {
                    scope: binding_scope.to_string(),
                    decision: bound.clone(),
                },
            ));
        }
        let previous = bindings.get(binding_scope);
        let available = |candidate: &&RouteCandidate| {
            candidate.available
                && !unavailable.contains(&candidate.account_id)
                && candidate.capabilities.contains(&capability)
        };
        let candidate = previous
            .and_then(|bound| {
                self.candidates.iter().find(|candidate| {
                    available(candidate)
                        && candidate.provider == bound.provider
                        && candidate.model == bound.model
                })
            })
            .or_else(|| self.candidates.iter().find(available))
            .ok_or(RouteError::CapabilityUnavailable)?;
        let decision = RouteDecision {
            account_id: candidate.account_id.clone(),
            provider: candidate.provider.clone(),
            model: candidate.model.clone(),
        };
        let event = if bindings
            .insert(binding_scope.to_string(), decision.clone())
            .is_some()
        {
            RouteEvent::Switched {
                scope: binding_scope.to_string(),
                decision: decision.clone(),
            }
        } else {
            RouteEvent::Selected {
                scope: binding_scope.to_string(),
                decision: decision.clone(),
            }
        };
        Ok((decision, event))
    }

    pub fn event_type(event: &RouteEvent) -> &'static str {
        match event {
            RouteEvent::Selected { .. } => "route.selected",
            RouteEvent::Switched { .. } => "route.switched",
            RouteEvent::UsageUpdated { .. } => "usage.updated",
        }
    }

    pub fn report(
        &self,
        decision: &RouteDecision,
        outcome: RouteOutcome,
    ) -> Result<(), RouteError> {
        if matches!(
            outcome,
            RouteOutcome::Failed(RouteFailureKind::Retryable)
                | RouteOutcome::Failed(RouteFailureKind::CapabilityUnavailable)
                | RouteOutcome::Failed(RouteFailureKind::Fatal)
        ) {
            self.unavailable_accounts
                .lock()
                .map_err(|_| RouteError::StateUnavailable)?
                .insert(decision.account_id.clone());
        }
        Ok(())
    }

    pub fn reserve_usage(
        &self,
        decision: &RouteDecision,
        idempotency_key: &str,
        requirements: &[QuotaRequirement],
    ) -> Result<Vec<RouteEvent>, RouteError> {
        let mut usage_keys = self
            .usage_keys
            .lock()
            .map_err(|_| RouteError::StateUnavailable)?;
        let usage_key = (decision.account_id.clone(), idempotency_key.to_string());
        if usage_keys.contains(&usage_key) {
            return Ok(Vec::new());
        }
        let mut remaining = self
            .remaining_quota
            .lock()
            .map_err(|_| RouteError::StateUnavailable)?;
        for requirement in requirements {
            let key = (decision.account_id.clone(), requirement.metric.clone());
            if remaining
                .get(&key)
                .is_some_and(|remaining| *remaining < requirement.amount)
            {
                return Err(RouteError::QuotaExceeded {
                    account_id: decision.account_id.clone(),
                    metric: requirement.metric.clone(),
                });
            }
        }
        let events = requirements
            .iter()
            .map(|requirement| {
                let key = (decision.account_id.clone(), requirement.metric.clone());
                if let Some(remaining) = remaining.get_mut(&key) {
                    *remaining -= requirement.amount;
                }
                RouteEvent::UsageUpdated {
                    account_id: decision.account_id.clone(),
                    metric: requirement.metric.clone(),
                    amount: requirement.amount,
                }
            })
            .collect();
        usage_keys.insert(usage_key);
        Ok(events)
    }

    pub fn health(&self, capability: Capability) -> Result<RouteHealth, RouteError> {
        let unavailable = self
            .unavailable_accounts
            .lock()
            .map_err(|_| RouteError::StateUnavailable)?;
        Ok(RouteHealth {
            capability,
            available_candidates: self
                .candidates
                .iter()
                .filter(|candidate| {
                    candidate.available
                        && !unavailable.contains(&candidate.account_id)
                        && candidate.capabilities.contains(&capability)
                })
                .count(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_a_sticky_route_then_switches_accounts_before_models() {
        let selector = RouteSelector::new(vec![
            candidate("a", "model-a"),
            candidate("c", "model-b"),
            candidate("b", "model-a"),
        ]);
        let (first, _) = selector
            .select(Capability::TextReasoning, "conversation:1")
            .expect("first route");
        let (sticky, _) = selector
            .select(Capability::TextReasoning, "conversation:1")
            .expect("sticky route");
        assert_eq!(first, sticky);
        selector
            .report(&first, RouteOutcome::Failed(RouteFailureKind::Fatal))
            .expect("report");
        let (switched, event) = selector
            .select(Capability::TextReasoning, "conversation:1")
            .expect("fallback route");
        assert_eq!(switched.account_id, "b");
        assert!(matches!(event, RouteEvent::Switched { .. }));
    }

    #[test]
    fn reserves_all_quota_metrics_atomically_and_idempotently() {
        let selector = RouteSelector::new(vec![candidate("a", "model-a")]);
        selector.set_quota("a", "tokens", 100).expect("tokens");
        selector.set_quota("a", "requests", 1).expect("requests");
        let (decision, _) = selector
            .select(Capability::TextStructuredOutput, "execution:1")
            .expect("route");
        let requirements = vec![
            QuotaRequirement {
                metric: "tokens".to_string(),
                amount: 80,
            },
            QuotaRequirement {
                metric: "requests".to_string(),
                amount: 1,
            },
        ];
        assert_eq!(
            selector
                .reserve_usage(&decision, "usage-1", &requirements)
                .expect("reserve")
                .len(),
            2
        );
        assert!(
            selector
                .reserve_usage(&decision, "usage-1", &requirements)
                .expect("idempotent reserve")
                .is_empty()
        );
        assert!(matches!(
            selector.reserve_usage(
                &decision,
                "usage-2",
                &[QuotaRequirement {
                    metric: "tokens".to_string(),
                    amount: 21,
                }],
            ),
            Err(RouteError::QuotaExceeded { .. })
        ));
    }

    fn candidate(account_id: &str, model: &str) -> RouteCandidate {
        RouteCandidate {
            account_id: account_id.to_string(),
            provider: "fixed".to_string(),
            model: model.to_string(),
            capabilities: vec![Capability::TextReasoning, Capability::TextStructuredOutput],
            available: true,
        }
    }
}
