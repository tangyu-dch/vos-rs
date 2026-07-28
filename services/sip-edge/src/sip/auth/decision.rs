#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthDecision {
    Disabled,
    Authorized { username: String },
    Challenge,
    ChallengeWithFailure,
}
